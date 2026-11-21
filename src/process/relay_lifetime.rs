//! Completion and stop belong to the game created by the relay, not the outer wrapper.
use super::child::Child;
use std::{
    io,
    process::ExitStatus,
    time::{Duration, Instant},
};
use sunrise_launcher_smod::local_control::Channel;

const STOP: u8 = b'S';
const POLL: Duration = Duration::from_millis(50);
const WRITE: Duration = Duration::from_secs(1);

pub(super) struct RelayedGame {
    channel: Channel,
    pid: u32,
    raw: [u8; 4],
    received: usize,
    finished: Option<ExitStatus>,
}

impl RelayedGame {
    pub(super) fn new(channel: Channel, pid: u32) -> Self {
        Self {
            channel,
            pid,
            raw: [0; 4],
            received: 0,
            finished: None,
        }
    }

    pub(super) fn id(&self) -> u32 {
        self.pid
    }

    pub(super) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.finished.is_none() {
            if let Some(count) = self.channel.poll_relay(&mut self.raw[self.received..])? {
                self.received += count;
                if self.received == self.raw.len() {
                    self.finished = Some(decode(self.raw));
                }
            }
        }
        Ok(self.finished)
    }

    pub(super) fn wait(&mut self) -> io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            std::thread::sleep(POLL);
        }
    }

    pub(super) fn kill(&mut self) -> io::Result<()> {
        if self.try_wait()?.is_some() {
            return Ok(());
        }
        self.channel.write_relay(&[STOP], Instant::now() + WRITE)
    }
}

/// Run in the private relay, which alone can wait on and stop this owned process.
/// Losing the launcher only closes its control connection; the game keeps playing.
pub(super) fn observe(mut child: Child, mut channel: Channel) -> io::Result<ExitStatus> {
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        let mut request = [0];
        match channel.poll_relay(&mut request) {
            Ok(None) => {}
            Ok(Some(_)) if request == [STOP] => {
                // The child may have just exited. Its wait result is authoritative.
                let _ = child.kill();
            }
            Ok(Some(_)) | Err(_) => return child.wait(),
        }
        std::thread::sleep(POLL);
    };
    let _ = channel.write_relay(&encode(status), Instant::now() + WRITE);
    Ok(status)
}

#[cfg(unix)]
fn encode(status: ExitStatus) -> [u8; 4] {
    use std::os::unix::process::ExitStatusExt;
    status.into_raw().to_le_bytes()
}

#[cfg(windows)]
fn encode(status: ExitStatus) -> [u8; 4] {
    status
        .code()
        .expect("a Windows exit always has a code")
        .to_le_bytes()
}

#[cfg(unix)]
fn decode(raw: [u8; 4]) -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    ExitStatus::from_raw(i32::from_le_bytes(raw))
}

#[cfg(windows)]
fn decode(raw: [u8; 4]) -> ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    ExitStatus::from_raw(u32::from_le_bytes(raw))
}
