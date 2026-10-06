//! Audio-first player built on a single ffmpeg process.
//!
//! That one process does three jobs at once:
//!
//!   * decodes the stream (the video track only when the clip is asked for);
//!   * plays it on the OS sink — PulseAudio (`-f pulse`) on Linux,
//!     AudioToolbox (`-f audiotoolbox`) on macOS — which also paces the whole
//!     graph;
//!   * emits a mono 16 kHz s16 copy on stdout, used by the analyser.
//!
//! Consequences: no second ffmpeg, no PortAudio, no FFT library. Pause is
//! SIGSTOP on the process, which is instantaneous and cannot desynchronise
//! anything, and seek/volume restart the process at the current position.

use std::fs::File;
use std::io::Read;
use std::os::fd::FromRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use crate::exec;
use crate::sources::Headers;

const VIS_RATE: f64 = 16000.0; // analyser feed: 32 kB/s, negligible
// macOS has no PulseAudio; ffmpeg talks to CoreAudio through audiotoolbox,
// which takes no buffer option — its own latency is ~100 ms.
pub const BACKEND: &str = if cfg!(target_os = "macos") { "audiotoolbox" } else { "pulse" };
const SINK_BUFFER_MS: u32 = if cfg!(target_os = "macos") { 100 } else { 200 };
// sources::resolve probes each URL the way ffmpeg fetches it, so an attempt
// reaching this point is expected to work; the few that still fail lost a
// race against expiry and a freshly signed URL fixes them.
const ATTEMPTS: u32 = 4;
const OPEN_TIMEOUT: Duration = Duration::from_secs(5); // wait for the first samples
// A stream that ends this far before its known duration was cut, not
// finished: googlevideo drops long-lived connections, and ffmpeg's own
// -reconnect gives up on a URL that has meanwhile expired. Resume from the
// position with a freshly signed URL, a bounded number of times per track.
const CUT_MARGIN_S: f64 = 5.0;
const MAX_RESUMES: u32 = 3;
// Decode grid for the optional clip: fixed, so a terminal resize never has
// to restart ffmpeg — the widget samples this buffer down to whatever cell
// grid it has. 320x180 at 12 fps is 2 Mo/s through a pipe, and 16:9 like
// the source.
pub const VID_W: usize = 320;
pub const VID_H: usize = 180;
const VID_FPS: u32 = 12;
const VID_FRAME: usize = VID_W * VID_H * 3;
const WINDOW: usize = 256; // samples per analysis window (16 ms)
pub const NBANDS: usize = 32;
const FMIN: f64 = 55.0;
const FMAX: f64 = 7000.0;

/// `provider(refresh)` returns `(url, headers)`; called again with
/// refresh=true whenever an attempt fails, so each retry gets a freshly
/// signed URL rather than the dead one.
pub type Provider = Arc<dyn Fn(bool) -> Result<(String, Headers), String> + Send + Sync>;

pub enum PlayerEvent {
    Finished,
    Error(String),
    Attempt(u32, u32),
    /// The stream broke off at this position; playback resumes from there.
    Resumed(f64, String),
}

/// One-shot ffmpeg decode of a video's default thumbnail, straight to the
/// `w` x `h` rgb24 grid it will be drawn at. `mqdefault.jpg` is the one
/// thumbnail size YouTube guarantees for every public video (320x180), so
/// ffmpeg fetches it directly as input; no HTTP client, no image crate. And
/// since ffmpeg does the scaling (area filter: better looking than nearest
/// neighbour), a thumbnail costs ~1 kB resident instead of a 170 kB frame.
pub fn decode_thumbnail(video_id: &str, w: usize, h: usize) -> Option<Vec<u8>> {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-nostdin", "-loglevel", "error", "-i"])
        .arg(format!("https://i.ytimg.com/vi/{video_id}/mqdefault.jpg"))
        .args(["-frames:v", "1", "-vf"])
        .arg(format!("scale={w}:{h}:flags=area"))
        .args(["-pix_fmt", "rgb24", "-f", "rawvideo", "pipe:1"]);
    let out = exec::run(cmd, Duration::from_secs(10)).ok()?;
    (out.stdout.len() == w * h * 3).then_some(out.stdout)
}

/// An ffmpeg build without the sink muxer would fail ATTEMPTS times in a row
/// for a reason no retry can fix; one cached probe says it up front.
fn sink_available() -> bool {
    static OK: OnceLock<bool> = OnceLock::new();
    *OK.get_or_init(|| {
        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-hide_banner", "-muxers"]);
        let Ok(out) = exec::run(cmd, Duration::from_secs(10)) else {
            return true; // undecidable: let the normal error path speak
        };
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .any(|l| l.split_whitespace().nth(1) == Some(BACKEND))
    })
}

/// Latest decoded picture. The buffer is swapped, never reallocated: one
/// frame here plus one being filled by the reader is all the clip costs.
pub struct Frame {
    pub data: Vec<u8>,
    pub no: u64,
    pub valid: bool,
}

struct Ring {
    buf: [f32; WINDOW],
    pos: usize,
}

struct Ctl {
    child: Option<Child>,
    provider: Option<Provider>,
    duration: Option<f64>,
    volume: f32,
    // Asking ffmpeg to map a video stream that the input does not have is
    // fatal, so the caller decides before play().
    video: bool,
    start_offset: f64,
    resumes: u32,
}

struct Shared {
    notify: Box<dyn Fn(PlayerEvent) + Send + Sync>,
    generation: AtomicU64,
    ctl: Mutex<Ctl>,
    samples_read: AtomicU64,
    loaded: AtomicBool,
    paused: AtomicBool,
    window: Mutex<Ring>,
    frame: Mutex<Frame>,
}

#[derive(Clone)]
pub struct Player {
    s: Arc<Shared>,
}

impl Player {
    pub fn new(notify: impl Fn(PlayerEvent) + Send + Sync + 'static) -> Self {
        Self {
            s: Arc::new(Shared {
                notify: Box::new(notify),
                generation: AtomicU64::new(0),
                ctl: Mutex::new(Ctl {
                    child: None,
                    provider: None,
                    duration: None,
                    volume: 1.0,
                    video: false,
                    start_offset: 0.0,
                    resumes: 0,
                }),
                samples_read: AtomicU64::new(0),
                loaded: AtomicBool::new(false),
                paused: AtomicBool::new(false),
                window: Mutex::new(Ring { buf: [0.0; WINDOW], pos: 0 }),
                frame: Mutex::new(Frame { data: Vec::new(), no: 0, valid: false }),
            }),
        }
    }

    fn ctl(&self) -> MutexGuard<'_, Ctl> {
        self.s.ctl.lock().unwrap_or_else(|e| e.into_inner())
    }

    // -------------------------------------------------------------- state

    pub fn position(&self) -> f64 {
        let played = self.s.samples_read.load(Ordering::Relaxed) as f64 / VIS_RATE;
        self.ctl().start_offset + (played - SINK_BUFFER_MS as f64 / 1000.0).max(0.0)
    }

    pub fn loaded(&self) -> bool {
        self.s.loaded.load(Ordering::Relaxed)
    }

    pub fn paused(&self) -> bool {
        self.s.paused.load(Ordering::Relaxed)
    }

    pub fn playing(&self) -> bool {
        self.loaded() && !self.paused()
    }

    pub fn duration(&self) -> Option<f64> {
        self.ctl().duration
    }

    pub fn volume(&self) -> f32 {
        self.ctl().volume
    }

    pub fn video(&self) -> bool {
        self.ctl().video
    }

    pub fn set_video(&self, on: bool) {
        self.ctl().video = on;
    }

    fn current(&self, epoch: u64) -> bool {
        self.s.generation.load(Ordering::SeqCst) == epoch
    }

    // ---------------------------------------------------------- transport

    /// Googlevideo hands out signed URLs that are frequently dead on arrival
    /// (403 on the very first request, roughly two times out of three,
    /// headers make no difference). So opening a stream is a retry loop, run
    /// off the UI thread — never a single shot.
    pub fn play(&self, provider: Provider, duration: Option<f64>, start: f64) -> Result<(), String> {
        self.ctl().resumes = 0;
        self.start(provider, duration, start, false)
    }

    /// `play` for the same track: seek, volume, resume after a cut.
    fn start(
        &self,
        provider: Provider,
        duration: Option<f64>,
        start: f64,
        refresh_first: bool,
    ) -> Result<(), String> {
        if exec::which("ffmpeg").is_none() {
            return Err("ffmpeg introuvable dans le PATH".into());
        }
        if !sink_available() {
            let hint = if cfg!(target_os = "macos") {
                "réinstallez ffmpeg (brew install ffmpeg)"
            } else {
                "installez un ffmpeg avec le support PulseAudio"
            };
            return Err(format!("ffmpeg sans sortie « {BACKEND} » — {hint}"));
        }
        self.stop();
        let epoch = {
            let mut ctl = self.ctl();
            let epoch = self.s.generation.fetch_add(1, Ordering::SeqCst) + 1;
            ctl.provider = Some(provider.clone());
            ctl.duration = duration;
            ctl.start_offset = start.max(0.0);
            self.s.samples_read.store(0, Ordering::Relaxed);
            self.s.paused.store(false, Ordering::Relaxed);
            self.s.loaded.store(true, Ordering::Relaxed);
            epoch
        };
        let me = self.clone();
        exec::spawn("launch", move || me.launch(provider, epoch, start.max(0.0), refresh_first));
        Ok(())
    }

    fn launch(&self, provider: Provider, epoch: u64, start: f64, refresh_first: bool) {
        let mut last = String::from("flux indisponible");
        for attempt in 0..ATTEMPTS {
            if !self.current(epoch) {
                return;
            }
            if attempt > 0 {
                (self.s.notify)(PlayerEvent::Attempt(attempt + 1, ATTEMPTS));
            }
            let (url, headers) = match provider(refresh_first || attempt > 0) {
                Ok(found) => found,
                Err(e) => {
                    last = e;
                    continue;
                }
            };
            if url.is_empty() {
                continue;
            }
            match self.spawn(&url, &headers, epoch, start) {
                Ok(()) => return,
                Err(e) => last = e,
            }
        }
        if self.current(epoch) {
            self.s.loaded.store(false, Ordering::Relaxed);
            (self.s.notify)(PlayerEvent::Error(format!("lecture impossible — {last}")));
        }
    }

    /// Start ffmpeg and return once audio actually flows; Err carries the
    /// reason ffmpeg gave otherwise.
    fn spawn(&self, url: &str, headers: &Headers, epoch: u64, start: f64) -> Result<(), String> {
        let (volume, video) = {
            let ctl = self.ctl();
            (ctl.volume, ctl.video)
        };
        let vol = format!("volume={volume:.3}");
        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-nostdin", "-loglevel", "error"]);
        cmd.args(["-reconnect", "1", "-reconnect_streamed", "1", "-reconnect_delay_max", "5"]);
        if let Some((_, ua)) = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("user-agent")) {
            cmd.arg("-user_agent").arg(ua);
        }
        let extra: String = headers
            .iter()
            .filter(|(k, _)| !k.eq_ignore_ascii_case("user-agent"))
            .map(|(k, v)| format!("{k}: {v}\r\n"))
            .collect();
        if !extra.is_empty() {
            cmd.arg("-headers").arg(extra);
        }
        if start > 0.0 {
            cmd.arg("-ss").arg(format!("{start:.3}"));
        }
        if !video {
            cmd.arg("-vn");
        }
        cmd.args(["-i", url, "-sn", "-dn"]);
        // 1. the audible output; the OS sink paces the whole graph
        cmd.args(["-map", "0:a:0", "-af", &vol]);
        if cfg!(target_os = "macos") {
            cmd.args(["-f", "audiotoolbox", "-"]);
        } else {
            cmd.args(["-f", "pulse", "-buffer_duration", &SINK_BUFFER_MS.to_string(), "ytui"]);
        }
        // 2. the analyser feed, same clock, 1/12th of the bandwidth
        cmd.args(["-map", "0:a:0", "-af", &format!("{vol},aresample={}", VIS_RATE as u32)]);
        cmd.args(["-ac", "1", "-f", "s16le", "pipe:1"]);
        // 3. the clip, when asked for: raw frames on fd 3. Same process, so
        // the audio sink keeps pacing the picture and nothing can drift.
        let mut video_fds: Option<(i32, i32)> = None;
        if video {
            let mut fds = [0i32; 2];
            if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
                return Err("pipe vidéo impossible".into());
            }
            for fd in fds {
                unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
            }
            let write_fd = fds[1];
            video_fds = Some((fds[0], write_fd));
            cmd.args(["-map", "0:v:0", "-vf"])
                .arg(format!("fps={VID_FPS},scale={VID_W}:{VID_H}:flags=bilinear"))
                .args(["-pix_fmt", "rgb24", "-f", "rawvideo", "pipe:3"]);
            unsafe {
                cmd.pre_exec(move || {
                    // dup2 onto itself would keep CLOEXEC set: clear it instead.
                    if write_fd == 3 {
                        libc::fcntl(3, libc::F_SETFD, 0);
                    } else if libc::dup2(write_fd, 3) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let spawned = cmd.spawn();
        if let Some((_, w)) = video_fds {
            unsafe { libc::close(w) };
        }
        let mut child = match spawned {
            Ok(c) => c,
            Err(e) => {
                if let Some((r, _)) = video_fds {
                    unsafe { libc::close(r) };
                }
                return Err(e.to_string());
            }
        };
        let video_pipe = video_fds.map(|(r, _)| unsafe { File::from_raw_fd(r) });

        if !self.current(epoch) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("annulé".into());
        }

        let started = Arc::new(AtomicBool::new(false));
        let stderr_last = Arc::new(Mutex::new(String::new()));
        let stderr_done = {
            let pipe = child.stderr.take();
            let last = stderr_last.clone();
            let (tx, rx) = std::sync::mpsc::channel::<()>();
            exec::spawn("ffmpeg-err", move || {
                let mut text = Vec::new();
                if let Some(mut p) = pipe {
                    let _ = p.read_to_end(&mut text);
                }
                let text = String::from_utf8_lossy(&text);
                if let Some(line) = text.trim().lines().last() {
                    *last.lock().unwrap() = line.chars().take(160).collect();
                }
                let _ = tx.send(());
            });
            rx
        };
        {
            let me = self.clone();
            let pipe = child.stdout.take();
            let started = started.clone();
            let stderr_last = stderr_last.clone();
            exec::spawn("ffmpeg-pcm", move || me.read_vis(pipe, epoch, &started, &stderr_last));
        }
        if let Some(pipe) = video_pipe {
            let me = self.clone();
            exec::spawn("ffmpeg-rgb", move || me.read_video(pipe, epoch));
        }

        // A dead signed URL fails on its very first request, so waiting for
        // either the first samples or ffmpeg's exit settles it in a second.
        let deadline = Instant::now() + OPEN_TIMEOUT;
        while Instant::now() < deadline && self.current(epoch) {
            std::thread::sleep(Duration::from_millis(50));
            if started.load(Ordering::SeqCst) {
                // Same lock as stop(): either stop() already moved the
                // generation on and this process dies here, or it lands where
                // stop() will find it.
                let mut ctl = self.ctl();
                if self.current(epoch) {
                    ctl.child = Some(child);
                    return Ok(());
                }
                break;
            }
            if matches!(child.try_wait(), Ok(Some(_))) {
                break;
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        let _ = stderr_done.recv_timeout(Duration::from_secs(1));
        let err = std::mem::take(&mut *stderr_last.lock().unwrap());
        Err(if err.is_empty() { "aucun flux audio reçu".into() } else { err })
    }

    pub fn stop(&self) {
        let child = {
            let mut ctl = self.ctl();
            self.s.generation.fetch_add(1, Ordering::SeqCst);
            self.s.loaded.store(false, Ordering::Relaxed);
            self.s.paused.store(false, Ordering::Relaxed);
            ctl.child.take()
        };
        if let Some(mut child) = child {
            // A stopped process ignores SIGTERM; SIGKILL works, but a
            // SIGCONT first keeps the sink from glitching on some setups.
            unsafe { libc::kill(child.id() as i32, libc::SIGCONT) };
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Ok(mut ring) = self.s.window.lock() {
            ring.buf = [0.0; WINDOW];
        }
        // The reader checks the epoch under this same lock before swapping,
        // so nothing refills the slot after this: the 170 kB go back now.
        if let Ok(mut frame) = self.s.frame.lock() {
            frame.valid = false;
            frame.data = Vec::new();
        }
    }

    /// SIGSTOP/SIGCONT: ffmpeg stops feeding the sink, nothing drifts.
    pub fn toggle_pause(&self) -> bool {
        let mut ctl = self.ctl();
        let paused = self.paused();
        if !self.loaded() {
            return paused;
        }
        let Some(child) = ctl.child.as_mut() else { return paused };
        if !matches!(child.try_wait(), Ok(None)) {
            return paused;
        }
        let sig = if paused { libc::SIGCONT } else { libc::SIGSTOP };
        if unsafe { libc::kill(child.id() as i32, sig) } != 0 {
            return paused;
        }
        self.s.paused.store(!paused, Ordering::Relaxed);
        !paused
    }

    pub fn seek(&self, seconds: f64) -> Result<(), String> {
        let (provider, duration) = {
            let ctl = self.ctl();
            (ctl.provider.clone(), ctl.duration)
        };
        let Some(provider) = provider.filter(|_| self.loaded()) else { return Ok(()) };
        let mut target = self.position() + seconds;
        if let Some(d) = duration.filter(|d| *d > 0.0) {
            target = target.min((d - 1.0).max(0.0));
        }
        self.start(provider, duration, target.max(0.0), false)
    }

    /// Volume lives in ffmpeg's filter graph, so it restarts in place.
    pub fn set_volume(&self, value: f32) -> Result<(), String> {
        let (provider, duration) = {
            let mut ctl = self.ctl();
            ctl.volume = value.clamp(0.0, 1.5);
            (ctl.provider.clone(), ctl.duration)
        };
        match provider.filter(|_| self.loaded()) {
            Some(p) => self.start(p, duration, self.position(), false),
            None => Ok(()),
        }
    }

    // --------------------------------------------------------------- clip

    /// Locked access to the latest frame, for the renderer.
    pub fn frame(&self) -> MutexGuard<'_, Frame> {
        self.s.frame.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn frame_no(&self) -> Option<u64> {
        let f = self.frame();
        f.valid.then_some(f.no)
    }

    /// Drains the picture pipe, keeping only the newest frame. Like the
    /// analyser pipe this must never stall: a full pipe blocks ffmpeg, which
    /// would block the sound as well. So it reads flat out and swaps.
    fn read_video(&self, mut pipe: File, epoch: u64) {
        let mut buf = vec![0u8; VID_FRAME];
        while self.current(epoch) {
            if pipe.read_exact(&mut buf).is_err() {
                break;
            }
            let mut frame = self.frame();
            if !self.current(epoch) {
                break;
            }
            std::mem::swap(&mut frame.data, &mut buf);
            if buf.len() != VID_FRAME {
                buf = vec![0u8; VID_FRAME]; // first frame: the shared slot was empty
            }
            frame.no += 1;
            frame.valid = true;
        }
    }

    // ---------------------------------------------------------- internals

    /// Drains the analyser pipe. Must never stall: ffmpeg blocks on a full
    /// pipe, and that would stall playback too. So this thread only reads
    /// and keeps the tail — the maths happen in `Analyser`, on the UI side.
    fn read_vis(
        &self,
        pipe: Option<std::process::ChildStdout>,
        epoch: u64,
        started: &AtomicBool,
        stderr_last: &Mutex<String>,
    ) {
        let mut raw = [0u8; 2048]; // 1024 samples
        if let Some(mut pipe) = pipe {
            while self.current(epoch) {
                let n = match pipe.read(&mut raw) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n & !1,
                };
                started.store(true, Ordering::SeqCst);
                let count = n / 2;
                self.s.samples_read.fetch_add(count as u64, Ordering::Relaxed);
                // Keep only the tail: the window is all the analyser needs.
                let tail = count.saturating_sub(WINDOW);
                let mut ring = self.s.window.lock().unwrap();
                for i in tail..count {
                    let s = i16::from_le_bytes([raw[2 * i], raw[2 * i + 1]]);
                    let pos = ring.pos;
                    ring.buf[pos] = s as f32 / 32768.0;
                    ring.pos = (pos + 1) % WINDOW;
                }
            }
        }
        if !started.load(Ordering::SeqCst) {
            return; // never produced audio: spawn() will retry
        }
        let position = self.position();
        let (child, resume) = {
            let mut ctl = self.ctl();
            if !self.current(epoch) {
                return;
            }
            let cut = ctl.duration.is_some_and(|d| position < d - CUT_MARGIN_S);
            let resume = match ctl.provider.clone() {
                Some(p) if cut && ctl.resumes < MAX_RESUMES => {
                    ctl.resumes += 1;
                    Some((p, ctl.duration))
                }
                _ => None,
            };
            self.s.loaded.store(false, Ordering::Relaxed);
            (ctl.child.take(), resume)
        };
        if let Some(mut child) = child {
            let _ = child.wait();
        }
        let Some((provider, duration)) = resume else {
            return (self.s.notify)(PlayerEvent::Finished);
        };
        // ffmpeg's last words, if its stderr has been drained by now.
        std::thread::sleep(Duration::from_millis(100));
        let reason = stderr_last.lock().map(|s| s.clone()).unwrap_or_default();
        (self.s.notify)(PlayerEvent::Resumed(position, reason));
        if let Err(e) = self.start(provider, duration, position, true) {
            (self.s.notify)(PlayerEvent::Error(e));
        }
    }

    fn window(&self, out: &mut [f32; WINDOW]) {
        let ring = self.s.window.lock().unwrap();
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = ring.buf[(ring.pos + i) % WINDOW];
        }
    }
}

/// Goertzel filter bank: one band = one two-tap recursion. Lives on the UI
/// side and only runs while the analyser is on screen.
pub struct Analyser {
    coeffs: [f32; NBANDS],
    hann: [f32; WINDOW],
    tilt: [f32; NBANDS],
    samples: [f32; WINDOW],
    pub bands: [f32; NBANDS],
    pub peaks: [f32; NBANDS],
}

impl Analyser {
    pub fn new() -> Self {
        let step = (FMAX.ln() - FMIN.ln()) / (NBANDS - 1) as f64;
        // Pre-computed coefficients: the whole analyser cost is
        // NBANDS * WINDOW multiply-adds per frame (~8k), i.e. nothing.
        let coeffs = std::array::from_fn(|i| {
            let f = (FMIN.ln() + i as f64 * step).exp();
            (2.0 * (2.0 * std::f64::consts::PI * f / VIS_RATE).cos()) as f32
        });
        let hann = std::array::from_fn(|i| {
            (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (WINDOW - 1) as f64).cos()) as f32
        });
        let tilt = std::array::from_fn(|i| {
            let t = i as f32 / (NBANDS - 1) as f32;
            9.0 * t.powf(1.5) + 6.0 * (1.0 - t).powi(2)
        });
        Self {
            coeffs,
            hann,
            tilt,
            samples: [0.0; WINDOW],
            bands: [0.0; NBANDS],
            peaks: [0.0; NBANDS],
        }
    }

    pub fn active(&self) -> bool {
        self.bands.iter().chain(&self.peaks).any(|v| *v > 0.002)
    }

    /// Levels and decaying peaks, both in [0, 1].
    pub fn update(&mut self, player: &Player) {
        if !player.playing() {
            for (b, p) in self.bands.iter_mut().zip(self.peaks.iter_mut()) {
                *b *= 0.80;
                *p = (*p * 0.90).max(*b);
                if *p < 0.002 {
                    *b = 0.0;
                    *p = 0.0;
                }
            }
            return;
        }
        player.window(&mut self.samples);
        for (s, h) in self.samples.iter_mut().zip(&self.hann) {
            *s *= h;
        }
        let norm = (WINDOW * WINDOW) as f32 / 16.0;
        for i in 0..NBANDS {
            let coeff = self.coeffs[i];
            let (mut s1, mut s2) = (0.0f32, 0.0f32);
            for &x in &self.samples {
                let s0 = x + coeff * s1 - s2;
                s2 = s1;
                s1 = s0;
            }
            let power = s1 * s1 + s2 * s2 - coeff * s1 * s2;
            let db = 10.0 * (power / norm + 1e-12).log10();
            let level = ((db + self.tilt[i] + 56.0) / 38.0).clamp(0.0, 1.0);
            // Fast attack, slow release: the classic analyser feel.
            let prev = self.bands[i];
            self.bands[i] = if level > prev {
                prev * 0.4 + level * 0.6
            } else {
                prev * 0.78 + level * 0.22
            };
            self.peaks[i] = (self.peaks[i] - 0.012).max(self.bands[i]);
        }
    }
}
