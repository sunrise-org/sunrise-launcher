//! Children a worker waits on (a version's preparer and porter, a preparation's process group): the launcher stops those still running when it quits, so none outlives it.
use super::child::Child;
use std::io;
use std::process::{Command, ExitStatus};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// A child process, or the process group it leads, or on Windows the job it runs in.
#[derive(Clone, Debug)]
struct Entry {
    pid: u32,
    group: bool,
    /// A preparer's job: a quit ends the whole job, as a stop does, and not only the leader.
    #[cfg(windows)]
    job: Option<std::sync::Arc<super::leases::CompilerJob>>,
}

/// The children a set of workers own: the launcher's is [`CHILDREN`]; a test keeps its own.
pub struct Registry(Mutex<Vec<Entry>>);

/// The launcher's children, stopped as it quits ([`stop_all`]).
pub static CHILDREN: Registry = Registry::new();

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    pub const fn new() -> Self {
        Self(Mutex::new(Vec::new()))
    }

    fn entries(&self) -> MutexGuard<'_, Vec<Entry>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Spawns `command` as a child of this registry.
    pub fn spawn(&'static self, command: &mut Command) -> io::Result<Owned> {
        let child = command.spawn()?.into();
        Ok(self.track(child))
    }

    fn track(&'static self, child: Child) -> Owned {
        self.track_entry(
            Entry {
                pid: child.id(),
                group: false,
                #[cfg(windows)]
                job: None,
            },
            child,
        )
    }

    /// Tracks a piped preparer: on Unix the process group it leads, on Windows its job.
    pub(crate) fn track_preparer(&'static self, child: Child) -> Owned {
        self.track_entry(
            Entry {
                pid: child.id(),
                group: cfg!(unix),
                #[cfg(windows)]
                job: child.preparer_job(),
            },
            child,
        )
    }

    fn track_entry(&'static self, entry: Entry, child: Child) -> Owned {
        self.entries().push(entry);
        Owned {
            child,
            registry: self,
            reaped: None,
        }
    }

    /// Stops the process group `pid` leads (it was started in its own group) with the others, until [`Registry::release`].
    pub fn track_group(&self, pid: u32) {
        self.entries().push(Entry {
            pid,
            group: true,
            #[cfg(windows)]
            job: None,
        });
    }

    /// Forgets `pid`, which ended.
    pub fn release(&self, pid: u32) {
        self.entries().retain(|entry| entry.pid != pid);
    }

    /// Stops every child and group still running, under the registry's lock: a reap forgets its
    /// child under the same lock first ([`Owned::reap`]), so no signal here reaches a PID that a
    /// reap freed for another process.
    pub fn stop_all(&self) {
        let mut entries = self.entries();
        for entry in std::mem::take(&mut *entries) {
            stop(entry);
        }
    }

    #[cfg(all(test, unix))]
    pub(crate) fn holds(&self, pid: u32) -> bool {
        self.entries().iter().any(|entry| entry.pid == pid)
    }
}

/// A child the launcher stops when it quits; one dropped before it ended is stopped then.
pub struct Owned {
    child: Child,
    registry: &'static Registry,
    /// Its status once reaped. From then on its PID may be another process's, so nothing here
    /// looks at or signals the PID again (review of #670).
    reaped: Option<ExitStatus>,
}

impl Owned {
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        CHILDREN.spawn(command)
    }

    /// Start a piped staged child on the caller's already-admitted profile. The caller
    /// waits for the worker's exit proof before touching or sealing its candidate output.
    pub fn spawn_staged(
        command: Command,
        gate: super::leases::ProfileHandoff<'_>,
        role: super::leases::StagedChild,
    ) -> io::Result<Self> {
        Ok(CHILDREN.track(gate.spawn(command, role)?))
    }

    /// Tracks a piped preparer, which leads its own process group on Unix and runs in its own job
    /// on Windows: the launcher's quit stops the whole group, or ends the whole job.
    pub(crate) fn start_preparer(child: Child) -> Self {
        CHILDREN.track_preparer(child)
    }

    #[cfg(windows)]
    pub(crate) fn spawn_compiler(command: &Command) -> io::Result<Self> {
        Ok(CHILDREN.track(super::leases::spawn_compiler(command)?))
    }

    pub fn child(&mut self) -> &mut Child {
        &mut self.child
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.reaped.is_some() {
            return Ok(self.reaped);
        }
        if !self.child.ended_unreaped(false)? {
            return Ok(None);
        }
        self.reap().map(Some)
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        if let Some(status) = self.reaped {
            return Ok(status);
        }
        drop(self.child.stdin.take());
        self.child.ended_unreaped(true)?;
        self.reap()
    }

    /// Reaps the child, which has ended. The registry forgets it first, under the lock a quit
    /// signals under: once reaped, its PID can go to another process, which a quit must not
    /// signal (review of #670).
    fn reap(&mut self) -> io::Result<ExitStatus> {
        let pid = self.child.id();
        self.registry.release(pid);
        let status = self.child.wait();
        #[cfg(all(test, unix))]
        tests::REAPS.with(|reaps| reaps.borrow_mut().push((pid, self.registry.holds(pid))));
        if let Ok(status) = status {
            self.reaped = Some(status);
        }
        status
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        if self.reaped.is_some() {
            return;
        }
        if !matches!(self.child.ended_unreaped(false), Ok(true)) {
            let _ = self.child.kill();
        }
        let _ = self.child.ended_unreaped(true);
        let _ = self.reap();
    }
}

/// Stops every child the launcher still runs: called as it quits.
pub fn stop_all() {
    CHILDREN.stop_all();
}

#[cfg(unix)]
fn stop(entry: Entry) {
    let pid = entry.pid as libc::pid_t;
    // SAFETY: a signal to a child (or the group it leads) this launcher started and hasn't reaped.
    unsafe {
        libc::kill(if entry.group { -pid } else { pid }, libc::SIGTERM);
    }
}

#[cfg(windows)]
fn stop(entry: Entry) {
    if let Some(job) = &entry.job {
        let _ = job.terminate();
        return;
    }
    if entry.group {
        let _ = std::process::Command::new(super::taskkill())
            .args(["/PID", &entry.pid.to_string(), "/T", "/F"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        return;
    }
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    // SAFETY: the handle is checked before use and closed after.
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, entry.pid);
        if !handle.is_null() {
            TerminateProcess(handle, 1);
            CloseHandle(handle);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    thread_local! {
        /// Each reap on this thread: the PID, and whether the registry still held it right after.
        pub(super) static REAPS: std::cell::RefCell<Vec<(u32, bool)>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    /// Whether the registry still held `pid` just after its reap, once the PID was free for
    /// another process. A quit then would signal whatever got the PID.
    fn held_after_its_reap(pid: u32) -> Option<bool> {
        REAPS.with(|reaps| {
            reaps
                .borrow()
                .iter()
                .find(|(reaped, _)| *reaped == pid)
                .map(|(_, held)| *held)
        })
    }

    /// A child is reaped once. After that its PID may be another process's, so a later wait, look
    /// or drop gives the kept status and never touches the PID (review of #670).
    #[test]
    fn a_reaped_child_is_never_looked_at_again() {
        static OWN: Registry = Registry::new();
        let mut child = OWN.spawn(&mut Command::new("true")).unwrap();
        let pid = child.child().id();
        let status = child.wait().unwrap();
        assert_eq!(child.try_wait().unwrap(), Some(status));
        assert_eq!(child.wait().unwrap(), status);
        drop(child);
        let reaps = REAPS.with(|reaps| {
            reaps
                .borrow()
                .iter()
                .filter(|(reaped, _)| *reaped == pid)
                .count()
        });
        assert_eq!(reaps, 1, "reaped once, and its PID left alone after");
    }

    /// Every way a child is reaped, a wait, a polled end and a drop, forgets it in the registry
    /// before the reap that frees its PID, so a quit racing the reap can't signal a reused PID
    /// or group (review of #670).
    #[test]
    fn every_reap_forgets_the_child_before_its_pid_is_freed() {
        static OWN: Registry = Registry::new();
        let mut waited = OWN.spawn(&mut Command::new("true")).unwrap();
        let pid = waited.child().id();
        assert!(waited.wait().unwrap().success());
        assert_eq!(held_after_its_reap(pid), Some(false), "a wait");

        let mut polled = OWN.spawn(&mut Command::new("true")).unwrap();
        let pid = polled.child().id();
        while polled.try_wait().unwrap().is_none() {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(held_after_its_reap(pid), Some(false), "a polled end");

        let mut dropped = OWN.spawn(Command::new("sleep").arg("30")).unwrap();
        let pid = dropped.child().id();
        drop(dropped);
        assert_eq!(held_after_its_reap(pid), Some(false), "a drop");
        assert!(!OWN.holds(pid));
    }

    #[test]
    #[ignore = "daily"]
    fn quitting_stops_the_children_and_groups_still_running() {
        use std::os::unix::process::CommandExt;
        static OWN: Registry = Registry::new();
        let mut child = OWN.spawn(Command::new("sleep").arg("30")).unwrap();
        let pid = child.child().id();
        let mut group = Command::new("sh")
            .args(["-c", "sleep 30 & wait"])
            .process_group(0)
            .spawn()
            .unwrap();
        OWN.track_group(group.id());
        assert!(OWN.holds(pid) && OWN.holds(group.id()));
        OWN.stop_all();
        assert!(!child.wait().unwrap().success(), "stopped by its signal");
        assert!(!group.wait().unwrap().success(), "its group stopped too");
        assert!(!OWN.holds(pid));
    }
}
