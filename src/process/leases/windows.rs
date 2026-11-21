//! Stable Win32 creation with an explicit stdio and lifetime-handle allowlist.
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io, mem,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    ptr,
    sync::Arc,
};
use windows_sys::Win32::{
    Foundation::*,
    Globalization::CompareStringOrdinal,
    System::{Console::*, JobObjects::*, Threading::*},
};

/// An owned process object; neither waiting nor termination guesses ownership from a PID.
pub struct WindowsChild(OwnedHandle, u32, Option<Arc<CompilerJob>>);

/// An unnamed non-inheritable job. The launcher's death closes its last handle and kills the tree.
#[derive(Debug)]
pub(crate) struct CompilerJob(OwnedHandle);

impl CompilerJob {
    fn create() -> io::Result<Self> {
        // SAFETY: no name or inherited security descriptor; the caller owns the returned handle.
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: CreateJobObjectW returned an owned handle. Set a fixed-size, no-breakaway limit.
        let job = Self(unsafe { OwnedHandle::from_raw_handle(handle) });
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                job.0.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                mem::size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    pub(crate) fn terminate(&self) -> io::Result<()> {
        // SAFETY: the live job handle identifies exactly this attempt, not a recycled PID.
        if unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// The leader's owned handle has already signalled. The job accounting can still count
    /// it (or another exited child) while the kernel finishes bookkeeping. Observe exit on
    /// process handles, not on that counter. The total number ever assigned fences the PID
    /// snapshot against a listed child starting an unlisted one before it exits.
    fn check_finished(&self, leader: u32) -> io::Result<()> {
        let result = self.finished(leader);
        if result.is_err() {
            self.terminate()?;
        }
        result
    }

    pub(crate) fn accounting(&self) -> io::Result<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION> {
        let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        // SAFETY: a live handle and the fixed-size output buffer for this class.
        if unsafe {
            QueryInformationJobObject(
                self.0.as_raw_handle(),
                JobObjectBasicAccountingInformation,
                (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                mem::size_of_val(&info) as u32,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(info)
    }

    fn finished(&self, leader: u32) -> io::Result<()> {
        use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;

        // The documented variable-length JOBOBJECT_BASIC_PROCESS_ID_LIST layout.
        #[repr(C)]
        struct Members {
            assigned: u32,
            listed: u32,
            pids: [usize; 128],
        }
        let job = self.0.as_raw_handle();
        let before = self.accounting()?;
        // If the only process ever assigned was the leader, its waited handle is the
        // entire tree. No PID-list snapshot can add evidence about its exit.
        if before.ActiveProcesses == 0 || before.TotalProcesses == 1 {
            return Ok(());
        }
        let mut members = Members {
            assigned: 0,
            listed: 0,
            pids: [0; 128],
        };
        // SAFETY: Members is the documented C header followed by pointer-sized IDs.
        if unsafe {
            QueryInformationJobObject(
                job,
                JobObjectBasicProcessIdList,
                (&mut members as *mut Members).cast(),
                mem::size_of_val(&members) as u32,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let Some(pids) = members.pids.get(..members.listed as usize) else {
            return Err(io::ErrorKind::InvalidData.into());
        };
        if members.assigned != members.listed {
            return Err(io::Error::other(format!(
                "cannot prove the native build job stopped: {} assigned, {} listed, {} active of {} total",
                members.assigned, members.listed, before.ActiveProcesses, before.TotalProcesses
            )));
        }
        for &pid in pids {
            let pid = u32::try_from(pid).map_err(|_| io::ErrorKind::InvalidData)?;
            if pid == leader {
                continue; // The already-signalled SDK handle is still held, so its PID is exact.
            }
            // SAFETY: own the opened process handle, then prove it is in this job before waiting.
            let raw =
                unsafe { OpenProcess(SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
            if raw.is_null() {
                let error = io::Error::last_os_error();
                if pid != 0 && error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
                    continue; // It exited since the snapshot.
                }
                return Err(error);
            }
            let process = unsafe { OwnedHandle::from_raw_handle(raw) };
            let mut in_job = 0;
            if unsafe { IsProcessInJob(process.as_raw_handle(), job, &mut in_job) } == 0 {
                return Err(io::Error::last_os_error());
            }
            if in_job == 0 {
                continue; // That snapshot PID has since been reused outside our job.
            }
            match unsafe { WaitForSingleObject(process.as_raw_handle(), 0) } {
                WAIT_OBJECT_0 => {}
                WAIT_TIMEOUT => {
                    return Err(io::Error::other(format!(
                        "the native build tool left a running child ({pid})",
                    )))
                }
                _ => return Err(io::Error::last_os_error()),
            }
        }
        let after = self.accounting()?;
        if after.TotalProcesses != before.TotalProcesses {
            return Err(io::Error::other(format!(
                "cannot prove the native build job stopped: total members changed from {} to {}",
                before.TotalProcesses, after.TotalProcesses
            )));
        }
        Ok(())
    }
}
impl WindowsChild {
    pub(crate) fn id(&self) -> u32 {
        self.1
    }

    pub(crate) fn compiler_job(&self) -> Option<Arc<CompilerJob>> {
        self.2.clone()
    }

    pub(crate) fn kill(&self) -> io::Result<()> {
        if let Some(job) = &self.2 {
            return job.terminate();
        }
        // SAFETY: the process handle is live and identifies the process we created.
        if unsafe { TerminateProcess(self.0.as_raw_handle(), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(crate) fn try_wait(&self) -> io::Result<Option<std::process::ExitStatus>> {
        self.status(0)
    }

    /// Wait for the owned process, retaining its full Windows exit code.
    pub fn wait(&self) -> io::Result<std::process::ExitStatus> {
        let status = self.status(INFINITE)?.ok_or(io::ErrorKind::TimedOut)?;
        if let Some(job) = &self.2 {
            job.check_finished(self.1)?;
        }
        Ok(status)
    }

    fn status(&self, millis: u32) -> io::Result<Option<std::process::ExitStatus>> {
        use std::os::windows::process::ExitStatusExt;
        // SAFETY: this object owns a live process handle; out points to initialized storage.
        unsafe {
            match WaitForSingleObject(self.0.as_raw_handle(), millis) {
                WAIT_OBJECT_0 => {}
                WAIT_TIMEOUT => return Ok(None),
                _ => return Err(io::Error::last_os_error()),
            }
            let mut code = 0;
            if GetExitCodeProcess(self.0.as_raw_handle(), &mut code) == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Some(std::process::ExitStatus::from_raw(code)))
        }
    }
}
struct Attributes(Vec<usize>);
impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: constructed only after InitializeProcThreadAttributeList succeeds.
        unsafe {
            DeleteProcThreadAttributeList(self.0.as_mut_ptr().cast());
        }
    }
}

fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let value: Vec<_> = value.encode_wide().collect();
    if value.contains(&0) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    Ok(value)
}
fn argument(out: &mut Vec<u16>, value: &[u16]) {
    out.push(34);
    let mut slashes = 0;
    for &c in value {
        if c == 92 {
            slashes += 1;
            continue;
        }
        out.extend(std::iter::repeat_n(
            92,
            if c == 34 { slashes * 2 + 1 } else { slashes },
        ));
        out.push(c);
        slashes = 0;
    }
    out.extend(std::iter::repeat_n(92, slashes * 2));
    out.push(34);
}

pub(super) fn spawn(
    files: &[&File],
    data: &Path,
    program: &Path,
    args: &[OsString],
    environment: &[(OsString, OsString)],
    breakaway: bool,
) -> io::Result<WindowsChild> {
    spawn_exact(
        files,
        program,
        args,
        environment,
        Start {
            runtime_root: Some(data),
            breakaway,
            ..Start::default()
        },
    )
}

#[derive(Default)]
pub(super) struct Start<'a> {
    pub directory: Option<&'a Path>,
    pub streams: Option<[HANDLE; 3]>,
    pub runtime_root: Option<&'a Path>,
    pub profile: Option<&'a File>,
    pub breakaway: bool,
    pub attached: bool,
    pub compiler: bool,
}

pub(super) fn spawn_exact(
    files: &[&File],
    program: &Path,
    args: &[OsString],
    environment: &[(OsString, OsString)],
    start: Start<'_>,
) -> io::Result<WindowsChild> {
    let mut application = wide(program.as_os_str())?;
    let mut line = Vec::new();
    argument(&mut line, &application);
    application.push(0);
    let directory = start
        .directory
        .map(|dir| {
            let mut path = wide(dir.as_os_str())?;
            path.push(0);
            Ok::<_, io::Error>(path)
        })
        .transpose()?;
    for arg in args {
        line.push(32);
        argument(&mut line, &wide(arg)?);
    }
    line.push(0);
    let job = start
        .compiler
        .then(CompilerJob::create)
        .transpose()?
        .map(Arc::new);
    let nul = File::options().read(true).write(true).open("NUL")?;
    let mut roots: Vec<_> = files.iter().map(|file| file.as_raw_handle()).collect();
    roots.sort_unstable_by_key(|h| *h as usize);
    roots.dedup();
    let runtime_end = 3 + roots.len();
    // SAFETY: Win32 calls borrow valid stdio/file handles; duplicates are owned immediately.
    unsafe {
        let own = GetCurrentProcess();
        let stdio = start.streams.unwrap_or_else(|| {
            [
                GetStdHandle(STD_INPUT_HANDLE),
                GetStdHandle(STD_OUTPUT_HANDLE),
                GetStdHandle(STD_ERROR_HANDLE),
            ]
        });
        let mut inherited = Vec::new();
        for source in stdio
            .into_iter()
            .chain(roots)
            .chain(start.profile.map(AsRawHandle::as_raw_handle))
        {
            let source = if source.is_null() || source == INVALID_HANDLE_VALUE {
                nul.as_raw_handle()
            } else {
                source
            };
            let mut duplicate = ptr::null_mut();
            if DuplicateHandle(
                own,
                source,
                own,
                &mut duplicate,
                0,
                1,
                DUPLICATE_SAME_ACCESS,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            inherited.push(OwnedHandle::from_raw_handle(duplicate));
        }
        let handles: Vec<_> = inherited.iter().map(AsRawHandle::as_raw_handle).collect();
        let numbers = handles[3..runtime_end]
            .iter()
            .map(|h| (*h as usize).to_string())
            .collect::<Vec<_>>()
            .join(",");
        let mut env = Vec::new();
        for (key, value) in environment
            .iter()
            .map(|(k, v)| (k.as_os_str(), v.as_os_str()))
            .filter(|(key, _)| {
                start.runtime_root.is_none()
                    || !["SUNRISE_RUNTIME_LEASE_HANDLES", "SUNRISE_RUNTIME_DATA_ROOT"]
                        .iter()
                        .any(|k| key.to_string_lossy().eq_ignore_ascii_case(k))
            })
            .chain(start.runtime_root.into_iter().flat_map(|data| {
                [
                    (
                        OsStr::new("SUNRISE_RUNTIME_LEASE_HANDLES"),
                        OsStr::new(&numbers),
                    ),
                    (OsStr::new("SUNRISE_RUNTIME_DATA_ROOT"), data.as_os_str()),
                ]
            }))
        {
            let mut entry = wide(key)?;
            let start = usize::from(entry.first() == Some(&61)); // Preserve Windows =C: entries.
            if entry.len() == start || entry[start..].contains(&61) {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            let key_length = i32::try_from(entry.len()).map_err(|_| io::ErrorKind::InvalidInput)?;
            entry.push(61);
            entry.extend(wide(value)?);
            entry.push(0);
            env.push((entry, key_length));
        }
        env.sort_unstable_by(|(a, a_length), (b, b_length)| {
            CompareStringOrdinal(a.as_ptr(), *a_length, b.as_ptr(), *b_length, 1).cmp(&2)
        });
        let mut environment: Vec<u16> = env.into_iter().flat_map(|(entry, _)| entry).collect();
        environment.push(0);
        let mut bytes = 0;
        let attributes = 1 + u32::from(job.is_some());
        InitializeProcThreadAttributeList(ptr::null_mut(), attributes, 0, &mut bytes);
        let mut storage = vec![0usize; bytes.div_ceil(mem::size_of::<usize>())];
        let list = storage.as_mut_ptr().cast();
        if InitializeProcThreadAttributeList(list, attributes, 0, &mut bytes) == 0 {
            return Err(io::Error::last_os_error());
        }
        let _attributes = Attributes(storage);
        if UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            handles.as_ptr().cast(),
            mem::size_of_val(handles.as_slice()),
            ptr::null_mut(),
            ptr::null(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        // Put the SDK in its job atomically in CreateProcessW, before it can spawn a child.
        // This is independent of the narrow allowlist of inherited handles above.
        let jobs = job.as_ref().map(|job| [job.0.as_raw_handle()]);
        if let Some(jobs) = &jobs {
            if UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
                jobs.as_ptr().cast(),
                mem::size_of_val(jobs),
                ptr::null_mut(),
                ptr::null(),
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = mem::size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = handles[0];
        startup.StartupInfo.hStdOutput = handles[1];
        startup.StartupInfo.hStdError = handles[2];
        startup.lpAttributeList = list;
        let mut information = PROCESS_INFORMATION::default();
        let flags = EXTENDED_STARTUPINFO_PRESENT
            | CREATE_UNICODE_ENVIRONMENT
            | if start.compiler {
                // CREATE_NO_WINDOW still creates a console host. It joins the compiler's job
                // and may be shutting down after the SDK exits. The SDK already has real pipe
                // handles; run without any console rather than exempt an unsignalled job member.
                DETACHED_PROCESS
            } else if start.attached {
                0
            } else {
                CREATE_NO_WINDOW
            }
            | if start.breakaway {
                CREATE_BREAKAWAY_FROM_JOB
            } else {
                0
            };
        if CreateProcessW(
            // Profile commands may be a bare program name selected by the player. Let
            // CreateProcess resolve it as usual; the private runtime holder is always exact.
            if start.runtime_root.is_some() || program.is_absolute() {
                application.as_ptr()
            } else {
                ptr::null()
            },
            line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            flags,
            environment.as_ptr().cast(),
            directory.as_ref().map_or(ptr::null(), |path| path.as_ptr()),
            &startup.StartupInfo,
            &mut information,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let process = OwnedHandle::from_raw_handle(information.hProcess);
        drop(OwnedHandle::from_raw_handle(information.hThread));
        // inherited closes without unlock; the game's file objects retain sharing exclusion.
        Ok(WindowsChild(process, information.dwProcessId, job))
    }
}
