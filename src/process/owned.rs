//! Children a worker waits on (a version's preparer and porter, a preparation's process group): the launcher stops those still running when it quits, so none outlives it.
use super::child::Child;
use std::io;
use std::process::{Command, ExitStatus};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// A child process, or the process group it leads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    pid: u32,
    group: bool,
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
        self.entries().push(Entry {
            pid: child.id(),
            group: false,
        });
        Owned {
            child,
            registry: self,
        }
    }

    /// Stops the process group `pid` leads (it was started in its own group) with the others, until [`Registry::release`].
    pub fn track_group(&self, pid: u32) {
        self.entries().push(Entry { pid, group: true });
    }

    /// Forgets `pid`, which ended.
    pub fn release(&self, pid: u32) {
        self.entries().retain(|entry| entry.pid != pid);
    }

    /// Stops every child and group still running.
    pub fn stop_all(&self) {
        for entry in std::mem::take(&mut *self.entries()) {
            stop(entry);
        }
    }

    #[cfg(all(test, unix))]
    fn holds(&self, pid: u32) -> bool {
        self.entries().iter().any(|entry| entry.pid == pid)
    }
}

/// A child the launcher stops when it quits; one dropped before it ended is stopped then.
pub struct Owned {
    child: Child,
    registry: &'static Registry,
}

impl Owned {
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        CHILDREN.spawn(command)
    }

    pub(crate) fn spawn_profile(
        command: &mut Command,
        lease: &crate::browser::sunrise::txn::Lease,
    ) -> io::Result<Self> {
        #[cfg(unix)]
        {
            lease.inherit(command)?;
            Self::spawn(command)
        }
        #[cfg(windows)]
        {
            let child = super::leases::spawn_profile(command, lease, true)?;
            Ok(CHILDREN.track(child))
        }
    }

    #[cfg(windows)]
    pub(crate) fn spawn_compiler(
        command: &Command,
        lease: Option<&crate::browser::sunrise::txn::Lease>,
    ) -> io::Result<Self> {
        Ok(CHILDREN.track(super::leases::spawn_compiler(command, lease)?))
    }

    pub fn child(&mut self) -> &mut Child {
        &mut self.child
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self.child.wait();
        self.registry.release(self.child.id());
        status
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        self.registry.release(self.child.id());
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
