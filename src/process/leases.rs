//! Child-owned kernel claims. Not wired into managed launch until the complete safety stack.
use crate::{
    native_cache::{Error, LockMode, Release},
    run_leases::{Claim, Registry},
};
use cap_std::fs::Dir;
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
};

/// Independent open descriptions: dropping the creator never explicitly unlocks the child.
pub struct ChildLeases {
    claims: Vec<Claim>,
    data: PathBuf,
}
impl ChildLeases {
    /// Acquire the shared gate, then each held tree. Fails immediately on contention.
    pub fn acquire(data: &Path, trees: &[Dir]) -> io::Result<Self> {
        let registry = Registry::with_release(data, Release::Close).map_err(error)?;
        let claims = trees
            .iter()
            .map(|tree| registry.lock(tree, LockMode::Shared).map_err(error))
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Self {
            claims,
            data: data.canonicalize()?,
        })
    }
    /// Create exactly one native child inside the isolated holder. Inherits that holder's cwd
    /// and stdio; the supplied environment is complete, not an overlay. No launcher-global
    /// handle is made inheritable. A refused breakaway is returned for the caller's job policy.
    #[cfg(windows)]
    pub fn spawn_windows(
        self,
        program: &Path,
        args: &[std::ffi::OsString],
        environment: &[(std::ffi::OsString, std::ffi::OsString)],
        breakaway: bool,
    ) -> io::Result<WindowsChild> {
        let child = windows::spawn(
            &self.files(),
            &self.data,
            program,
            args,
            environment,
            breakaway,
        );
        drop(self);
        child
    }

    fn files(&self) -> Vec<&File> {
        self.claims.iter().flat_map(Claim::files).collect()
    }
    /// Only the chosen fork clears CLOEXEC. Consume the command, so stale pre_exec fds cannot
    /// be reused after these owners drop. No allocation or lock is taken in the child callback.
    #[cfg(unix)]
    pub fn spawn(self, mut command: std::process::Command) -> io::Result<std::process::Child> {
        use std::os::{
            fd::{AsRawFd, FromRawFd},
            unix::process::CommandExt,
        };
        let transfer = self
            .files()
            .iter()
            .map(|file| {
                // SAFETY: file is owned; duplicate above stdio so Command cannot replace it.
                let fd = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
                if fd < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    // SAFETY: fcntl returned a new owned descriptor; File closes without unlock.
                    Ok(unsafe { File::from_raw_fd(fd) })
                }
            })
            .collect::<io::Result<Vec<_>>>()?;
        let fds: Vec<_> = transfer.iter().map(AsRawFd::as_raw_fd).collect();
        command.env("SUNRISE_RUNTIME_DATA_ROOT", &self.data).env(
            "SUNRISE_RUNTIME_LEASE_FDS",
            fds.iter().map(i32::to_string).collect::<Vec<_>>().join(","),
        );
        // SAFETY: only async-signal-safe fcntl calls run after fork. transfer owns every fd until
        // spawn returns, and the creator's copies keep their original CLOEXEC flags.
        unsafe {
            command.pre_exec(move || {
                // Command's fork path can allocate a pipe at a closed stdio slot. Its
                // dup2(fd, fd) leaves CLOEXEC set, so exec would discard that pipe.
                // Stdio setup is complete here; preserve every configured stdio handle.
                for fd in 0..3 {
                    let flags = libc::fcntl(fd, libc::F_GETFD);
                    if flags < 0 {
                        let error = io::Error::last_os_error();
                        if error.raw_os_error() == Some(libc::EBADF) {
                            continue;
                        }
                        return Err(error);
                    }
                    if flags & libc::FD_CLOEXEC != 0
                        && libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0
                    {
                        return Err(io::Error::last_os_error());
                    }
                }
                for &fd in &fds {
                    let flags = libc::fcntl(fd, libc::F_GETFD);
                    if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                        return Err(io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        let child = command.spawn();
        drop(transfer); // File closes only: inherited descriptions are never explicitly unlocked.
        drop(self); // Close-only, including the gate: never unlock the transferred set.
        child
    }
}
/// Give exactly this Unix child a reference to the profile lock's open description.
/// The command owns the CLOEXEC duplicate in the parent until its pre-exec hook is dropped.
#[cfg(unix)]
pub(crate) fn inherit_profile(command: &mut std::process::Command, file: &File) -> io::Result<()> {
    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::CommandExt,
    };
    // SAFETY: file is a live owned descriptor; duplicate it above the stdio slots.
    let fd = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fcntl returned a new owned descriptor.
    let file = unsafe { File::from_raw_fd(fd) };
    // SAFETY: only an async-signal-safe fcntl runs after fork; the closure owns the fd.
    unsafe {
        command.pre_exec(move || {
            // A pipe can occupy an otherwise closed stdio slot; preserve the configured
            // streams if Command's dup2 had no need to clear CLOEXEC.
            for fd in 0..3 {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags < 0 {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error() == Some(libc::EBADF) {
                        continue;
                    }
                    return Err(error);
                }
                if flags & libc::FD_CLOEXEC != 0
                    && libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0
                {
                    return Err(io::Error::last_os_error());
                }
            }
            let fd = file.as_raw_fd();
            if libc::fcntl(fd, libc::F_SETFD, 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(())
}

#[cfg(windows)]
pub(super) mod windows;
#[cfg(windows)]
pub(crate) use windows::CompilerJob;
#[cfg(windows)]
pub use windows::WindowsChild;

/// Create a Windows profile child with explicitly piped or attached streams. It inherits
/// only the selected lease and stream handles, not other launcher handles.
#[cfg(windows)]
pub(crate) fn spawn_profile(
    command: &std::process::Command,
    lease: &crate::browser::sunrise::txn::Lease,
    piped: bool,
) -> io::Result<super::child::Child> {
    spawn_managed(command, Some(lease), piped, false)
}

#[cfg(windows)]
pub(crate) fn spawn_compiler(
    command: &std::process::Command,
    lease: Option<&crate::browser::sunrise::txn::Lease>,
) -> io::Result<super::child::Child> {
    spawn_managed(command, lease, true, true)
}

#[cfg(windows)]
fn spawn_managed(
    command: &std::process::Command,
    lease: Option<&crate::browser::sunrise::txn::Lease>,
    piped: bool,
    compiler: bool,
) -> io::Result<super::child::Child> {
    use std::os::windows::io::AsRawHandle;
    let mut environment: Vec<_> = std::env::vars_os().collect();
    for (key, value) in command.get_envs() {
        environment.retain(|(existing, _)| {
            !existing
                .to_string_lossy()
                .eq_ignore_ascii_case(&key.to_string_lossy())
        });
        if let Some(value) = value {
            environment.push((key.to_owned(), value.to_owned()));
        }
    }
    let arguments: Vec<_> = command.get_args().map(std::ffi::OsStr::to_owned).collect();
    let pipes = if piped {
        let (input, parent_input) = io::pipe()?;
        let (parent_output, output) = io::pipe()?;
        let (parent_error, error) = io::pipe()?;
        Some((
            (input, output, error),
            (parent_input, parent_output, parent_error),
        ))
    } else {
        None
    };
    let streams = pipes.as_ref().map(|((input, output, error), _)| {
        [
            input.as_raw_handle(),
            output.as_raw_handle(),
            error.as_raw_handle(),
        ]
    });
    let files: Vec<_> = lease.iter().map(|lease| lease.file()).collect();
    let child = windows::spawn_exact(
        &files,
        Path::new(command.get_program()),
        &arguments,
        &environment,
        windows::Start {
            directory: command.get_current_dir(),
            streams,
            attached: !piped,
            compiler,
            ..windows::Start::default()
        },
    )?;
    Ok(super::child::Child::windows(
        child,
        pipes.map(|(_, parent)| parent),
    ))
}

#[cfg(test)]
mod tests;

fn error(error: Error) -> io::Error {
    match error {
        Error::Busy | Error::UnprovenExclusion | Error::IncompleteUnit => {
            io::ErrorKind::WouldBlock.into()
        }
        Error::UnsupportedFilesystem => io::ErrorKind::Unsupported.into(),
        Error::Unsafe(_) => io::ErrorKind::InvalidInput.into(),
        Error::Io(error) => error,
    }
}
