//! The SDK's native build tool, `cargo sunrise-mod native-build`, run as a process with the launcher's toolchain (docs/modding/src/reference/sdk/08-native-build.md).
use super::workspace::Workspace;
use crate::rust_toolchain::Toolchain;
use crate::{
    i18n::{tr, trf},
    process::{
        leases::{ProfileHandoff, StagedChild},
        owned::Owned,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex, MutexGuard,
};
use std::time::{Duration, Instant};

/// The contract this launcher's plans speak.
pub(super) const CONTRACT: u32 = 2;

/// The tool's program in its unpacked SDK slice.
pub(super) const PROGRAM: &str = if cfg!(windows) {
    "cargo-sunrise-mod.exe"
} else {
    "cargo-sunrise-mod"
};

/// Cancellation shared by the owner and worker of one build attempt. It is never reset.
#[derive(Clone, Default)]
pub struct Cancellation {
    inner: Arc<Mutex<Attempt>>,
    flag: Arc<AtomicBool>,
}

/// Clears a tracked process PID when the watched child finishes or exits early.
pub(crate) struct ProcessGuard {
    cancellation: Cancellation,
    pid: u32,
}

impl Drop for ProcessGuard {
    fn drop(&mut self) {
        let mut state = self.cancellation.state();
        if state.tool == Some(self.pid) {
            state.tool = None;
        }
    }
}

#[derive(Default)]
struct Attempt {
    stopped: bool,
    publishing: bool,
    tool: Option<u32>,
    #[cfg(windows)]
    job: Option<Arc<crate::process::leases::CompilerJob>>,
}

impl Attempt {
    fn check(&self) -> Result<(), String> {
        if self.stopped {
            return Err(super::CANCELLED.into());
        }
        Ok(())
    }
}

impl Cancellation {
    fn state(&self) -> MutexGuard<'_, Attempt> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Atomic stop flag mirrored from [`Self::stop`], for routines taking `&AtomicBool`.
    pub(crate) fn flag(&self) -> &AtomicBool {
        &self.flag
    }

    /// Registers a spawned child process group so [`Self::stop`] terminates it. If already
    /// stopped, terminates the child immediately and returns the cancellation error.
    pub(crate) fn track_process(&self, pid: u32) -> Result<ProcessGuard, String> {
        let mut state = self.state();
        if state.stopped {
            drop(state);
            // It has done nothing yet: it is killed outright, with no grace to ignore.
            kill_process(pid);
            return Err(super::CANCELLED.into());
        }
        state.tool = Some(pid);
        Ok(ProcessGuard {
            cancellation: self.clone(),
            pid,
        })
    }

    pub(crate) fn check(&self) -> Result<(), String> {
        self.state().check()
    }

    /// Serialize the staged journal's cutoff with Stop. No worker cancellation follows it.
    pub(crate) fn begin_publication(&self) -> Result<(), String> {
        let mut state = self.state();
        state.check()?;
        state.publishing = true;
        Ok(())
    }
}

#[derive(Debug)]
struct CancelledRead;

impl std::fmt::Display for CancelledRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(super::CANCELLED)
    }
}

impl std::error::Error for CancelledRead {}

pub(crate) fn is_cancelled_io(error: &std::io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<CancelledRead>())
}

impl Cancellation {
    /// Check the attempt before each bounded download or extraction read.
    pub(crate) fn reader<R: std::io::Read>(&self, reader: R) -> impl std::io::Read + use<'_, R> {
        struct Reader<'a, R>(&'a Cancellation, R);
        impl<R: std::io::Read> std::io::Read for Reader<'_, R> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.0.is_stopped() {
                    return Err(std::io::Error::other(CancelledRead));
                }
                self.1.read(buf)
            }
        }
        Reader(self, reader)
    }

    pub fn is_stopped(&self) -> bool {
        self.state().stopped
    }

    /// Stops this attempt and the SDK tool it started, if any.
    pub fn stop(&self) {
        let mut state = self.state();
        if state.publishing {
            return;
        }
        state.stopped = true;
        self.flag.store(true, Ordering::Relaxed);
        #[cfg(windows)]
        if let Some(job) = &state.job {
            let _ = job.terminate();
            return;
        }
        if let Some(pid) = state.tool {
            terminate_process(pid);
            // One that ignores the request, or is stuck in the file system, is killed once
            // the grace is over, if it still runs for this attempt (review of #615).
            #[cfg(unix)]
            {
                let cancellation = self.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(STOP_GRACE);
                    if cancellation.state().tool == Some(pid) {
                        kill_process(pid);
                    }
                });
            }
        }
    }
}

/// How long a stopped child's process group gets to end on its own before it is killed.
pub(crate) const STOP_GRACE: Duration = Duration::from_secs(2);

/// Asks the child's process group to end: SIGTERM on Unix. On Windows the tree is ended at once
/// (`taskkill /T /F`), which is already the hard kill.
fn terminate_process(pid: u32) {
    // SAFETY: callers only pass PIDs of process groups they spawned for this attempt.
    #[cfg(unix)]
    unsafe {
        libc::kill(-(pid as libc::pid_t), libc::SIGTERM);
    }
    #[cfg(windows)]
    kill_process(pid);
}

/// Ends the child's process group outright: SIGKILL on Unix, `taskkill /T /F` on Windows.
fn kill_process(pid: u32) {
    // SAFETY: callers only pass PIDs of process groups they spawned for this attempt.
    #[cfg(unix)]
    unsafe {
        libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
    }
    #[cfg(windows)]
    let _ = crate::process::command(crate::process::taskkill())
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// The three slice folders a plan names.
pub(super) struct Package<'a> {
    pub runtime: &'a Path,
    pub api: &'a Path,
    pub engine: &'a Path,
}

/// What a build is for.
pub(super) struct Plan<'a> {
    pub work: &'a Path,
    pub workspace: &'a mut Workspace,
    pub package: Package<'a>,
    /// A folder of exactly the mods to compile.
    pub mods: &'a Path,
    pub server: bool,
    pub mixins_from_source: bool,
}

/// The mod a failed build stops at, and how.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Blame {
    #[serde(rename = "mod")]
    pub mod_id: String,
    pub kind: BlameKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlameKind {
    Compile,
    /// A build script, which the tool of a game build made before mods could run one refused.
    BuildScript,
    Features,
}

/// What a build made.
#[derive(Debug, Deserialize)]
pub(super) struct Built {
    pub route: CheckRoute,
    pub binary: PathBuf,
    pub libraries: Vec<PathBuf>,
    pub mods: Vec<String>,
    pub records: Vec<LibraryRecord>,
    pub internals: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct LibraryRecord {
    pub file: String,
    pub sha256: String,
    pub kind: LibraryKind,
    #[serde(rename = "mod")]
    pub mod_id: String,
    pub identity: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum LibraryKind {
    Mod,
    Patch,
}

#[derive(Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
enum Event {
    Mod {
        id: String,
        done: usize,
        total: usize,
    },
    /// The registry crates a build adds. A registry mod's whole release was reviewed, and a
    /// mod from source or a file was confirmed with its warning, so the answer is always yes.
    Consent {},
    Built {
        #[serde(flatten)]
        built: Built,
    },
    Failed {
        error: String,
        blame: Option<Blame>,
    },
}

/// Why a build failed: its error, and the mod it stops at when it names one.
#[derive(Debug)]
pub(super) struct Failure {
    pub error: String,
    pub blame: Option<Blame>,
}

impl From<String> for Failure {
    fn from(error: String) -> Self {
        Self { error, blame: None }
    }
}

/// The toolchain as a plan names it: the launcher's, as it set it up.
pub(super) fn toolchain(rust: &Toolchain) -> Value {
    serde_json::json!({
        "dir": rust.dir,
        "target": rust.host.target,
        "cargo_home": crate::rust_toolchain::cargo_home(),
        "cargo_config": rust.cargo_config(),
        "c_bin": rust.c_bin(),
    })
}

/// The plan's JSON line.
fn plan_line(rust: &Toolchain, plan: &Plan<'_>) -> String {
    let mut tools = toolchain(rust);
    tools["cargo_home"] = serde_json::json!(plan.workspace.path().join("cargo"));
    serde_json::json!({
        "contract": CONTRACT,
        "work": plan.work,
        "toolchain": tools,
        "package": {"runtime": plan.package.runtime, "api": plan.package.api, "engine": plan.package.engine},
        "mods": plan.mods,
        "target": if plan.server { "server" } else { "client" },
        "mixins_from_source": plan.mixins_from_source,
    })
    .to_string()
}

/// Compiles `plan` with the tool at `program`: each mod's start to `on_mod`.
pub(super) fn build(
    rust: &Toolchain,
    program: &Path,
    plan: Plan<'_>,
    on_mod: &dyn Fn(&str, usize, usize),
    cancellation: &Cancellation,
    profile: Option<ProfileHandoff<'_>>,
) -> Result<Built, Failure> {
    #[cfg(not(target_os = "linux"))]
    let mut command = command(rust, program);
    #[cfg(target_os = "linux")]
    let mut command = {
        let launcher = std::env::current_exe().map_err(unrun)?;
        // The lib test is not a launcher. The ignored integration fixture uses the real
        // launcher from the same Cargo build, not another implementation of the helper.
        #[cfg(test)]
        let launcher = launcher.with_file_name("").join("../sunrise-launcher");
        let mut command = command(rust, &launcher);
        command
            .arg(crate::process::compiler_worker::ARG)
            .arg(program);
        command
    };
    plan.workspace.isolate(&mut command).map_err(unrun)?;
    let line = plan_line(rust, &plan);
    run(
        command,
        &line,
        on_mod,
        cancellation,
        Some(plan.workspace),
        profile,
    )
}

/// A success or an attributed failure is useful only after the whole compiler tree stops: on
/// Linux its subreaper, on Windows its job object, on macOS its session (with the gap
/// [`crate::process::compiler_session`] names) proves it; the private directory is not read before.
fn compiler_handoff(terminal: Result<Built, Failure>, quiescent: bool) -> Result<Built, Failure> {
    match terminal {
        Err(failure) if quiescent || failure.blame.is_none() => Err(failure),
        Ok(built) if quiescent => Ok(built),
        _ => Err(tr("The launcher couldn't prove all native build workers stopped. Their build files were kept.")
            .to_owned()
            .into()),
    }
}

/// Runs the tool's `command` on the plan line `plan`.
fn run(
    mut command: std::process::Command,
    plan: &str,
    on_mod: &dyn Fn(&str, usize, usize),
    cancellation: &Cancellation,
    mut workspace: Option<&mut Workspace>,
    profile: Option<ProfileHandoff<'_>>,
) -> Result<Built, Failure> {
    command
        .arg("native-build")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    let (mut child, errors) = start(command, cancellation, true, profile)?;
    if let Some(workspace) = workspace.as_deref_mut() {
        workspace.allow_worker();
    }
    let result = talk(&mut child, plan, on_mod, cancellation);
    if result.is_err() && cancellation.state().tool.take().is_some() {
        // Don't wait on a tool which is still writing after a refused protocol message.
        let _ = child.child().kill();
    }
    let status = child.wait();
    #[cfg(target_os = "linux")]
    let quiescent = status
        .as_ref()
        .is_ok_and(crate::process::compiler_worker::proved);
    #[cfg(windows)]
    let quiescent = status.is_ok(); // WindowsChild::wait checked the compiler Job.
    #[cfg(target_os = "macos")]
    let quiescent = status.is_ok() && crate::process::compiler_session::ended(child.child().id());
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    let quiescent = false;
    if let Some(workspace) = workspace.as_deref_mut() {
        #[cfg(target_os = "linux")]
        let never_started = status
            .as_ref()
            .is_ok_and(crate::process::compiler_worker::no_worker);
        #[cfg(not(target_os = "linux"))]
        let never_started = false;
        if quiescent || never_started {
            workspace.worker_finished();
        }
    }
    if status.is_err() {
        let _ = child.child().kill();
    }
    let errors = finish(cancellation, errors);
    cancellation.check()?;
    let outcome = match (result, status) {
        (Ok(Some(Ok(built))), Ok(status)) if status.success() => Ok(built),
        (Ok(Some(Err(failure))), Ok(status)) if status.code() == Some(1) => Err(failure),
        (Ok(_), Ok(status)) => Err(ended(status.code()).into()),
        (Err(error), _) | (Ok(_), Err(error)) => Err(unrun(error).into()),
    };
    if outcome.is_err() {
        log_errors(&errors);
    }
    if let Some(workspace) = workspace {
        if quiescent {
            let log = workspace.import_log().map_err(unrun)?;
            if !log.is_empty() {
                super::append_log(&log)?;
            }
        }
        return compiler_handoff(outcome, quiescent);
    }
    outcome
}

/// `program` on `rust`, with the launcher's proxy setting and GitHub mirror in its environment.
pub(super) fn command(rust: &Toolchain, program: &Path) -> std::process::Command {
    let mut command = rust.command(program);
    crate::proxy::child_env(&mut command);
    command
}

/// The most of the tool's error output kept for the build log: its last lines.
const MAX_ERRORS: usize = 16 << 10;

/// The last lines of `stream`, read lossily, at most [`MAX_ERRORS`] bytes of them.
pub(super) fn tail(stream: impl std::io::Read) -> String {
    let mut kept = std::collections::VecDeque::new();
    let mut size = 0;
    for line in BufReader::new(stream).split(b'\n').map_while(Result::ok) {
        let line = String::from_utf8_lossy(&line).trim_end().to_owned();
        size += line.len() + 1;
        kept.push_back(line);
        while size > MAX_ERRORS {
            size -= kept.pop_front().map_or(0, |line| line.len() + 1);
        }
    }
    Vec::from(kept).join("\n")
}

/// Puts the end of what a failed tool wrote on stderr in the build log, where its failure message points.
pub(super) fn log_errors(errors: &str) {
    if !errors.is_empty() {
        let _ = super::append_log(&trf(
            "The native build tool's error output (its last lines):\n{output}",
            &[("output", errors)],
        ));
    }
}

/// Why the tool ended without its last line, from its exit code: 3 is a tool on another contract, 2 one that didn't take the request.
pub(super) fn ended(code: Option<i32>) -> String {
    match code {
        Some(3) => crate::i18n::tr(
            "This launcher is too old for this SDK's native build tool: update the launcher.",
        )
        .to_owned(),
        Some(2) => crate::i18n::tr(
            "The native build tool didn't accept the launcher's request: see the build log.",
        )
        .to_owned(),
        _ => trf(
            "The native build tool stopped ({status}) without saying why: see the build log.",
            &[(
                "status",
                &code.map_or_else(
                    || crate::i18n::tr("by a signal").to_owned(),
                    |c| c.to_string(),
                ),
            )],
        ),
    }
}

// Reading a pipe can outlive its SDK process: an escaped descendant can keep a writable
// handle even if it never writes. Keep draining independently; after exit a bounded grace
// lets the reader deliver already-buffered events without requiring an inherited EOF.
const EXIT_DRAIN: Duration = Duration::from_millis(750);
const EXIT_POLL: Duration = Duration::from_millis(20);

struct Output {
    lines: mpsc::Receiver<std::io::Result<String>>,
    deadline: Option<Instant>,
}

impl Output {
    fn new(stream: Box<dyn std::io::Read + Send>) -> Self {
        let (send, lines) = mpsc::sync_channel(16);
        std::thread::spawn(move || {
            for line in BufReader::new(stream).lines() {
                let failed = line.is_err();
                if send.send(line).is_err() || failed {
                    break;
                }
            }
        });
        Self {
            lines,
            deadline: None,
        }
    }

    fn line(
        &mut self,
        child: &mut crate::process::owned::Owned,
        cancellation: &Cancellation,
    ) -> std::io::Result<Option<String>> {
        loop {
            if self.deadline.is_none() {
                let mut state = cancellation.state();
                if child.try_wait()?.is_some() {
                    // Unix try_wait can reap the leader. Stop must never signal a reused PID.
                    state.tool = None;
                    self.deadline = Some(Instant::now() + EXIT_DRAIN);
                }
            }
            if let Err(stopped) = cancellation.check() {
                if self.deadline.is_none() {
                    let _ = child.child().kill();
                }
                return Err(std::io::Error::other(stopped));
            }
            if self.deadline.is_some_and(|at| Instant::now() >= at) {
                return Err(std::io::Error::other(tr(
                    "The native build tool stopped, but its output was still open.",
                )));
            }
            match self.lines.recv_timeout(EXIT_POLL) {
                Ok(line) => return line.map(Some),
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(None),
                Err(mpsc::RecvTimeoutError::Timeout) => (),
            }
        }
    }
}

/// Sends the plan, answers the crates question and reads events until the last.
fn talk(
    child: &mut crate::process::owned::Owned,
    plan: &str,
    on_mod: &dyn Fn(&str, usize, usize),
    cancellation: &Cancellation,
) -> std::io::Result<Option<Result<Built, Failure>>> {
    let mut input = child.child().stdin.take().expect("piped");
    writeln!(input, "{plan}")?;
    let mut output = Output::new(child.child().stdout.take().expect("piped"));
    let mut terminal = None;
    while let Some(line) = output.line(child, cancellation)? {
        if terminal.is_some() && !line.trim().is_empty() {
            return Err(std::io::Error::other(invalid_check_reply()));
        }
        let Ok(event) = serde_json::from_str::<Event>(&line) else {
            continue;
        };
        match event {
            Event::Mod { id, done, total } => on_mod(&id, done, total),
            Event::Consent {} => {
                cancellation.check().map_err(std::io::Error::other)?;
                writeln!(input, "true")?;
            }
            Event::Built { built } => terminal = Some(Ok(built)),
            Event::Failed { error, blame } => terminal = Some(Err(Failure { error, blame })),
        }
    }
    Ok(terminal)
}

/// Starts the tool in its own process group, tracked until it exits, with its stderr read into a capped tail.
fn start(
    mut command: std::process::Command,
    cancellation: &Cancellation,
    compiler: bool,
    profile: Option<ProfileHandoff<'_>>,
) -> Result<(crate::process::owned::Owned, mpsc::Receiver<String>), String> {
    command.stderr(Stdio::piped());
    // The compiler leads a session on macOS, which is also its process group: `run` proves
    // its workers stopped by that session.
    #[cfg(target_os = "macos")]
    if compiler {
        crate::process::compiler_session::lead(&mut command);
    } else {
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    // Stop either prevents the spawn or sees the PID. It cannot fall between them.
    let mut state = cancellation.state();
    state.check()?;
    let mut child = if let Some(profile) = profile {
        let role = if compiler {
            StagedChild::Compiler
        } else {
            StagedChild::Prepare
        };
        Owned::spawn_staged(command, profile, role)
    } else {
        #[cfg(windows)]
        if compiler {
            Owned::spawn_compiler(&command)
        } else {
            Owned::spawn(&mut command)
        }
        #[cfg(not(windows))]
        Owned::spawn(&mut command)
    }
    .map_err(unrun)?;
    state.tool = Some(child.child().id());
    #[cfg(windows)]
    {
        state.job = child.child().compiler_job();
    }
    drop(state);
    let stderr = child.child().stderr.take().expect("piped");
    let (send, errors) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = send.send(tail(stderr));
    });
    Ok((child, errors))
}

/// Forgets a tool that ended and returns the tail of its stderr.
fn finish(cancellation: &Cancellation, errors: mpsc::Receiver<String>) -> String {
    {
        let mut state = cancellation.state();
        state.tool = None;
        #[cfg(windows)]
        {
            state.job = None;
        }
    }
    errors.recv_timeout(EXIT_DRAIN).unwrap_or_default()
}

/// `native-check`'s reply: each mod's refusal, verdict or the file it needs.
#[derive(Debug, Deserialize)]
pub(super) struct Checked {
    pub contract: Option<u32>,
    /// Absent from a tool older than this launcher; `null` when no mod is usable.
    #[serde(default, deserialize_with = "present")]
    pub route: Option<Option<CheckRoute>>,
    pub mods: Vec<CheckedMod>,
}

/// A field that is there, `null` or not: [`Checked::route`]'s absence is told apart from its `null`.
fn present<'de, D: serde::Deserializer<'de>>(
    field: D,
) -> Result<Option<Option<CheckRoute>>, D::Error> {
    Option::<CheckRoute>::deserialize(field).map(Some)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum CheckRoute {
    PrebuiltApi,
    Prebuilt,
}

impl CheckRoute {
    pub fn name(self) -> &'static str {
        match self {
            Self::PrebuiltApi => "prebuilt-api",
            Self::Prebuilt => "prebuilt",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Need {
    Surface,
    BuildData,
}

#[derive(Debug, Deserialize)]
pub(super) struct CheckedMod {
    pub id: String,
    #[serde(default)]
    pub refusal: Option<String>,
    #[serde(default)]
    pub verdict: Option<super::fit::Verdict>,
    #[serde(default)]
    pub needs: Option<Need>,
}

/// Separate protocol refusals from local process or profile-handoff failures.
#[derive(Debug)]
pub(super) enum CheckFailure {
    Local(String),
    Refused(String),
}

/// Asks the tool at `program` how the mods `plan` names stand against its build (`native-check`).
pub(super) fn check(
    program: &Path,
    plan: &Value,
    cancellation: &Cancellation,
    profile: Option<ProfileHandoff<'_>>,
) -> Result<Checked, CheckFailure> {
    let value = ask_fenced(
        crate::process::command(program),
        "native-check",
        plan,
        cancellation,
        profile,
    )?;
    serde_json::from_value(value).map_err(|e| {
        CheckFailure::Refused(trf(
            "The native build tool's answer couldn't be read: {error}. Update the launcher.",
            &[("error", &e.to_string())],
        ))
    })
}

/// Runs one of the tool's one-line commands (`native-check`, `native-discover`, `native-lock`, `native-layout`) with `plan` (its `contract` filled in): its reply, or its error's message.
pub(super) fn ask(
    command: std::process::Command,
    name: &str,
    plan: &Value,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    ask_fenced(command, name, plan, cancellation, None).map_err(|error| match error {
        CheckFailure::Local(text) | CheckFailure::Refused(text) => text,
    })
}

fn ask_fenced(
    mut command: std::process::Command,
    name: &str,
    plan: &Value,
    cancellation: &Cancellation,
    profile: Option<ProfileHandoff<'_>>,
) -> Result<Value, CheckFailure> {
    let mut plan = plan.clone();
    plan["contract"] = CONTRACT.into();
    command
        .arg(name)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    let (mut child, errors) =
        start(command, cancellation, false, profile).map_err(CheckFailure::Local)?;
    let local = |error| CheckFailure::Local(unrun(error));
    let mut input = child.child().stdin.take().expect("piped");
    let sent = writeln!(input, "{plan}").map_err(local);
    drop(input);
    let mut output = Output::new(child.child().stdout.take().expect("piped"));
    let reply_line = (|| {
        let mut first = None;
        let mut extra = false;
        while let Some(line) = output.line(&mut child, cancellation).map_err(local)? {
            if !line.trim().is_empty() {
                if first.is_some() {
                    extra = true;
                } else {
                    first = Some(line);
                }
            }
        }
        if extra {
            return Err(CheckFailure::Refused(invalid_check_reply()));
        }
        Ok(first)
    })();
    let status = child.wait().map_err(local);
    let errors = finish(cancellation, errors);
    cancellation.check().map_err(CheckFailure::Local)?;
    sent?;
    let result = reply_line.and_then(|line| {
        let status = status?;
        match (line, status.code()) {
            (line, Some(0)) => reply(line.as_deref(), true),
            (line, Some(1)) => reply(line.as_deref(), false),
            (None, code) => Err(ended(code)),
            (Some(_), _) => Err(invalid_check_reply()),
        }
        .map_err(CheckFailure::Refused)
    });
    if result.is_err() {
        log_errors(&errors);
    }
    result
}

pub(super) fn invalid_check_reply() -> String {
    tr("The native build tool gave an incomplete or inconsistent answer. Update the launcher.")
        .into()
}

/// What a tool from before a command or reply field this launcher needs is told.
pub(super) fn too_old() -> String {
    crate::i18n::tr(
        "This build's native build tool is too old for this launcher: install a newer version.",
    )
    .to_owned()
}

/// A one-line command's reply: its value, its error's message, or (no line at all) a tool from before the command.
fn reply(line: Option<&str>, success: bool) -> Result<Value, String> {
    let value: Value = serde_json::from_str(line.ok_or_else(too_old)?).map_err(|_| too_old())?;
    let error = value.get("error");
    if !value.is_object() || (error.is_some() == success) {
        return Err(invalid_check_reply());
    }
    if let Some(error) = error {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(invalid_check_reply)?;
        return Err(trf(
            "The native build tool refused: {error}",
            &[("error", message)],
        ));
    }
    Ok(value)
}

fn unrun(error: std::io::Error) -> String {
    trf(
        "The native build tool couldn't run: {error}",
        &[("error", &error.to_string())],
    )
}

#[cfg(test)]
#[path = "tool_tests.rs"]
mod tests;
