//! The SDK's native build tool, `cargo sunrise-mod native-build`, run as a process with the launcher's toolchain (docs/modding/src/reference/sdk/08-native-build.md).
use super::workspace::Workspace;
use crate::rust_toolchain::Toolchain;
use crate::{
    backend::{NativeGrant, PermissionFence},
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
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
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
pub struct Cancellation(Arc<Mutex<Attempt>>);

#[derive(Default)]
struct Attempt {
    stopped: bool,
    publishing: bool,
    tool: Option<u32>,
    #[cfg(windows)]
    job: Option<Arc<crate::process::leases::CompilerJob>>,
    grant: Option<NativeGrant>,
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
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn admit_native_work(&self) -> Result<(), super::Failed> {
        let mut state = self.state();
        state.check()?;
        if let Some(grant) = state.grant {
            drop(grant.hold()?);
        } else {
            state.grant = Some(NativeGrant::read()?.ok_or(super::Failed::NoConsent)?);
        }
        Ok(())
    }

    pub(crate) fn check(&self) -> Result<(), String> {
        self.state().check()
    }

    pub(crate) fn publication_guard(&self) -> Result<Option<PermissionFence>, String> {
        let state = self.state();
        state.check()?;
        state.grant.map(NativeGrant::hold).transpose()
    }

    /// Serialize the staged journal's cutoff with Stop. No worker cancellation follows it.
    pub(crate) fn begin_publication(&self) -> Result<Option<PermissionFence>, String> {
        let mut state = self.state();
        state.check()?;
        let permission = state.grant.map(NativeGrant::hold).transpose()?;
        state.publishing = true;
        Ok(permission)
    }

    /// Stops this attempt and the SDK tool it started, if any.
    pub fn stop(&self) {
        let mut state = self.state();
        if state.publishing {
            return;
        }
        state.stopped = true;
        #[cfg(windows)]
        if let Some(job) = &state.job {
            let _ = job.terminate();
            return;
        }
        if let Some(pid) = state.tool {
            // SAFETY: this process group was created for this attempt by `start`.
            #[cfg(unix)]
            unsafe {
                libc::kill(-(pid as libc::pid_t), libc::SIGTERM);
            }
            #[cfg(windows)]
            let _ = crate::process::command(crate::process::taskkill())
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
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

/// The registry crates a build would add, which the player says yes or no to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Consent {
    pub added_crates: usize,
    pub build_scripts: Vec<String>,
    pub proc_macros: Vec<String>,
}

impl Consent {
    pub fn label(&self) -> String {
        trf(
            "Native mod adds {count} registry crates",
            &[("count", &self.added_crates.to_string())],
        )
    }

    pub fn detail(&self) -> String {
        let none = tr("None");
        let scripts = self.build_scripts.join(", ");
        let macros = self.proc_macros.join(", ");
        [
            tr("Registry dependencies are native code. Their build scripts and proc macros run during compilation, before the game starts.").to_owned(),
            trf("Build scripts: {crates}", &[("crates", if self.build_scripts.is_empty() { none } else { &scripts })]),
            trf("Proc macros: {crates}", &[("crates", if self.proc_macros.is_empty() { none } else { &macros })]),
        ].join("\n\n")
    }
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
    Consent {
        added_crates: usize,
        build_scripts: Vec<String>,
        proc_macros: Vec<String>,
    },
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

/// Compiles `plan` with the tool at `program`: each mod's start to `on_mod`, a registry question to `consent` (none refuses).
pub(super) fn build(
    rust: &Toolchain,
    program: &Path,
    plan: Plan<'_>,
    consent: Option<&dyn Fn(&Consent) -> bool>,
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
        consent,
        on_mod,
        cancellation,
        Some(plan.workspace),
        profile,
    )
}

/// A success or an attributed failure is useful only after the whole compiler tree stops.
/// In particular, macOS only has leader exit today; the private directory must not be read.
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
    consent: Option<&dyn Fn(&Consent) -> bool>,
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
    let result = talk(&mut child, plan, consent, on_mod, cancellation);
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
    #[cfg(not(any(target_os = "linux", windows)))]
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

/// Sends the plan, answers consent questions and reads events until the last.
fn talk(
    child: &mut crate::process::owned::Owned,
    plan: &str,
    consent: Option<&dyn Fn(&Consent) -> bool>,
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
            Event::Consent {
                added_crates,
                build_scripts,
                proc_macros,
            } => {
                let question = Consent {
                    added_crates,
                    build_scripts,
                    proc_macros,
                };
                let yes = consent.is_some_and(|ask| ask(&question));
                // The answer can arrive after revoke and regrant. Bind the affirmative
                // write, not the dialog, to this attempt's original revision.
                let _permission = if yes {
                    cancellation
                        .publication_guard()
                        .map_err(std::io::Error::other)?
                } else {
                    None
                };
                writeln!(input, "{yes}")?;
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
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    // Stop either prevents the spawn or sees the PID. It cannot fall between them.
    let mut state = cancellation.state();
    state.check()?;
    // A grant check and the worker handoff form one short admission, never held during I/O.
    let permission = state.grant.map(NativeGrant::hold).transpose()?;
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
    drop(permission);
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

/// Asks the tool at `program` how the mods `plan` names stand against its build (`native-check`).
pub(super) fn check(
    program: &Path,
    plan: &Value,
    cancellation: &Cancellation,
    profile: Option<ProfileHandoff<'_>>,
) -> Result<Checked, String> {
    let value = ask_fenced(
        crate::process::command(program),
        "native-check",
        plan,
        cancellation,
        profile,
    )?;
    serde_json::from_value(value).map_err(|e| {
        trf(
            "The native build tool's answer couldn't be read: {error}. Update the launcher.",
            &[("error", &e.to_string())],
        )
    })
}

/// Runs one of the tool's one-line commands (`native-check`, `native-discover`, `native-lock`, `native-layout`) with `plan` (its `contract` filled in): its reply, or its error's message.
pub(super) fn ask(
    command: std::process::Command,
    name: &str,
    plan: &Value,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    ask_fenced(command, name, plan, cancellation, None)
}

fn ask_fenced(
    mut command: std::process::Command,
    name: &str,
    plan: &Value,
    cancellation: &Cancellation,
    profile: Option<ProfileHandoff<'_>>,
) -> Result<Value, String> {
    let mut plan = plan.clone();
    plan["contract"] = CONTRACT.into();
    command
        .arg(name)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    let (mut child, errors) = start(command, cancellation, false, profile)?;
    let mut input = child.child().stdin.take().expect("piped");
    let sent = writeln!(input, "{plan}").map_err(unrun);
    drop(input);
    let mut output = Output::new(child.child().stdout.take().expect("piped"));
    let reply_line = (|| -> Result<Option<String>, String> {
        let mut first = None;
        let mut extra = false;
        while let Some(line) = output.line(&mut child, cancellation).map_err(unrun)? {
            if !line.trim().is_empty() {
                if first.is_some() {
                    extra = true;
                } else {
                    first = Some(line);
                }
            }
        }
        if extra {
            return Err(invalid_check_reply());
        }
        Ok(first)
    })();
    let status = child.wait().map_err(unrun);
    let errors = finish(cancellation, errors);
    cancellation.check()?;
    sent?;
    let result = reply_line.and_then(|line| {
        let status = status?;
        match (line, status.code()) {
            (line, Some(0)) => reply(line.as_deref(), true),
            (line, Some(1)) => reply(line.as_deref(), false),
            (None, code) => Err(ended(code)),
            (Some(_), _) => Err(invalid_check_reply()),
        }
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
