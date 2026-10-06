//! YouTube metadata via the yt-dlp *binary*.
//!
//! yt-dlp is deliberately never linked or embedded: every call runs in a
//! throw-away process, so the ~60 MB it allocates while parsing a page is
//! returned to the OS instead of staying resident in the TUI for the whole
//! session. Listings are asked for as one tab-separated line per entry
//! (`--print`) rather than a JSON dump, so this side never has to hold — or
//! parse — a multi-megabyte document either.

use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::{exec, tools};

const TIMEOUT: Duration = Duration::from_secs(90);
const PROBE_BYTES: usize = 8192;
const PROBE_TIMEOUT_S: &str = "8";

#[derive(Clone, Debug)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub uploader: String,
    pub duration: Option<u32>,
}

impl Video {
    pub fn new(id: &str, title: String) -> Self {
        Self { id: id.into(), title, uploader: String::new(), duration: None }
    }

    pub fn url(&self) -> String {
        format!("https://www.youtube.com/watch?v={}", self.id)
    }

    pub fn duration_str(&self) -> String {
        match self.duration {
            Some(d) if d > 0 => fmt_time(d as f64),
            _ => "--:--".into(),
        }
    }
}

pub type Headers = Vec<(String, String)>;

/// A signed stream URL that has been checked against the real fetch.
#[derive(Clone, Debug)]
pub struct Stream {
    pub url: String,
    pub headers: Headers,
    pub source: String,
    pub has_video: bool,
}

pub fn fmt_time(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    let (h, m, s) = (s / 3600, s % 3600 / 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

// ---------------------------------------------------------------- id parsing

fn is_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

fn is_video_id(s: &str) -> bool {
    s.len() == 11 && s.chars().all(is_id_char)
}

const YT_PREFIXES: [&str; 5] =
    ["youtube.com", "www.youtube.com", "youtu.be", "m.youtube.com", "music.youtube.com"];

struct Url<'a> {
    host: String,
    path: &'a str,
    query: &'a str,
}

/// Just enough of `urlparse` for YouTube links: host (lowercased, `www.`
/// dropped), path and query string.
fn split_url(text: &str) -> Option<Url<'_>> {
    let rest = text.split_once("://")?.1;
    let rest = rest.split('#').next().unwrap_or("");
    let (authority, tail) = match rest.find(['/', '?']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let host = authority.rsplit('@').next().unwrap_or("");
    let host = host.split(':').next().unwrap_or("").to_ascii_lowercase();
    let host = host.strip_prefix("www.").map(String::from).unwrap_or(host);
    let (path, query) = tail.split_once('?').unwrap_or((tail, ""));
    Some(Url { host, path, query })
}

fn query_param<'a>(query: &'a str, key: &str) -> &'a str {
    query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == key)
        .map_or("", |(_, v)| v)
}

fn with_scheme(text: &str) -> Option<String> {
    if text.contains("://") {
        return Some(text.to_string());
    }
    if !YT_PREFIXES.iter().any(|p| text.starts_with(p)) {
        return None;
    }
    Some(format!("https://{text}"))
}

/// Accept a bare id, a watch/shorts/embed URL or a youtu.be link.
pub fn parse_video_id(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || text.contains(' ') {
        return None;
    }
    if is_video_id(text) {
        return Some(text.to_string());
    }
    let full = with_scheme(text)?;
    let url = split_url(&full)?;
    let candidate = match url.host.as_str() {
        "youtu.be" => url.path.trim_start_matches('/').split('/').next().unwrap_or(""),
        "youtube.com" | "m.youtube.com" | "music.youtube.com" | "youtube-nocookie.com" => {
            if url.path == "/watch" {
                query_param(url.query, "v")
            } else {
                let mut parts = url.path.split('/').filter(|p| !p.is_empty());
                match parts.next() {
                    Some("shorts" | "embed" | "live" | "v") => parts.next().unwrap_or(""),
                    _ => "",
                }
            }
        }
        _ => "",
    };
    is_video_id(candidate).then(|| candidate.to_string())
}

const PLAYLIST_PREFIXES: [&str; 8] = ["PL", "OL", "UU", "LL", "FL", "RD", "UL", "TL"];

/// Accept a bare playlist id or any URL carrying a `list=` parameter.
pub fn parse_playlist_id(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || text.contains(' ') {
        return None;
    }
    if PLAYLIST_PREFIXES.iter().any(|p| text.starts_with(p))
        && text.len() >= 12
        && text.chars().all(is_id_char)
    {
        return Some(text.to_string());
    }
    let full = with_scheme(text)?;
    let url = split_url(&full)?;
    if !matches!(
        url.host.as_str(),
        "youtube.com" | "m.youtube.com" | "music.youtube.com" | "youtube-nocookie.com" | "youtu.be"
    ) {
        return None;
    }
    let candidate = query_param(url.query, "list");
    (!candidate.is_empty() && candidate.chars().all(is_id_char)).then(|| candidate.to_string())
}

// ------------------------------------------------------------------ yt-dlp

// Opt-in only: reading a browser's cookie jar on every search would be
// surprising (a keychain prompt, a locked-database error while the browser is
// open, extra latency) for the common case of public videos. Starts from
// YTUI_COOKIES_FROM_BROWSER (same syntax as yt-dlp's --cookies-from-browser,
// e.g. "firefox" or "chrome:Profile 1") so a profile/keyring suffix set before
// launch survives; "L" in the app then cycles through the plain browser names
// below without needing a restart.
pub const COOKIE_BROWSERS: [Option<&str>; 10] = [
    None,
    Some("firefox"),
    Some("chrome"),
    Some("chromium"),
    Some("edge"),
    Some("brave"),
    Some("opera"),
    Some("vivaldi"),
    Some("safari"),
    Some("whale"),
];

static COOKIE_BROWSER: Mutex<Option<String>> = Mutex::new(None);

pub fn init_cookie_browser_from_env() {
    let value = std::env::var("YTUI_COOKIES_FROM_BROWSER").unwrap_or_default();
    let value = value.trim();
    if !value.is_empty() {
        set_cookie_browser(Some(value.to_string()));
    }
}

/// Currently active --cookies-from-browser value, if any.
pub fn cookie_browser() -> Option<String> {
    COOKIE_BROWSER.lock().unwrap().clone()
}

/// Set at startup from a --<browser> CLI flag — equivalent to
/// YTUI_COOKIES_FROM_BROWSER but shorter to type.
pub fn set_cookie_browser(value: Option<String>) {
    *COOKIE_BROWSER.lock().unwrap() = value;
}

/// Advances to the next browser in COOKIE_BROWSERS (wrapping to "no
/// cookies"). Called from the app's "L" binding — a deliberate, visible way
/// to turn authentication on for age-gated or members-only videos, since
/// silently always sending cookies would be the surprising default.
pub fn cycle_cookie_browser() -> Option<String> {
    let mut current = COOKIE_BROWSER.lock().unwrap();
    let index = COOKIE_BROWSERS
        .iter()
        .position(|b| b.map(String::from) == *current)
        .map_or(0, |i| i + 1);
    *current = COOKIE_BROWSERS[index % COOKIE_BROWSERS.len()].map(String::from);
    current.clone()
}

/// Turn yt-dlp's stderr into something actionable. "Requested format is not
/// available" is yt-dlp's generic message for an empty format list — the
/// actual reason (age gate, members-only, a signature/PO-token failure) is
/// usually a WARNING line printed just above it, so that's surfaced too
/// instead of only the last line, which alone is close to meaningless.
fn explain(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
    let Some(last) = lines.last() else {
        return "yt-dlp a échoué".into();
    };
    if stderr.contains("CERTIFICATE_VERIFY_FAILED") {
        return "aucun certificat CA disponible pour yt-dlp — réinstallez-le \
                (brew install yt-dlp, pipx install yt-dlp)"
            .into();
    }
    if last.starts_with("ERROR:")
        && let Some(w) = lines[..lines.len() - 1].iter().rev().find(|l| l.contains("WARNING:")) {
            return format!("{last} — {w}");
        }
    last.to_string()
}

/// Returns (stdout, stderr). yt-dlp often exits non-zero while still printing
/// usable output, so a failed return code alone is not an error.
///
/// use_cookies=false is for the anonymous-friendly stream-resolution
/// strategies below: sending cookies on every request (not just the ones that
/// actually need them) made yt-dlp drop the Android client entirely — it
/// doesn't support cookie auth and gets skipped whenever cookies are present
/// — which broke playback for every video, not just gated ones. Metadata
/// listing (search/playlist/related/home feed) is unaffected by that and
/// still benefits from cookies unconditionally.
fn run(args: &[&str], use_cookies: bool) -> Result<(String, String), String> {
    let mut cmd = tools::ytdlp_cmd();
    cmd.args(["--no-warnings", "--ignore-config"]);
    if let Some(rt) = tools::js_runtime() {
        cmd.args(["--js-runtimes", rt]);
    }
    if use_cookies
        && let Some(browser) = cookie_browser() {
            cmd.args(["--cookies-from-browser", &browser]);
        }
    cmd.args(args);
    let out = exec::run(cmd, TIMEOUT).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "yt-dlp introuvable — son installation se fait au premier lancement, \
             vérifiez la connexion (ou installez-le : pipx install yt-dlp)"
                .into()
        } else {
            e.to_string()
        }
    })?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.success && stdout.trim().is_empty() {
        return Err(explain(&stderr));
    }
    Ok((stdout, stderr))
}

// Title last: it is the only field that could ever carry a tab, and splitn
// keeps it whole.
const LIST_PRINT: &str = "%(id)s\t%(duration|)s\t%(uploader,channel|)s\t%(title|)s";

fn parse_entry(line: &str) -> Option<Video> {
    // A partially failed extraction yields junk or empty lines — YouTube
    // errors, a blocked page — so nothing here is assumed well-formed.
    let mut f = line.splitn(4, '\t');
    let id = f.next()?;
    if !is_video_id(id) {
        return None;
    }
    let duration = f.next().and_then(|d| d.parse::<f64>().ok()).map(|d| d as u32);
    let uploader = f.next().filter(|u| *u != "NA").unwrap_or("").to_string();
    let title = f.next().filter(|t| !t.is_empty() && *t != "NA").unwrap_or("(sans titre)");
    Some(Video { id: id.into(), title: title.into(), uploader, duration })
}

fn flat(url: &str, extra: &[&str]) -> Result<Vec<Video>, String> {
    let mut args = vec!["--flat-playlist", "--print", LIST_PRINT];
    args.extend_from_slice(extra);
    args.push(url);
    let (out, err) = run(&args, true)?;
    let videos: Vec<Video> = out.lines().filter_map(parse_entry).collect();
    // Every entry unusable while stderr complained: report the real cause
    // instead of an empty, silent result list.
    if videos.is_empty() && !err.trim().is_empty() {
        return Err(explain(&err));
    }
    Ok(videos)
}

pub fn search(query: &str) -> Result<Vec<Video>, String> {
    flat(&format!("ytsearch60:{query}"), &[])
}

/// The logged-in home page ("Recommandé pour vous"). Only meaningful with a
/// session — needs cookie_browser() to be set, same as any other
/// account-gated request in this module.
pub fn home_feed() -> Result<Vec<Video>, String> {
    flat("https://www.youtube.com/feed/recommended", &["--playlist-end", "60"])
}

/// Suggestions = the YouTube auto-mix (radio) built around the video.
pub fn related(video_id: &str) -> Result<Vec<Video>, String> {
    let mix = format!("https://www.youtube.com/watch?v={video_id}&list=RD{video_id}");
    let mut videos = flat(&mix, &["--playlist-end", "41"])?;
    videos.retain(|v| v.id != video_id);
    videos.truncate(40);
    Ok(videos)
}

/// Flat listing of a playlist, in playlist order.
pub fn playlist(playlist_id: &str) -> Result<Vec<Video>, String> {
    // Auto-mix ids (RD…) are only served from a watch page, not from /playlist.
    let url = match playlist_id.strip_prefix("RD") {
        Some(seed) => format!("https://www.youtube.com/watch?v={seed}&list={playlist_id}"),
        None => format!("https://www.youtube.com/playlist?list={playlist_id}"),
    };
    flat(&url, &["--playlist-end", "500"])
}

// ------------------------------------------------------- stream resolution

/// One way of asking YouTube for something playable.
struct Strategy {
    label: &'static str,
    extractor_args: &'static [&'static str],
    fmt: &'static str,
    auth: bool, // only true for the strategy that needs cookies
}

// YouTube only hands a plain https URL to the clients that need no PO token,
// and it now refuses long-range requests on their audio-only formats: ffmpeg
// opens a stream with `Range: bytes=0-` and gets 403 every single time, which
// is what turned playback into a losing retry loop. The progressive stream of
// the android client still answers, at the price of downloading a 360p video
// track that ffmpeg throws away. So: audio-only first, muxed as a safety net.
const STRATEGIES: [Strategy; 2] = [
    Strategy {
        label: "audio seul",
        extractor_args: &[],
        fmt: "bestaudio[abr<=160]/bestaudio",
        auth: false,
    },
    Strategy {
        label: "flux muxé",
        extractor_args: &["--extractor-args", "youtube:player_client=android"],
        fmt: "bestaudio/18/best[acodec!=none]",
        auth: false,
    },
];

// Only a progressive stream carries picture and sound in one URL, which is
// what the single-ffmpeg design needs to show the clip without a second
// download.
const VIDEO_STRATEGY: Strategy = Strategy {
    label: "clip 360p",
    extractor_args: &["--extractor-args", "youtube:player_client=android"],
    fmt: "18/best[acodec!=none][vcodec!=none]",
    auth: false,
};

// Browser cookies authenticate the *web* client only: yt-dlp's mobile/embedded
// client spoofs (android above, used specifically to dodge the PO-token
// requirement for anonymous access) sign in through their own app-level flow
// and simply ignore cookies — worse, yt-dlp skips them outright once cookies
// are attached to the request at all, since they can't use them. So this is
// a last-resort fallback, tried after the anonymous strategies above (which
// stay cookie-free and keep working for public videos exactly as before),
// only for whatever they couldn't resolve — i.e. actually gated content.
const AUTH_STRATEGY: Strategy = Strategy {
    label: "connecté",
    extractor_args: &["--extractor-args", "youtube:player_client=web"],
    fmt: "bestaudio[abr<=160]/bestaudio/best",
    auth: true,
};

// Title last for the same reason as LIST_PRINT. The signed URL is only valid
// for the exact headers yt-dlp negotiated; without them googlevideo answers
// 403, hence http_headers.
const STREAM_PRINT: &str =
    "%(url)s\t%(duration)s\t%(vcodec)s\t%(http_headers)j\t%(uploader)s\t%(title)s";

// What worked for the previous track, tried first for the next one: whichever
// way YouTube is treating this session tends to hold for the whole session,
// and a doomed first strategy costs a yt-dlp round trip per track.
static PREFERRED: AtomicUsize = AtomicUsize::new(0);

/// Metadata learnt while resolving — a pasted link starts with a placeholder
/// title, and flat listings often lack the duration.
#[derive(Clone, Debug, Default)]
pub struct Meta {
    pub title: Option<String>,
    pub uploader: Option<String>,
    pub duration: Option<u32>,
}

/// Probe the URL exactly the way ffmpeg will fetch it — one open-ended range
/// request — because that is the request YouTube rejects. A URL that fails
/// here would fail ten times in a row in the player.
///
/// The probe is a throw-away curl, like yt-dlp: no TLS stack is linked into
/// the TUI just to read 8 kB once per track.
pub fn stream_playable(url: &str, headers: &Headers) -> bool {
    use std::io::Read;
    use std::process::Stdio;
    if url.contains(".m3u8") || url.contains("/manifest/") {
        return true; // HLS: fetched segment by segment
    }
    let mut cmd = Command::new("curl");
    cmd.args(["-sfL", "--max-time", PROBE_TIMEOUT_S, "-H", "Range: bytes=0-"]);
    for (k, v) in headers {
        cmd.arg("-H").arg(format!("{k}: {v}"));
    }
    cmd.arg(url).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    let Ok(mut child) = cmd.spawn() else {
        return true; // no curl: undecidable, let the player's retries judge
    };
    let mut buf = [0u8; 1024];
    let mut got = 0usize;
    if let Some(mut out) = child.stdout.take() {
        while got < PROBE_BYTES {
            match out.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => got += n,
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    got > 0
}

fn resolve_with(video: &Video, strategy: &Strategy) -> Result<(Stream, Meta), String> {
    let url = video.url();
    let mut args: Vec<&str> = strategy.extractor_args.to_vec();
    args.extend_from_slice(&["-f", strategy.fmt, "--print", STREAM_PRINT, &url]);
    let (out, err) = run(&args, strategy.auth)?;
    let Some(line) = out.lines().find(|l| l.starts_with("http")) else {
        return Err(if err.trim().is_empty() { "flux audio introuvable".into() } else { explain(&err) });
    };
    let mut f = line.splitn(6, '\t');
    let stream_url = f.next().unwrap_or("").to_string();
    let duration = f.next().and_then(|d| d.parse::<f64>().ok()).map(|d| d as u32);
    let vcodec = f.next().unwrap_or("");
    let headers = parse_headers(f.next().unwrap_or(""));
    let known = |s: Option<&str>| s.filter(|s| !s.is_empty() && *s != "NA").map(String::from);
    let uploader = known(f.next());
    let title = known(f.next());
    let has_video = !matches!(vcodec, "none" | "" | "NA");
    let source = if has_video && !strategy.label.contains("360p") {
        format!("{} 360p", strategy.label)
    } else {
        strategy.label.to_string()
    };
    Ok((
        Stream { url: stream_url, headers, source, has_video },
        Meta { title, uploader, duration },
    ))
}

/// Find a stream URL that has been checked against the real fetch.
///
/// Every strategy is tried in order and validated; only a URL that actually
/// served bytes is handed to the player. `want_video` puts the progressive
/// stream first — the audio-only ones stay in the list, so asking for the
/// clip can never cost the sound.
pub fn resolve(video: &Video, want_video: bool) -> Result<(Stream, Meta), String> {
    let preferred = PREFERRED.load(Ordering::Relaxed);
    let mut order: Vec<(Option<usize>, &Strategy)> = Vec::with_capacity(4);
    if want_video {
        order.push((None, &VIDEO_STRATEGY));
    }
    order.push((Some(preferred), &STRATEGIES[preferred]));
    order.extend(
        STRATEGIES.iter().enumerate().filter(|(i, _)| *i != preferred).map(|(i, s)| (Some(i), s)),
    );
    if cookie_browser().is_some() {
        // Appended, not tried first: the anonymous ones stay cookie-free and
        // already handle every public video on their own, exactly as before
        // "L" existed. AUTH_STRATEGY only gets a turn for whatever they
        // couldn't resolve — actually gated content — instead of adding
        // cookies (and the Android-gets-skipped fallout) to every lookup.
        order.push((None, &AUTH_STRATEGY));
    }
    let mut last = String::from("flux audio introuvable");
    for (index, strategy) in order {
        let (stream, meta) = match resolve_with(video, strategy) {
            Ok(found) => found,
            Err(e) => {
                last = e;
                continue;
            }
        };
        if !stream_playable(&stream.url, &stream.headers) {
            last = format!("403 sur « {} » — YouTube exige un PO token pour ce client", strategy.label);
            continue;
        }
        if let Some(i) = index {
            PREFERRED.store(i, Ordering::Relaxed);
        }
        return Ok((stream, meta));
    }
    Err(last)
}

/// `%(http_headers)j` is a flat object of strings; a full JSON parser would
/// be dead weight for that, so this reads exactly that shape and nothing more.
fn parse_headers(json: &str) -> Headers {
    let mut out = Headers::new();
    let mut chars = json.trim().chars().peekable();
    if chars.next() != Some('{') {
        return out;
    }
    fn string(it: &mut std::iter::Peekable<std::str::Chars>) -> Option<String> {
        while it.peek()?.is_whitespace() {
            it.next();
        }
        if it.next()? != '"' {
            return None;
        }
        let mut s = String::new();
        loop {
            match it.next()? {
                '"' => return Some(s),
                '\\' => match it.next()? {
                    'n' => s.push('\n'),
                    't' => s.push('\t'),
                    'r' => s.push('\r'),
                    'b' | 'f' => {}
                    'u' => {
                        let hex: String = (0..4).filter_map(|_| it.next()).collect();
                        let code = u32::from_str_radix(&hex, 16).ok()?;
                        s.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                    }
                    c => s.push(c),
                },
                c => s.push(c),
            }
        }
    }
    while let Some(key) = string(&mut chars) {
        while chars.peek().is_some_and(|c| c.is_whitespace() || *c == ':') {
            chars.next();
        }
        // Non-string values are skipped: only strings make valid headers.
        if chars.peek() == Some(&'"') {
            let Some(value) = string(&mut chars) else { break };
            out.push((key, value));
        } else {
            while chars.peek().is_some_and(|c| *c != ',' && *c != '}') {
                chars.next();
            }
        }
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        if chars.next() != Some(',') {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_ids() {
        let id = Some("dQw4w9WgXcQ".to_string());
        assert_eq!(parse_video_id("dQw4w9WgXcQ"), id);
        assert_eq!(parse_video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=3"), id);
        assert_eq!(parse_video_id("youtu.be/dQw4w9WgXcQ?si=x"), id);
        assert_eq!(parse_video_id("https://youtube.com/shorts/dQw4w9WgXcQ"), id);
        assert_eq!(parse_video_id("m.youtube.com/embed/dQw4w9WgXcQ"), id);
        assert_eq!(parse_video_id("lofi hip hop"), None);
        assert_eq!(parse_video_id("https://example.com/watch?v=dQw4w9WgXcQ"), None);
    }

    #[test]
    fn playlist_ids() {
        let pl = "PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG";
        assert_eq!(parse_playlist_id(pl).as_deref(), Some(pl));
        let url = format!("https://www.youtube.com/watch?v=dQw4w9WgXcQ&list={pl}");
        assert_eq!(parse_playlist_id(&url).as_deref(), Some(pl));
        assert_eq!(parse_playlist_id("dQw4w9WgXcQ"), None);
        assert_eq!(parse_playlist_id("PLAYLIST"), None);
    }

    #[test]
    fn entries_and_headers() {
        let v = parse_entry("dQw4w9WgXcQ\t212.0\tRick\tNever\tGonna").unwrap();
        assert_eq!((v.duration, v.uploader.as_str(), v.title.as_str()), (Some(212), "Rick", "Never\tGonna"));
        assert!(parse_entry("not-an-id\t1\ta\tb").is_none());
        let h = parse_headers(r#"{"User-Agent": "Mozilla/5.0 \"x\"", "N": 1, "Accept": "*/*"}"#);
        assert_eq!(h, vec![
            ("User-Agent".into(), "Mozilla/5.0 \"x\"".into()),
            ("Accept".into(), "*/*".into()),
        ]);
        assert_eq!(fmt_time(3725.0), "1:02:05");
        assert_eq!(fmt_time(65.0), "1:05");
    }
}
