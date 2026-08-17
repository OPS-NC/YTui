"""Audio-only player built on a single ffmpeg process.

That one process does three jobs at once:

  * decodes the audio-only stream (never a video stream);
  * plays it on the OS sink — PulseAudio (`-f pulse`) on Linux, AudioToolbox
    (`-f audiotoolbox`) on macOS — which also paces the whole graph;
  * emits a mono 16 kHz s16 copy on stdout, used by the analyser.

Consequences: no second ffmpeg, no PortAudio, no numpy. Pause is SIGSTOP on
the process, which is instantaneous and cannot desynchronise anything, and
seek/volume restart the process at the current position.
"""

from __future__ import annotations

import math
import shutil
import signal
import subprocess
import sys
import threading
import time
from array import array
from collections import deque
from functools import lru_cache

VIS_RATE = 16000            # analyser feed: 32 kB/s, negligible
# macOS has no PulseAudio; ffmpeg talks to CoreAudio through audiotoolbox,
# which takes no buffer option — its own latency is ~100 ms.
IS_MAC = sys.platform == "darwin"
BACKEND = "audiotoolbox" if IS_MAC else "pulse"
SINK_BUFFER_MS = 100 if IS_MAC else 200
# sources.resolve_audio now probes each URL the way ffmpeg fetches it, so an
# attempt reaching this point is expected to work; the few that still fail lost
# a race against expiry and a freshly signed URL fixes them.
ATTEMPTS = 4
OPEN_TIMEOUT = 5.0          # seconds to wait for the first samples
WINDOW = 256                # samples per analysis window (16 ms)
NBANDS = 32
FMIN, FMAX = 55.0, 7000.0


@lru_cache(maxsize=1)
def _sink_available() -> bool:
    """An ffmpeg build without the sink muxer would fail ATTEMPTS times in a
    row for a reason no retry can fix; one cached probe says it up front."""
    try:
        out = subprocess.run(
            ["ffmpeg", "-hide_banner", "-muxers"],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            text=True, timeout=10,
        ).stdout
    except (OSError, subprocess.SubprocessError):
        return True         # undecidable: let the normal error path speak
    return any(parts[1] == BACKEND
               for parts in (ln.split() for ln in out.splitlines())
               if len(parts) >= 2)


def _band_freqs(n: int) -> list[float]:
    step = (math.log(FMAX) - math.log(FMIN)) / (n - 1)
    return [math.exp(math.log(FMIN) + i * step) for i in range(n)]


class Player:
    def __init__(self, on_finished=None, on_error=None, on_attempt=None):
        self._on_finished = on_finished
        self._on_error = on_error
        self._on_attempt = on_attempt

        self._proc: subprocess.Popen | None = None
        self._lock = threading.Lock()
        self._generation = 0

        self._samples_read = 0
        self._start_offset = 0.0
        self._provider = None
        self._last_error = ""
        self._window = deque([0.0] * WINDOW, maxlen=WINDOW)

        self.paused = False
        self.volume = 1.0
        self.duration: float | None = None
        self.loaded = False
        self.backend = BACKEND

        freqs = _band_freqs(NBANDS)
        # Pre-computed Goertzel coefficients: the whole analyser cost is
        # NBANDS * WINDOW multiply-adds per frame (~8k), i.e. nothing.
        self._coeffs = [2.0 * math.cos(2.0 * math.pi * f / VIS_RATE) for f in freqs]
        self._hann = [0.5 - 0.5 * math.cos(2.0 * math.pi * i / (WINDOW - 1))
                      for i in range(WINDOW)]
        self._bands = [0.0] * NBANDS
        self._peaks = [0.0] * NBANDS
        self._tilt = [9.0 * (i / (NBANDS - 1)) ** 1.5 + 6.0 * (1 - i / (NBANDS - 1)) ** 2
                      for i in range(NBANDS)]

    # ------------------------------------------------------------------ state

    @property
    def position(self) -> float:
        played = self._samples_read / VIS_RATE
        return self._start_offset + max(0.0, played - SINK_BUFFER_MS / 1000.0)

    @property
    def playing(self) -> bool:
        return self.loaded and not self.paused

    # ------------------------------------------------------------- transport

    def play(self, provider, duration: float | None = None, start: float = 0.0) -> None:
        """`provider(refresh)` returns `(url, headers)`; called again with
        refresh=True whenever an attempt fails.

        Googlevideo hands out signed URLs that are frequently dead on arrival
        (403 on the very first request, roughly two times out of three, headers
        make no difference). So opening a stream is a retry loop, run off the
        UI thread — never a single shot.
        """
        if not shutil.which("ffmpeg"):
            raise RuntimeError("ffmpeg introuvable dans le PATH")
        if not _sink_available():
            hint = ("réinstallez ffmpeg (brew install ffmpeg)" if IS_MAC
                    else "installez un ffmpeg avec le support PulseAudio")
            raise RuntimeError(f"ffmpeg sans sortie « {BACKEND} » — {hint}")
        self.stop()
        with self._lock:
            self._generation += 1
            gen = self._generation
            self._provider = provider
            self.duration = duration
            self._start_offset = max(0.0, start)
            self._samples_read = 0
            self.paused = False
            self.loaded = True
        threading.Thread(
            target=self._launch, args=(provider, gen, start), daemon=True
        ).start()

    def _launch(self, provider, gen: int, start: float) -> None:
        last = "flux indisponible"
        for attempt in range(ATTEMPTS):
            if gen != self._generation:
                return
            if attempt and self._on_attempt:
                self._on_attempt(attempt + 1, ATTEMPTS)
            try:
                url, headers = provider(attempt > 0)
            except Exception as exc:
                last = str(exc)
                continue
            if not url:
                continue
            try:
                if self._spawn(url, headers, gen, start):
                    return
                last = self._last_error or last
            except Exception as exc:
                last = str(exc)
        if gen == self._generation:
            self.loaded = False
            if self._on_error:
                self._on_error(RuntimeError(f"lecture impossible — {last}"))

    def _spawn(self, url: str, headers: dict | None, gen: int, start: float) -> bool:
        """Start ffmpeg and return True once audio actually flows."""
        vol = f"volume={self.volume:.3f}"
        cmd = [
            "ffmpeg", "-nostdin", "-loglevel", "error",
            "-reconnect", "1", "-reconnect_streamed", "1", "-reconnect_delay_max", "5",
        ]
        extra = {k: v for k, v in (headers or {}).items() if k.lower() != "user-agent"}
        if headers and headers.get("User-Agent"):
            cmd += ["-user_agent", headers["User-Agent"]]
        if extra:
            cmd += ["-headers", "".join(f"{k}: {v}\r\n" for k, v in extra.items())]
        if start > 0:
            cmd += ["-ss", f"{start:.3f}"]
        # 1. the audible output; the OS sink paces the whole graph
        if IS_MAC:
            sink = ["-f", "audiotoolbox", "-"]
        else:
            sink = ["-f", "pulse", "-buffer_duration", str(SINK_BUFFER_MS), "ytui"]
        cmd += [
            "-i", url, "-vn", "-sn", "-dn",
            "-map", "0:a:0", "-af", vol, *sink,
            # 2. the analyser feed, same clock, 1/12th of the bandwidth
            "-map", "0:a:0", "-af", f"{vol},aresample={VIS_RATE}",
            "-ac", "1", "-f", "s16le", "pipe:1",
        ]
        proc = subprocess.Popen(
            cmd, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, bufsize=0,
        )

        if gen != self._generation:
            proc.kill()
            return False

        self._proc = proc
        self._last_error = ""
        started = threading.Event()
        threading.Thread(
            target=self._read_vis, args=(proc, gen, started), daemon=True
        ).start()

        # A dead signed URL fails on its very first request, so waiting for
        # either the first samples or ffmpeg's exit settles it in a second.
        deadline = time.monotonic() + OPEN_TIMEOUT
        while time.monotonic() < deadline and gen == self._generation:
            if started.wait(0.1):
                return True
            if proc.poll() is not None:
                break
        proc.kill()
        if not self._last_error:
            self._last_error = "aucun flux audio reçu"
        return False

    def stop(self) -> None:
        with self._lock:
            self._generation += 1
            self.loaded = False
            self.paused = False
            proc, self._proc = self._proc, None
        if proc is not None:
            try:
                if proc.poll() is None:
                    proc.send_signal(signal.SIGCONT)   # a stopped process ignores SIGTERM
                proc.kill()
            except Exception:
                pass
        self._window.extend([0.0] * WINDOW)

    def toggle_pause(self) -> bool:
        """SIGSTOP/SIGCONT: ffmpeg stops feeding PulseAudio, nothing drifts."""
        proc = self._proc
        if not self.loaded or proc is None or proc.poll() is not None:
            return self.paused
        try:
            proc.send_signal(signal.SIGCONT if self.paused else signal.SIGSTOP)
        except Exception:
            return self.paused
        self.paused = not self.paused
        return self.paused

    def seek(self, seconds: float) -> None:
        if not self.loaded or self._provider is None:
            return
        target = self.position + seconds
        if self.duration:
            target = min(target, max(0.0, self.duration - 1.0))
        self._restart(max(0.0, target))

    def set_volume(self, value: float) -> None:
        """Volume lives in ffmpeg's filter graph, so it restarts in place."""
        self.volume = min(1.5, max(0.0, value))
        if self.loaded and self._provider is not None:
            self._restart(self.position)

    def _restart(self, position: float) -> None:
        provider = self._provider
        if provider is not None:
            self.play(provider, self.duration, position)

    # ------------------------------------------------------------- internals

    def _read_vis(self, proc: subprocess.Popen, gen: int,
                  started: threading.Event) -> None:
        """Drains the analyser pipe. Must never stall: ffmpeg blocks on a full
        pipe, and that would stall playback too. So this thread only reads and
        appends — all the maths happen in spectrum(), on the UI side."""
        chunk = 1024 * 2                       # 1024 samples
        try:
            while gen == self._generation:
                raw = proc.stdout.read(chunk)
                if not raw:
                    break
                started.set()
                block = array("h")
                block.frombytes(raw[: len(raw) - len(raw) % 2])
                self._samples_read += len(block)
                # Keep only the tail: the window is all the analyser needs.
                self._window.extend(s / 32768.0 for s in block[-WINDOW:])
        except Exception as exc:
            self._last_error = str(exc)
        finally:
            err = b""
            try:
                err = proc.stderr.read() or b""
            except Exception:
                pass
            if err:
                self._last_error = err.decode(errors="replace").strip().splitlines()[-1][:160]
            if not started.is_set():
                return          # never produced audio: _spawn will retry
            if gen == self._generation:
                self.loaded = False
                if self._on_finished:
                    try:
                        self._on_finished()
                    except Exception:
                        pass

    # ------------------------------------------------------------- analysis

    def spectrum(self, nbands: int = NBANDS) -> tuple[list[float], list[float]]:
        """Goertzel filter bank: one band = one two-tap recursion. Returns
        levels and decaying peaks, both in [0, 1]."""
        if not self.playing:
            self._bands = [b * 0.80 for b in self._bands]
            self._peaks = [max(p * 0.90, b) for p, b in zip(self._peaks, self._bands)]
            return self._bands, self._peaks

        samples = list(self._window)
        windowed = [s * h for s, h in zip(samples, self._hann)]

        for i, coeff in enumerate(self._coeffs):
            s1 = s2 = 0.0
            for x in windowed:
                s0 = x + coeff * s1 - s2
                s2 = s1
                s1 = s0
            power = s1 * s1 + s2 * s2 - coeff * s1 * s2
            db = 10.0 * math.log10(power / (WINDOW * WINDOW / 16.0) + 1e-12)
            level = (db + self._tilt[i] + 56.0) / 38.0
            level = 0.0 if level < 0.0 else (1.0 if level > 1.0 else level)
            # Fast attack, slow release: the classic analyser feel.
            prev = self._bands[i]
            self._bands[i] = (prev * 0.4 + level * 0.6) if level > prev \
                else (prev * 0.78 + level * 0.22)
            self._peaks[i] = max(self._peaks[i] - 0.012, self._bands[i])

        return self._bands, self._peaks
