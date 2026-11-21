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

    /// Start the selected game with the committed profile and cache claims in one
    /// child creation. A configured terminal stays attached; a UI launch gets streams.
    pub fn spawn_game(
        self,
        command: std::process::Command,
        profile: ProfileHandoff<'_>,
        piped: bool,
    ) -> io::Result<super::child::Child> {
        #[cfg(unix)]
        {
            profile
                .spawn_unix(
                    profile_streams(command, piped),
                    &self.files(),
                    Some(&self.data),
                )
                .map(Into::into)
        }
        #[cfg(windows)]
        spawn_windows_command(
            &command,
            &self.files(),
            windows::Start {
                runtime_root: Some(&self.data),
                profile: Some(profile.file),
                attached: !piped,
                ..windows::Start::default()
            },
            piped,
        )
    }

    fn files(&self) -> Vec<&File> {
        self.claims.iter().flat_map(Claim::files).collect()
    }
    #[cfg(unix)]
    pub fn spawn(self, command: std::process::Command) -> io::Result<std::process::Child> {
        spawn_unix(command, &self.files(), Some(&self.data), None)
    }
}

/// Borrow the owning profile gate through exactly one spawn. Parent and child close their
/// references independently; neither releases the other's exclusion.
pub struct ProfileHandoff<'a> {
    pub(crate) file: &'a File,
    #[cfg(unix)]
    pub(crate) handed_off: &'a std::sync::atomic::AtomicBool,
}

/// Preparation only needs the profile gate. The compiler also needs the existing Windows
/// Job cutover (the Unix compiler uses its worker executable for process-tree proof).
#[derive(Clone, Copy)]
pub enum StagedChild {
    Prepare,
    Compiler,
}

impl ProfileHandoff<'_> {
    #[cfg(unix)]
    fn spawn_unix(
        self,
        command: std::process::Command,
        roots: &[&File],
        runtime_root: Option<&Path>,
    ) -> io::Result<std::process::Child> {
        let child = spawn_unix(command, roots, runtime_root, Some(self.file))?;
        self.handed_off
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(child)
    }

    pub(super) fn spawn(
        self,
        command: std::process::Command,
        role: StagedChild,
    ) -> io::Result<super::child::Child> {
        #[cfg(unix)]
        {
            let _ = role;
            self.spawn_unix(profile_streams(command, true), &[], None)
                .map(Into::into)
        }
        #[cfg(windows)]
        spawn_windows_command(
            &command,
            &[],
            windows::Start {
                profile: Some(self.file),
                compiler: matches!(role, StagedChild::Compiler),
                ..windows::Start::default()
            },
            true,
        )
    }
}

#[cfg(unix)]
fn profile_streams(mut command: std::process::Command, piped: bool) -> std::process::Command {
    if piped {
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
    }
    command
}

/// Only the chosen fork clears CLOEXEC. Consume the command, so stale pre_exec fds cannot
/// be reused after these owners drop. No allocation or lock is taken in the child callback.
#[cfg(unix)]
fn spawn_unix(
    mut command: std::process::Command,
    roots: &[&File],
    runtime_root: Option<&Path>,
    profile: Option<&File>,
) -> io::Result<std::process::Child> {
    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::CommandExt,
    };
    let transfer = roots
        .iter()
        .copied()
        .chain(profile)
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
    if let Some(data) = runtime_root {
        command.env("SUNRISE_RUNTIME_DATA_ROOT", data).env(
            "SUNRISE_RUNTIME_LEASE_FDS",
            fds[..roots.len()]
                .iter()
                .map(i32::to_string)
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    // SAFETY: only async-signal-safe fcntl calls run after fork. transfer owns every fd until
    // spawn returns, and the creator's copies keep their original CLOEXEC flags.
    unsafe {
        command.pre_exec(move || {
            // Command's fork path can allocate a pipe at a closed stdio slot. Its
            // dup2(fd, fd) leaves CLOEXEC set, so exec would discard that pipe.
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
                if libc::fcntl(fd, libc::F_SETFD, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    command.spawn()
}

#[cfg(windows)]
pub(super) mod windows;
#[cfg(windows)]
pub(crate) use windows::CompilerJob;
#[cfg(windows)]
pub use windows::WindowsChild;

/// Start a piped compiler with a Job-list attribute so it belongs to the Job at creation.
#[cfg(windows)]
pub(crate) fn spawn_compiler(command: &std::process::Command) -> io::Result<super::child::Child> {
    spawn_windows_command(
        command,
        &[],
        windows::Start {
            compiler: true,
            ..windows::Start::default()
        },
        true,
    )
}

#[cfg(windows)]
fn spawn_windows_command<'a>(
    command: &'a std::process::Command,
    files: &[&File],
    mut start: windows::Start<'a>,
    piped: bool,
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
    start.directory = command.get_current_dir();
    let program = Path::new(command.get_program());
    if !piped {
        return windows::spawn_exact(files, program, &arguments, &environment, start)
            .map(|child| super::child::Child::windows(child, None));
    }
    let (input, parent_input) = io::pipe()?;
    let (parent_output, output) = io::pipe()?;
    let (parent_error, error) = io::pipe()?;
    start.streams = Some([
        input.as_raw_handle(),
        output.as_raw_handle(),
        error.as_raw_handle(),
    ]);
    let child = windows::spawn_exact(files, program, &arguments, &environment, start)?;
    Ok(super::child::Child::windows(
        child,
        Some((parent_input, parent_output, parent_error)),
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
