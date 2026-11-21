//! A named-pipe client receives only the claims explicitly duplicated into its exact process.
use crate::process::{
    child::Child,
    leases::{self, WrapperJob},
};
use std::{
    ffi::OsString,
    fs::File,
    io,
    os::windows::{
        ffi::OsStringExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::PathBuf,
    process::Command,
    ptr,
};
use sunrise_launcher_smod::local_control::{Channel, MAX_RELAY_CLAIMS};
use windows_sys::Win32::{
    Foundation::*,
    Storage::FileSystem::{GetFileType, FILE_TYPE_DISK},
    System::Threading::*,
};

pub(super) fn send_claims(
    channel: Channel,
    job: &WrapperJob,
    files: &[&File],
) -> io::Result<Channel> {
    let pid = channel.relay_client_pid()?;
    // SAFETY: the kernel reported the client PID for the authenticated pipe, never a numeric
    // PID supplied in a request. Pin the process object before checking its image and job.
    let handle = unsafe {
        OpenProcess(
            PROCESS_DUP_HANDLE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid,
        )
    };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    let mut transfer = Remote {
        process: unsafe { OwnedHandle::from_raw_handle(handle) },
        pending: Vec::new(),
    };
    if !job.contains(&transfer.process)? || channel.relay_client_pid()? != pid {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    let mut image = vec![0u16; 1 << 15];
    let mut length = image.len() as u32;
    // SAFETY: the process is pinned and both output pointers live across the call.
    if unsafe {
        QueryFullProcessImageNameW(
            transfer.process.as_raw_handle(),
            0,
            image.as_mut_ptr(),
            &mut length,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let image = PathBuf::from(OsString::from_wide(&image[..length as usize]));
    if image.canonicalize()? != std::env::current_exe()?.canonicalize()? {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    for file in files {
        let mut duplicate = ptr::null_mut();
        // SAFETY: owned source file and process handle; successful outputs live in that
        // process, initially non-inheritable and not yet adopted by the peer.
        if unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                file.as_raw_handle(),
                transfer.process.as_raw_handle(),
                &mut duplicate,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        transfer.pending.push(duplicate);
    }
    let handles: Vec<_> = transfer
        .pending
        .iter()
        .map(|handle| *handle as usize)
        .collect();
    let channel = channel.send(&handles)?;
    // Only receipt of the entire frame allows the peer to adopt or close the handles. Once
    // sent, never close by remote number again: the relay can close and reuse them at will.
    transfer.pending.clear();
    Ok(channel)
}

struct Remote {
    process: OwnedHandle,
    pending: Vec<HANDLE>,
}
impl Drop for Remote {
    fn drop(&mut self) {
        for handle in self.pending.drain(..) {
            // SAFETY: no complete offer was sent. Only the broker can retire these remote
            // handles; DUPLICATE_CLOSE_SOURCE closes in the pinned process even on failure.
            unsafe {
                DuplicateHandle(
                    self.process.as_raw_handle(),
                    handle,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    0,
                    0,
                    DUPLICATE_CLOSE_SOURCE,
                );
            }
        }
    }
}

pub(super) fn receive_claims(channel: Channel, count: usize) -> io::Result<(Channel, Vec<File>)> {
    let (channel, handles) = channel.receive::<Vec<usize>>()?;
    if count == 0 || count > MAX_RELAY_CLAIMS || handles.len() != count {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut owned = Vec::with_capacity(count);
    for number in handles {
        let handle = number as HANDLE;
        if handle.is_null() || handle == INVALID_HANDLE_VALUE || !seen.insert(number) {
            return Err(io::ErrorKind::InvalidData.into());
        }
        // SAFETY: the authenticated broker has just duplicated these exclusively owned file
        // handles into this process. Check the handle type before adopting the OS resource.
        if unsafe { GetFileType(handle) } != FILE_TYPE_DISK {
            return Err(io::ErrorKind::InvalidData.into());
        }
        owned.push(unsafe { File::from_raw_handle(handle) });
    }
    Ok((channel, owned))
}

pub(super) fn spawn(command: Command, claims: &[File]) -> io::Result<Child> {
    leases::spawn_windows_relay(&command, &claims.iter().collect::<Vec<_>>())
}
