//! Subprocess and thread plumbing shared by `sources` and `player`.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Worker threads only shuffle a few strings around: a small stack is plenty,
/// and the default 2–8 MiB per thread is address space the app never needs.
const WORKER_STACK: usize = 256 * 1024;

pub fn spawn<F: FnOnce() + Send + 'static>(name: &str, f: F) {
    let _ = thread::Builder::new()
        .name(name.into())
        .stack_size(WORKER_STACK)
        .spawn(f);
}

/// `shutil.which`: first executable called `name` on PATH.
pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| is_executable(p))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

pub struct Output {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Runs `cmd` to completion with both pipes drained concurrently (a full
/// stderr pipe would otherwise wedge the child), killing it past `timeout`.
pub fn run(mut cmd: Command, timeout: Duration) -> std::io::Result<Output> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let status = wait_deadline(&mut child, timeout)?;
    Ok(Output {
        success: status.is_some_and(|s| s.success()),
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> thread::JoinHandle<Vec<u8>> {
    thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        })
        .expect("thread spawn")
}

/// None when the deadline killed it.
fn wait_deadline(
    child: &mut Child,
    timeout: Duration,
) -> std::io::Result<Option<std::process::ExitStatus>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        thread::sleep(Duration::from_millis(40));
    }
}
