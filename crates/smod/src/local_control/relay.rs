//! The launcher-only relay protocol, separate from the game holder's stable control format.
use super::*;
use sha2::{Digest, Sha256};

/// Bound descriptor transfers independently of the frame limit for game args and paths.
pub const MAX_RELAY_CLAIMS: usize = 128;

#[cfg(windows)]
pub fn connect_relay(marker: &Marker, issuer: u32, deadline: Instant) -> io::Result<Stream> {
    use windows_sys::Win32::Storage::FileSystem::{SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT};
    let stream = platform::connect_with_flags(
        marker,
        deadline,
        SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
    )?;
    if stream.server_process_id()? != issuer {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(stream)
}

impl Channel {
    /// Prove possession in both directions before the client accepts incoming kernel claims.
    /// Never transmit the key. Nonces and role labels prevent echoing or replaying a proof.
    pub fn authenticate_relay(
        stream: Stream,
        marker: &Marker,
        server: bool,
        deadline: Instant,
        revoked: Option<Arc<AtomicBool>>,
    ) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        let mut channel = Self {
            stream,
            deadline,
            revoked,
        };
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).map_err(io::Error::other)?;
        let proof = |role, first: &[u8; 16], second: &[u8; 16]| {
            let mut digest = Sha256::new();
            digest.update(b"Sunrise managed Play relay\0");
            digest.update([role, VERSION]);
            digest.update(marker.key);
            digest.update(first);
            digest.update(second);
            let proof: [u8; 32] = digest.finalize().into();
            proof
        };
        if server {
            let mut first = [0; 16];
            channel.read_all(&mut first)?;
            channel.write_all(&nonce)?;
            channel.write_all(&proof(b'S', &first, &nonce))?;
            let mut reply = [0; 32];
            channel.read_all(&mut reply)?;
            if !bool::from(reply.ct_eq(&proof(b'C', &first, &nonce))) {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
        } else {
            channel.write_all(&nonce)?;
            let mut second = [0; 16];
            let mut reply = [0; 32];
            channel.read_all(&mut second)?;
            channel.read_all(&mut reply)?;
            if !bool::from(reply.ct_eq(&proof(b'S', &nonce, &second))) {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
            channel.write_all(&proof(b'C', &nonce, &second))?;
        }
        Ok(channel)
    }

    #[cfg(windows)]
    pub fn relay_client_pid(&self) -> io::Result<u32> {
        self.stream.client_process_id()
    }

    /// Commit is irreversible. A missing reply after this cut isn't revocation.
    pub fn relay_committed(mut self, deadline: Instant) -> Self {
        self.deadline = deadline;
        self.revoked = None;
        self
    }

    /// Read a piece of the post-spawn status or stop request without a launch deadline.
    /// The transport remains nonblocking; absence of a reply is not a game exit.
    pub fn poll_relay(&mut self, bytes: &mut [u8]) -> io::Result<Option<usize>> {
        loop {
            match self.stream.read(bytes) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(count) => return Ok(Some(count)),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
    }

    pub fn write_relay(&mut self, bytes: &[u8], deadline: Instant) -> io::Result<()> {
        self.deadline = deadline;
        self.write_all(bytes)
    }

    #[cfg(unix)]
    pub fn send_relay_claims(mut self, files: &[&std::fs::File]) -> io::Result<Self> {
        use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags, sendmsg};
        use std::{io::IoSlice, mem::MaybeUninit, os::fd::AsFd};
        if files.is_empty() || files.len() > MAX_RELAY_CLAIMS {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let files: Vec<_> = files.iter().map(AsFd::as_fd).collect();
        let mut raw = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(MAX_RELAY_CLAIMS))];
        let mut control = SendAncillaryBuffer::new(&mut raw);
        if !control.push(SendAncillaryMessage::ScmRights(&files)) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        #[cfg(target_vendor = "apple")]
        let flags = SendFlags::empty(); // Rust's process startup ignores SIGPIPE.
        #[cfg(not(target_vendor = "apple"))]
        let flags = SendFlags::NOSIGNAL;
        if self.retry_io(|stream| {
            sendmsg(stream, &[IoSlice::new(b"R")], &mut control, flags).map_err(Into::into)
        })? != 1
        {
            return Err(io::ErrorKind::WriteZero.into());
        }
        Ok(self)
    }

    #[cfg(unix)]
    pub fn receive_relay_claims(mut self, count: usize) -> io::Result<(Self, Vec<std::fs::File>)> {
        use rustix::{
            io::{FdFlags, fcntl_setfd},
            net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags, recvmsg},
        };
        use std::{io::IoSliceMut, mem::MaybeUninit};
        if count == 0 || count > MAX_RELAY_CLAIMS {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let mut raw = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(MAX_RELAY_CLAIMS))];
        let mut control = RecvAncillaryBuffer::new(&mut raw);
        let mut marker = [0];
        let mut truncated = false;
        #[cfg(target_os = "linux")]
        let flags = RecvFlags::CMSG_CLOEXEC;
        #[cfg(not(target_os = "linux"))]
        let flags = RecvFlags::empty();
        let received = self.retry_io(|stream| {
            let message = recvmsg(
                stream,
                &mut [IoSliceMut::new(&mut marker)],
                &mut control,
                flags,
            )?;
            truncated = message
                .flags
                .intersects(ReturnFlags::CTRUNC | ReturnFlags::TRUNC);
            Ok(message.bytes)
        })?;
        let mut files = Vec::with_capacity(count);
        let mut unknown = false;
        for message in control.drain() {
            match message {
                RecvAncillaryMessage::ScmRights(fds) => {
                    for fd in fds {
                        // The relay receives before any other threads can spawn a child.
                        fcntl_setfd(&fd, FdFlags::CLOEXEC)?;
                        files.push(std::fs::File::from(fd));
                    }
                }
                _ => unknown = true,
            }
        }
        if received != 1 || marker != *b"R" || truncated || unknown || files.len() != count {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok((self, files))
    }
}

#[cfg(windows)]
impl Marker {
    pub fn runtime() -> io::Result<Self> {
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).map_err(io::Error::other)?;
        Self::new(
            std::env::temp_dir().join(format!("sunrise-play-{:032x}", u128::from_le_bytes(nonce))),
        )
    }
}
