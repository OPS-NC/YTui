//! Resident memory of the TUI plus every process it spawned (ffmpeg, and
//! yt-dlp / curl while they run).
//!
//! Linux reads straight from /proc — one file read per process. macOS has no
//! /proc, but libproc answers the same questions with two syscalls per
//! process; the Python version forked `ps` once a second for this, which cost
//! more than everything it was measuring.

/// Bytes resident for this process tree.
pub fn total_rss() -> u64 {
    let me = std::process::id() as i32;
    let mut total = 0u64;
    let mut seen: Vec<i32> = Vec::with_capacity(8);
    let mut stack = vec![me];
    while let Some(pid) = stack.pop() {
        if seen.contains(&pid) {
            continue;
        }
        seen.push(pid);
        total += rss(pid);
        children(pid, &mut stack);
    }
    total
}

#[cfg(target_os = "linux")]
fn rss(pid: i32) -> u64 {
    // statm: size resident shared ... (in pages)
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(0) as u64;
    std::fs::read_to_string(format!("/proc/{pid}/statm"))
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
        .map_or(0, |pages| pages * page)
}

#[cfg(target_os = "linux")]
fn children(pid: i32, out: &mut Vec<i32>) {
    let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) else { return };
    for task in tasks.flatten() {
        let path = task.path().join("children");
        if let Ok(text) = std::fs::read_to_string(path) {
            out.extend(text.split_whitespace().filter_map(|p| p.parse::<i32>().ok()));
        }
    }
}

#[cfg(target_os = "macos")]
fn rss(pid: i32) -> u64 {
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
    let got = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTASKINFO,
            0,
            (&raw mut info).cast(),
            size,
        )
    };
    if got == size { info.pti_resident_size } else { 0 }
}

#[cfg(target_os = "macos")]
fn children(pid: i32, out: &mut Vec<i32>) {
    let mut pids = [0 as libc::pid_t; 64];
    let n = unsafe {
        libc::proc_listchildpids(
            pid,
            pids.as_mut_ptr().cast(),
            std::mem::size_of_val(&pids) as libc::c_int,
        )
    };
    if n > 0 {
        out.extend(pids.iter().take(n as usize).copied().filter(|&p| p > 0));
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn rss(_pid: i32) -> u64 {
    0
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn children(_pid: i32, _out: &mut Vec<i32>) {}

/// `12,345.6 MB`.
pub fn human(nbytes: u64) -> String {
    let tenths = (nbytes as f64 / (1024.0 * 1024.0) * 10.0).round() as u64;
    let (int, frac) = (tenths / 10, tenths % 10);
    let digits = int.to_string();
    let mut grouped = String::with_capacity(digits.len() + 4);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{grouped}.{frac} MB")
}
