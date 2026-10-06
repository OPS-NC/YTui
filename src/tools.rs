//! Where the external programs come from: ffmpeg, the JS runtime yt-dlp
//! needs, and yt-dlp itself.
//!
//! ffmpeg and quickjs-ng's `qjs` are embedded in the binary (see build.rs and
//! scripts/build-bundle.sh) and extracted once to ~/.local/share/ytui/bin.
//! The embedded ffmpeg is cut down to what ytui runs, which makes it lighter
//! resident than a distribution build (12 Mo against 20 Mo here), so it is
//! preferred; the system one is the fallback (a musl distro, say, where the
//! glibc-linked Linux bundle cannot start).
//!
//! yt-dlp cannot be frozen into the binary: YouTube changes its player every
//! few weeks and a stale yt-dlp stops resolving anything. So ytui keeps its
//! own copy, installed on first launch and checked for a new release once a
//! day, verified against the release's SHA2-256SUMS before it replaces the
//! old one. Downloads go through curl, like everything else that touches the
//! network: no HTTP or TLS stack lives in this process.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use crate::exec;

/// ~/.local/share/ytui (XDG_DATA_HOME honoured).
pub fn data_dir() -> Option<PathBuf> {
    let root = match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(base) => PathBuf::from(base),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".local/share"),
    };
    Some(root.join("ytui"))
}

fn bin_dir() -> Option<PathBuf> {
    let dir = data_dir()?.join("bin");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

// ------------------------------------------------------------- embedded

#[cfg(bundled)]
mod embedded {
    pub static FFMPEG: &[u8] = include_bytes!(env!("YTUI_BUNDLE_FFMPEG"));
    pub static QJS: &[u8] = include_bytes!(env!("YTUI_BUNDLE_QJS"));
    pub const ID: &str = env!("YTUI_BUNDLE_ID");
}

/// Writes an embedded program to bin/<name> unless the copy there already
/// comes from this very build.
#[cfg(bundled)]
fn extract(name: &str, bytes: &'static [u8]) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let dir = bin_dir()?;
    let path = dir.join(name);
    let stamp = dir.join(format!("{name}.id"));
    if path.is_file() && std::fs::read_to_string(&stamp).ok().as_deref() == Some(embedded::ID) {
        return Some(path);
    }
    let tmp = dir.join(format!(".{name}.{}", std::process::id()));
    std::fs::write(&tmp, bytes).ok()?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).ok()?;
    std::fs::rename(&tmp, &path).ok()?;
    let _ = std::fs::write(&stamp, embedded::ID);
    release(bytes);
    Some(path)
}

/// The embedded bytes were paged in to be copied out; they are never read
/// again, so hand those pages back instead of letting them count as resident.
#[cfg(bundled)]
fn release(bytes: &'static [u8]) {
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(4096) as usize;
    let start = bytes.as_ptr() as usize;
    let aligned = start.div_ceil(page) * page;
    let end = (start + bytes.len()) / page * page;
    if end > aligned {
        unsafe { libc::madvise(aligned as *mut libc::c_void, end - aligned, libc::MADV_DONTNEED) };
    }
}

fn starts(path: &Path) -> bool {
    let mut cmd = Command::new(path);
    cmd.arg("-version");
    exec::run(cmd, Duration::from_secs(5)).is_ok_and(|o| o.success)
}

/// The ffmpeg to run: the embedded one when it starts here, else PATH's.
pub fn ffmpeg() -> &'static Path {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        #[cfg(bundled)]
        if let Some(path) = extract("ffmpeg", embedded::FFMPEG).filter(|p| starts(p)) {
            return path;
        }
        exec::which("ffmpeg").unwrap_or_else(|| PathBuf::from("ffmpeg"))
    })
}

/// Options to put before every https `-i` of the ffmpeg above.
///
/// ffmpeg 9 verifies TLS peers by default. The macOS bundle (SecureTransport)
/// and distribution builds (OpenSSL/GnuTLS) find the system's trust store on
/// their own; the Linux bundle's static mbedTLS has none and needs the CA
/// file spelled out — without it every https input fails with "Input/output
/// error". The file's location differs per distribution. With no bundle at
/// all, verification is turned off rather than making playback impossible:
/// the URLs come signed from yt-dlp, which fetched them over verified TLS.
pub fn ffmpeg_tls_args() -> &'static [String] {
    static ARGS: OnceLock<Vec<String>> = OnceLock::new();
    ARGS.get_or_init(|| {
        let embedded = data_dir().is_some_and(|d| ffmpeg().starts_with(d));
        if !cfg!(target_os = "linux") || !embedded {
            return Vec::new();
        }
        let from_env = std::env::var_os("SSL_CERT_FILE").map(PathBuf::from);
        let known = [
            "/etc/ssl/certs/ca-certificates.crt",                // Debian, Ubuntu, Arch, Gentoo
            "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem", // Fedora, RHEL
            "/etc/pki/tls/certs/ca-bundle.crt",                  // older RHEL
            "/etc/ssl/ca-bundle.pem",                            // openSUSE
            "/etc/ssl/cert.pem",                                 // Alpine, Void
        ];
        let found = from_env.into_iter().chain(known.iter().map(PathBuf::from)).find(|p| p.is_file());
        match found {
            Some(ca) => vec!["-ca_file".into(), ca.to_string_lossy().into_owned()],
            None => vec!["-tls_verify".into(), "0".into()],
        }
    })
}

pub fn ffmpeg_available() -> bool {
    let path = ffmpeg();
    path.is_absolute() && path.is_file()
}

/// `--js-runtimes` for yt-dlp, which needs a JS engine to solve YouTube's
/// signature and `n` challenges (without one the URLs it returns are
/// throttled below playback speed). An installed deno/node/bun wins — they
/// solve faster than quickjs —, the embedded qjs guarantees there is one.
pub fn js_runtime() -> Option<&'static str> {
    static ARG: OnceLock<Option<String>> = OnceLock::new();
    ARG.get_or_init(|| {
        if let Some(rt) = ["deno", "node", "bun"].into_iter().find(|r| exec::which(r).is_some()) {
            return Some(rt.to_string());
        }
        let qjs = embedded_qjs().or_else(|| exec::which("qjs"))?;
        Some(format!("quickjs:{}", qjs.display()))
    })
    .as_deref()
}

#[cfg(bundled)]
fn embedded_qjs() -> Option<PathBuf> {
    extract("qjs", embedded::QJS).filter(|p| {
        let mut cmd = Command::new(p);
        cmd.arg("--help"); // qjs has no --version; --help exits 1 but prints
        exec::run(cmd, Duration::from_secs(5)).is_ok_and(|o| !o.stdout.is_empty())
    })
}

#[cfg(not(bundled))]
fn embedded_qjs() -> Option<PathBuf> {
    None
}

// --------------------------------------------------------------- yt-dlp

const RELEASES: &str = "https://github.com/yt-dlp/yt-dlp/releases";
const CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);

fn ytdlp_dir() -> Option<PathBuf> {
    Some(bin_dir()?.join("yt-dlp"))
}

/// The managed copy's executable, if installed. `ENTRY` names it, since the
/// zipapp and the frozen builds don't share a file name.
fn managed_ytdlp() -> Option<PathBuf> {
    let dir = ytdlp_dir()?;
    let entry = std::fs::read_to_string(dir.join("ENTRY")).ok()?;
    let exe = dir.join(entry.trim());
    exe.is_file().then_some(exe)
}

/// YTUI_YTDLP pins a yt-dlp (a path, or a name looked up on PATH) and turns
/// the managed copy off.
fn pinned_ytdlp() -> Option<String> {
    std::env::var("YTUI_YTDLP").ok().filter(|v| !v.trim().is_empty())
}

pub fn ytdlp_cmd() -> Command {
    if let Some(pinned) = pinned_ytdlp() {
        return Command::new(pinned);
    }
    // A yt-dlp shipped next to the binary wins, then the managed copy, then
    // PATH, then the Python module.
    let local = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("yt-dlp")))
        .filter(|p| p.is_file());
    if let Some(path) = local.or_else(managed_ytdlp).or_else(|| exec::which("yt-dlp")) {
        return Command::new(path);
    }
    let mut cmd = Command::new("python3");
    cmd.args(["-m", "yt_dlp"]);
    cmd
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Zipapp, // single file, run by the system's python3
    Frozen, // PyInstaller "onedir" build, shipped as a zip
}

/// Which release asset fits this machine.
///
/// The zipapp starts in 0.3 s but needs python ≥ 3.10 — common on Linux, not
/// on macOS (the system python3 is 3.9). The frozen *onefile* builds unpack
/// themselves into /tmp on every single call (5 s per call on macOS, where
/// each unpacked library is re-verified), so the *onedir* zip is used
/// instead: 0.17 s per call, at the price of updating it here rather than
/// with `yt-dlp -U`.
fn pick_asset() -> Option<(String, Kind)> {
    if cfg!(target_os = "linux") && python_ok() {
        return Some(("yt-dlp".into(), Kind::Zipapp));
    }
    let name = if cfg!(target_os = "macos") {
        "yt-dlp_macos"
    } else {
        let musl = std::fs::read_dir("/lib")
            .map(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with("ld-musl")))
            .unwrap_or(false);
        match (cfg!(target_arch = "aarch64"), musl) {
            (false, false) => "yt-dlp_linux",
            (true, false) => "yt-dlp_linux_aarch64",
            (false, true) => "yt-dlp_musllinux",
            (true, true) => "yt-dlp_musllinux_aarch64",
        }
    };
    unzip_tool()?;
    Some((format!("{name}.zip"), Kind::Frozen))
}

fn python_ok() -> bool {
    let mut cmd = Command::new("python3");
    cmd.args(["-c", "import sys; print(sys.version_info >= (3, 10))"]);
    exec::run(cmd, Duration::from_secs(5)).is_ok_and(|o| o.stdout.starts_with(b"True"))
}

fn unzip_tool() -> Option<&'static str> {
    ["ditto", "unzip", "bsdtar"].into_iter().find(|t| exec::which(t).is_some())
}

fn curl(args: &[&str]) -> Result<Vec<u8>, String> {
    let mut cmd = Command::new("curl");
    cmd.args(["-fsSL", "--retry", "2", "--max-time", "300"]).args(args);
    let out = exec::run(cmd, Duration::from_secs(320)).map_err(|e| format!("curl : {e}"))?;
    if !out.success {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("curl : {}", err.trim()));
    }
    Ok(out.stdout)
}

/// Latest release tag, read from where /releases/latest redirects to.
fn latest_tag() -> Result<String, String> {
    let out = curl(&["-o", "/dev/null", "-w", "%{url_effective}", &format!("{RELEASES}/latest")])?;
    let url = String::from_utf8_lossy(&out);
    url.rsplit_once("/tag/")
        .map(|(_, tag)| tag.trim().to_string())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| "version de yt-dlp introuvable".into())
}

fn sha256(path: &Path) -> Option<String> {
    for (tool, args) in [("shasum", &["-a", "256"][..]), ("sha256sum", &[][..])] {
        let mut cmd = Command::new(tool);
        cmd.args(args).arg(path);
        if let Ok(out) = exec::run(cmd, Duration::from_secs(60))
            && out.success
        {
            let text = String::from_utf8_lossy(&out.stdout);
            return text.split_whitespace().next().map(str::to_ascii_lowercase);
        }
    }
    None
}

/// Installs or refreshes the managed yt-dlp. Runs on a worker at launch;
/// `say` reports what happened, for the status bar. Quiet when there is
/// nothing to do, which is almost always: at most one HTTP HEAD a day.
pub fn maintain_ytdlp(say: impl Fn(String)) {
    if pinned_ytdlp().is_some() {
        return;
    }
    let Some(dir) = ytdlp_dir() else { return };
    let installed = managed_ytdlp()
        .and_then(|_| std::fs::read_to_string(dir.join("VERSION")).ok())
        .map(|v| v.trim().to_string());
    let stamp = dir.with_file_name(".yt-dlp.checked");
    let fresh = std::fs::metadata(&stamp)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < CHECK_EVERY);
    if installed.is_some() && fresh {
        return;
    }
    let tag = match latest_tag() {
        Ok(tag) => tag,
        Err(e) if installed.is_none() => return say(format!("installation de yt-dlp impossible — {e}")),
        Err(_) => return, // offline: keep the copy we have, try again next launch
    };
    let _ = std::fs::write(&stamp, &tag);
    if installed.as_deref() == Some(tag.as_str()) {
        return;
    }
    let Some((asset, kind)) = pick_asset() else {
        return say("installation de yt-dlp impossible — ni python ≥ 3.10 ni outil de décompression".into());
    };
    say(match &installed {
        None => format!("installation de yt-dlp {tag}…"),
        Some(old) => format!("mise à jour de yt-dlp {old} → {tag}…"),
    });
    match install(&dir, &tag, &asset, kind) {
        Ok(()) => say(format!("yt-dlp {tag} installé")),
        Err(e) => say(format!("yt-dlp {tag} non installé — {e}")),
    }
}

fn install(dir: &Path, tag: &str, asset: &str, kind: Kind) -> Result<(), String> {
    let staging = dir.with_file_name(format!(".yt-dlp.new.{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let result = stage(&staging, tag, asset, kind);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
        return result;
    }
    // Swap whole directories: a yt-dlp still running from the old copy keeps
    // its open files, and no reader ever sees a half-written tree.
    let old = dir.with_file_name(format!(".yt-dlp.old.{}", std::process::id()));
    if dir.exists() {
        std::fs::rename(dir, &old).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&staging, dir).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

fn stage(staging: &Path, tag: &str, asset: &str, kind: Kind) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let base = format!("{RELEASES}/download/{tag}");
    let sums = String::from_utf8_lossy(&curl(&[&format!("{base}/SHA2-256SUMS")])?).into_owned();
    let expected = sums
        .lines()
        .filter_map(|l| l.split_once(char::is_whitespace))
        .find(|(_, name)| name.trim().trim_start_matches('*') == asset)
        .map(|(hash, _)| hash.to_ascii_lowercase())
        .ok_or("somme de contrôle absente")?;
    let file = staging.join(asset);
    let path = file.to_string_lossy().into_owned();
    curl(&["-o", &path, &format!("{base}/{asset}")])?;
    if sha256(&file).as_deref() != Some(expected.as_str()) {
        return Err("somme de contrôle invalide".into());
    }
    let entry = match kind {
        Kind::Zipapp => {
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| e.to_string())?;
            asset.to_string()
        }
        Kind::Frozen => {
            let dest = staging.to_string_lossy().into_owned();
            let tool = unzip_tool().ok_or("aucun outil de décompression")?;
            let mut cmd = Command::new(tool);
            match tool {
                "ditto" => cmd.args(["-x", "-k", &path, &dest]),
                "unzip" => cmd.args(["-q", "-o", &path, "-d", &dest]),
                _ => cmd.args(["-xf", &path, "-C", &dest]),
            };
            let out = exec::run(cmd, Duration::from_secs(120)).map_err(|e| e.to_string())?;
            if !out.success {
                return Err("décompression échouée".into());
            }
            let _ = std::fs::remove_file(&file);
            asset.trim_end_matches(".zip").to_string()
        }
    };
    if !staging.join(&entry).is_file() {
        return Err("exécutable absent de l'archive".into());
    }
    std::fs::write(staging.join("ENTRY"), &entry).map_err(|e| e.to_string())?;
    std::fs::write(staging.join("VERSION"), tag).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ytui-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: tests touching XDG_DATA_HOME each run in their own process
        // invocation (see below); nothing else reads it concurrently here.
        unsafe { std::env::set_var("XDG_DATA_HOME", &dir) };
        dir
    }

    /// Offline: the embedded programs land on disk and start.
    #[cfg(bundled)]
    #[test]
    fn embedded_tools_extract_and_start() {
        let dir = scratch("embed");
        let ffmpeg = ffmpeg();
        assert!(ffmpeg.starts_with(&dir), "embedded ffmpeg preferred: {}", ffmpeg.display());
        assert!(starts(ffmpeg));
        let qjs = embedded_qjs().expect("qjs extracted");
        assert!(qjs.starts_with(&dir));
        // A second extraction is a no-op: same build id, file untouched.
        let before = std::fs::metadata(&qjs).unwrap().modified().unwrap();
        extract("qjs", embedded::QJS).unwrap();
        assert_eq!(before, std::fs::metadata(&qjs).unwrap().modified().unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Network (not YouTube): https input through the ffmpeg ytui picked,
    /// with its TLS options. `cargo test -- --ignored ffmpeg_https`.
    #[test]
    #[ignore]
    fn ffmpeg_https() {
        let dir = scratch("https");
        let mut cmd = Command::new(ffmpeg());
        cmd.args(["-nostdin", "-loglevel", "error"])
            .args(ffmpeg_tls_args())
            .args(["-i", "https://upload.wikimedia.org/wikipedia/commons/c/c8/Example.ogg"])
            .args(["-t", "1", "-ac", "1", "-f", "s16le", "pipe:1"]);
        let out = exec::run(cmd, Duration::from_secs(30)).unwrap();
        println!("{} {:?}: {}", ffmpeg().display(), ffmpeg_tls_args(), String::from_utf8_lossy(&out.stderr));
        assert!(!out.stdout.is_empty(), "no audio over https");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Network (GitHub only): `cargo test -- --ignored managed_ytdlp`.
    #[test]
    #[ignore]
    fn managed_ytdlp_installs() {
        let dir = scratch("ytdlp");
        let said = std::sync::Mutex::new(Vec::new());
        maintain_ytdlp(|s| said.lock().unwrap().push(s));
        let said = said.into_inner().unwrap();
        println!("{said:?}");
        let exe = managed_ytdlp().expect("installed");
        let mut cmd = Command::new(&exe);
        cmd.arg("--version");
        let out = exec::run(cmd, Duration::from_secs(60)).unwrap();
        let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
        println!("{} → {version}", exe.display());
        assert!(said.last().unwrap().ends_with("installé"));
        // Checked today: a second run stays silent.
        let again = std::sync::Mutex::new(Vec::new());
        maintain_ytdlp(|s| again.lock().unwrap().push(s));
        assert!(again.into_inner().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
