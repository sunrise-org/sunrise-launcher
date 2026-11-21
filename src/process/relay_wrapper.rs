//! A wrapper's exit can revoke an unused relay token even while its descendants survive.
use super::child::Child;
use std::{
    io::{self, Read},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TrySendError},
        Arc,
    },
};

pub(super) struct Wrapper {
    pub child: Child,
    #[cfg(target_os = "linux")]
    leader: std::os::unix::net::UnixStream,
}

impl Wrapper {
    pub fn start(command: Command, piped: bool) -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        let (mut command, leader) = linux::worker_command(command)?;
        #[cfg(target_os = "macos")]
        let mut command = command;
        #[cfg(unix)]
        let mut wrapper = {
            use std::os::unix::process::CommandExt;
            if piped {
                command
                    .process_group(0)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped());
            }
            Self {
                child: command.spawn()?.into(),
                #[cfg(target_os = "linux")]
                leader,
            }
        };
        #[cfg(windows)]
        let mut wrapper = {
            // Let std handle the configured program's Windows PATH and batch quoting in a
            // private Job member. Only this launcher worker gets the restricted job spawn.
            let mut worker = Command::new(std::env::current_exe()?);
            worker
                .arg(ARG)
                .arg(command.get_program())
                .args(command.get_args());
            crate::launch::same_place(&command, &mut worker);
            Self {
                child: super::leases::spawn_windows_wrapper(&worker, piped)?,
            }
        };
        if let Err(error) = buffer_streams(&mut wrapper.child) {
            wrapper.retire();
            return Err(error);
        }
        Ok(wrapper)
    }

    pub fn leader_exited(&mut self) -> io::Result<bool> {
        #[cfg(target_os = "linux")]
        {
            use io::Read;
            // Only the launcher-owned subreaper can write or close this endpoint. Closing it
            // also covers failure to start the configured command and a dead subreaper.
            let mut status = [0];
            match self.leader.read(&mut status) {
                Ok(_) => Ok(true),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(false),
                Err(error) => Err(error),
            }
        }
        #[cfg(target_os = "macos")]
        {
            self.child.try_wait().map(|status| status.is_some())
        }
        #[cfg(windows)]
        {
            self.child.wrapper_leader_exited()
        }
    }

    pub fn retire(self) {
        let mut child = self.child;
        // This is cleanup, not authorization: the token has already been revoked or used.
        // Don't signal user wrapper PIDs or wait for their descendants on the Play worker.
        let _ = std::thread::Builder::new()
            .name("wrapper-reaper".into())
            .spawn(move || {
                let _ = child.wait();
            });
    }
}

// The normal game monitor starts after a successful handoff. Until then, a verbose
// wrapper must never block on a full pipe. Keep a bounded prefix, then apply normal
// log backpressure once the caller reads the stream.
fn buffer_streams(child: &mut Child) -> io::Result<()> {
    for stream in [&mut child.stdout, &mut child.stderr] {
        let Some(mut pipe) = stream.take() else {
            continue;
        };
        let (send, receive) = mpsc::sync_channel(8);
        let reading = Arc::new(AtomicBool::new(false));
        let attached = Arc::clone(&reading);
        std::thread::Builder::new()
            .name("wrapper-output".into())
            .spawn(move || loop {
                let mut bytes = vec![0; 8192];
                let chunk = match pipe.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(count) => {
                        bytes.truncate(count);
                        Ok(bytes)
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        let _ = send.try_send(Err(error));
                        break;
                    }
                };
                if attached.load(Ordering::Acquire) {
                    if send.send(chunk).is_err() {
                        break;
                    }
                } else if let Err(TrySendError::Disconnected(_)) = send.try_send(chunk) {
                    break;
                }
            })?;
        *stream = Some(Box::new(Output {
            reading,
            receive,
            chunk: io::Cursor::new(Vec::new()),
        }));
    }
    Ok(())
}

struct Output {
    reading: Arc<AtomicBool>,
    receive: Receiver<io::Result<Vec<u8>>>,
    chunk: io::Cursor<Vec<u8>>,
}
impl Read for Output {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        self.reading.store(true, Ordering::Release);
        let read = self.chunk.read(buf)?;
        if read > 0 {
            return Ok(read);
        }
        let Ok(chunk) = self.receive.recv() else {
            return Ok(0);
        };
        self.chunk = io::Cursor::new(chunk?);
        self.chunk.read(buf)
    }
}

#[cfg(any(target_os = "linux", windows))]
pub(super) const ARG: &str = "--internal-game-wrapper";

#[cfg(target_os = "linux")]
pub(super) use linux::run;

#[cfg(windows)]
pub(super) fn run(mut args: impl Iterator<Item = std::ffi::OsString>) -> io::Result<i32> {
    let program = args.next().ok_or(io::ErrorKind::InvalidInput)?;
    let status = Command::new(program).args(args).status()?;
    Ok(status.code().unwrap_or(1))
}

#[cfg(target_os = "linux")]
mod linux {
    use std::{
        ffi::OsString,
        io::{self, Write},
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::{net::UnixStream, process::CommandExt},
        },
        process::Command,
    };

    pub fn worker_command(original: Command) -> io::Result<(Command, UnixStream)> {
        let (reader, writer) = UnixStream::pair()?;
        reader.set_nonblocking(true)?;
        let fd = writer.as_raw_fd();
        let mut command = Command::new(std::env::current_exe()?);
        command
            .arg(super::ARG)
            .arg(fd.to_string())
            .arg(original.get_program())
            .args(original.get_args());
        crate::launch::same_place(&original, &mut command);
        // SAFETY: writer owns the FD through this one fork. Only the child clears CLOEXEC.
        unsafe {
            command.pre_exec(move || {
                if libc::fcntl(writer.as_raw_fd(), libc::F_SETFD, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Ok((command, reader))
    }

    pub fn run(mut args: impl Iterator<Item = OsString>) -> io::Result<i32> {
        let fd: i32 = args
            .next()
            .and_then(|arg| arg.to_str()?.parse().ok())
            .filter(|fd| *fd > 2)
            .ok_or(io::ErrorKind::InvalidInput)?;
        let executable = args.next().ok_or(io::ErrorKind::InvalidInput)?;
        // SAFETY: this is an early private process; validate the FD before adopting it. The
        // requested mode affects this process only. The user's wrapper never gets the FD.
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut notify = unsafe { std::fs::File::from_raw_fd(fd) };
        // SAFETY: no other children or threads have started. This is the launcher-owned helper.
        if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let status = Command::new(executable).args(args).status();
        let _ = notify.write_all(b"E");
        drop(notify);
        loop {
            // SAFETY: waits on owned/adopted children only. This is cleanup, not a lease proof.
            let who = unsafe { libc::waitpid(-1, std::ptr::null_mut(), 0) };
            if who >= 0 {
                continue;
            }
            let error = io::Error::last_os_error();
            match error.raw_os_error() {
                Some(libc::EINTR) => continue,
                Some(libc::ECHILD) => return super::super::supervisor::exit_code(status?),
                _ => return Err(error),
            }
        }
    }
}
