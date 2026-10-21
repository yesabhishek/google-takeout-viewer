//! Drive care. Takeout libraries often live on old external hard drives, so
//! every read of an original file goes through `DRIVE`:
//!
//! - only a few files are read at once (one in Gentle mode), with one extra
//!   slot kept for what is on screen so browsing stays responsive;
//! - background work rests after each file in proportion to how long the drive
//!   took, so a slow or struggling drive automatically gets more breathing room;
//! - a run of read errors pauses background work instead of retrying in a
//!   tight loop.
//!
//! This module only paces reads. Nothing in the app writes to, moves or deletes
//! anything inside a Takeout folder: exports and caches are refused there (see
//! `inside`), and "open in another app" opens a read-only copy.

use std::cell::Cell;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Gentle,
    Balanced,
    Fast,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "gentle" => Some(Mode::Gentle),
            "balanced" => Some(Mode::Balanced),
            "fast" => Some(Mode::Fast),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Gentle => "gentle",
            Mode::Balanced => "balanced",
            Mode::Fast => "fast",
        }
    }

    /// (background reads at once, rest after each one as a fraction of the
    /// time it took)
    fn limits(self) -> (usize, f64) {
        match self {
            Mode::Gentle => (1, 0.5),
            Mode::Balanced => (2, 0.15),
            Mode::Fast => (cpus().clamp(2, 8), 0.0),
        }
    }
}

fn cpus() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Prio {
    /// Something the user is looking at right now.
    Interactive,
    /// Indexing, thumbnail pre-generation and export.
    Background,
}

const MAX_REST: Duration = Duration::from_secs(2);
const ERROR_RUN: u32 = 5;
const ERROR_PAUSE: Duration = Duration::from_secs(5);

struct State {
    mode: Mode,
    active: usize,
    background: usize,
    ui_waiting: usize,
    /// Earliest time the next background read may start.
    next_bg: Option<Instant>,
    /// Consecutive read errors.
    errors: u32,
}

pub struct Drive {
    st: Mutex<State>,
    cv: Condvar,
}

pub static DRIVE: Drive = Drive::new();

thread_local! {
    // A thread that already holds a permit reads under it; asking again would
    // deadlock in Gentle mode.
    static HELD: Cell<bool> = const { Cell::new(false) };
}

/// Held while reading an original. Dropping it lets the next read start.
pub struct Permit {
    drive: &'static Drive,
    prio: Option<Prio>,
    started: Instant,
}

impl Drive {
    pub const fn new() -> Drive {
        Drive {
            st: Mutex::new(State {
                mode: Mode::Gentle,
                active: 0,
                background: 0,
                ui_waiting: 0,
                next_bg: None,
                errors: 0,
            }),
            cv: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_mode(&self, mode: Mode) {
        self.lock().mode = mode;
        self.cv.notify_all();
    }

    pub fn mode(&self) -> Mode {
        self.lock().mode
    }

    /// Wait for a turn to read from the drive.
    pub fn acquire(&'static self, prio: Prio) -> Permit {
        if HELD.get() {
            return Permit { drive: self, prio: None, started: Instant::now() };
        }
        let mut st = self.lock();
        if prio == Prio::Interactive {
            st.ui_waiting += 1;
        }
        loop {
            let (limit, _) = st.mode.limits();
            let now = Instant::now();
            let wait = match prio {
                Prio::Interactive if st.active <= limit => None,
                Prio::Interactive => Some(Duration::from_secs(1)),
                Prio::Background if st.background >= limit || st.active >= limit || st.ui_waiting > 0 => {
                    Some(Duration::from_secs(1))
                }
                Prio::Background => st.next_bg.filter(|t| *t > now).map(|t| t - now),
            };
            match wait {
                None => break,
                Some(d) => st = self.cv.wait_timeout(st, d).unwrap_or_else(|e| e.into_inner()).0,
            }
        }
        match prio {
            Prio::Interactive => st.ui_waiting -= 1,
            Prio::Background => st.background += 1,
        }
        st.active += 1;
        HELD.set(true);
        Permit { drive: self, prio: Some(prio), started: Instant::now() }
    }

    /// Record whether a raw read of an original worked. A run of failures
    /// (a dying or disconnecting drive) slows background work right down.
    pub fn report(&self, ok: bool) {
        let mut st = self.lock();
        st.errors = if ok { 0 } else { st.errors.saturating_add(1) };
    }

    /// True while recent reads keep failing.
    pub fn struggling(&self) -> bool {
        self.lock().errors >= ERROR_RUN
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        let Some(prio) = self.prio else { return };
        HELD.set(false);
        let mut st = self.drive.lock();
        st.active -= 1;
        if prio == Prio::Background {
            st.background -= 1;
            let (_, rest) = st.mode.limits();
            let mut pause = self.started.elapsed().mul_f64(rest).min(MAX_REST);
            if st.errors >= ERROR_RUN {
                pause = pause.max(ERROR_PAUSE);
            }
            if !pause.is_zero() {
                st.next_bg = Some(Instant::now() + pause);
            }
        }
        drop(st);
        self.drive.cv.notify_all();
    }
}

/// The drive mode "Auto" picks for a set of library folders, with the reason
/// shown in Library & storage.
pub fn auto_mode(sources: &[PathBuf]) -> (Mode, &'static str) {
    let mut mode = (Mode::Balanced, "Your library is on this computer's internal disk.");
    for s in sources {
        if !is_external(s) {
            continue;
        }
        if solid_state(s) == Some(true) {
            mode = (Mode::Balanced, "Your library is on an external SSD.");
        } else {
            return (Mode::Gentle, "Your library is on an external drive, so it is read one file at a time.");
        }
    }
    mode
}

/// Whether a path lives on an external or secondary volume.
pub fn is_external(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        path.starts_with("/Volumes")
    }
    #[cfg(target_os = "windows")]
    {
        let sys = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()).to_uppercase();
        let p = path.to_string_lossy().to_uppercase();
        !p.trim_start_matches(r"\\?\").starts_with(&sys)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        ["/media", "/mnt", "/run/media"].iter().any(|m| path.starts_with(m))
    }
}

/// Ask the OS whether the volume holding `path` is an SSD. `None` when it
/// can't tell (many USB enclosures hide it), which is treated as a hard drive.
fn solid_state(path: &Path) -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        let mut comps = path.components();
        let mount: PathBuf = [comps.next()?, comps.next()?, comps.next()?].iter().collect();
        let mut cmd = Command::new("/usr/sbin/diskutil");
        cmd.arg("info").arg("-plist").arg(&mount);
        let out = String::from_utf8(run_captured(cmd, Duration::from_secs(5))?).ok()?;
        let rest = &out[out.find("<key>SolidState</key>")? + 21..];
        Some(rest.trim_start().starts_with("<true/>"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        None
    }
}

/// Run a helper program and return its stdout, killing it if it takes longer
/// than `limit` (a damaged file can make a decoder hang forever).
pub fn run_captured(mut cmd: Command, limit: Duration) -> Option<Vec<u8>> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = reader.join().ok()?;
                return status.success().then_some(out);
            }
            Ok(None) if started.elapsed() < limit => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// Resolve links in the part of `p` that exists (a destination folder may not
/// have been created yet).
fn resolve(p: &Path) -> PathBuf {
    if let Ok(c) = p.canonicalize() {
        return c;
    }
    match (p.parent(), p.file_name()) {
        (Some(parent), Some(name)) => resolve(parent).join(name),
        _ => p.to_path_buf(),
    }
}

/// Whether `path` is `root` or somewhere inside it. Case-insensitive where the
/// file system usually is (macOS, Windows).
pub fn inside(path: &Path, root: &Path) -> bool {
    let (p, r) = (resolve(path), resolve(root));
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        let low = |p: &Path| PathBuf::from(p.to_string_lossy().to_lowercase());
        low(&p).starts_with(low(&r))
    } else {
        p.starts_with(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn gentle_reads_one_file_at_a_time() {
        static D: Drive = Drive::new();
        D.set_mode(Mode::Gentle);
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..6)
            .map(|_| {
                let (active, peak) = (active.clone(), peak.clone());
                std::thread::spawn(move || {
                    for _ in 0..3 {
                        let _p = D.acquire(Prio::Background);
                        let n = active.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(n, Ordering::SeqCst);
                        // Nested reads on the same thread must not deadlock.
                        drop(D.acquire(Prio::Background));
                        std::thread::sleep(Duration::from_millis(2));
                        active.fetch_sub(1, Ordering::SeqCst);
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(peak.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn helper_timeout() {
        let mut cmd = Command::new("sleep");
        cmd.arg("5");
        let t = Instant::now();
        assert!(run_captured(cmd, Duration::from_millis(200)).is_none());
        assert!(t.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn inside_paths() {
        let dir = std::env::temp_dir();
        assert!(inside(&dir.join("not-created-yet/b"), &dir));
        assert!(inside(&dir, &dir));
        assert!(!inside(Path::new("/definitely/elsewhere"), &dir));
        assert!(!inside(&dir.parent().unwrap().join("other"), &dir));
    }
}
