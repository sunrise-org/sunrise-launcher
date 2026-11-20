//! The SDK's native build tool, `cargo sunrise-mod native-build`, run as a process with the launcher's toolchain (docs/modding/src/reference/sdk/08-native-build.md).
use crate::backend::{NativeGrant, Settings};
use crate::i18n::{tr, trf};
use crate::local_json::LockedJson;
use crate::rust_toolchain::Toolchain;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard};

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
pub struct Cancellation(
    Arc<Mutex<Attempt>>,
    Option<Arc<crate::browser::sunrise::txn::Lease>>,
);

#[derive(Default)]
struct Attempt {
    stopped: bool,
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
    pub fn for_profile(lease: Arc<crate::browser::sunrise::txn::Lease>) -> Self {
        Self(Arc::default(), Some(lease))
    }

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
            state.grant = Some(Settings::native_grant()?.ok_or(super::Failed::NoConsent)?);
        }
        Ok(())
    }

    pub(super) fn check(&self) -> Result<(), String> {
        let state = self.state();
        state.check()?;
        if let Some(grant) = state.grant {
            drop(grant.hold()?);
        }
        Ok(())
    }

    pub(crate) fn grant(&self) -> Result<Option<NativeGrant>, String> {
        let state = self.state();
        state.check()?;
        Ok(state.grant)
    }

    pub(crate) fn publication_guard(&self, image: &str) -> Result<Option<LockedJson>, String> {
        crate::native_status::publication_guard(image, self.grant()?)
    }

    /// Stops this attempt and the SDK tool it started, if any.
    pub fn stop(&self) {
        let mut state = self.state();
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
    pub workspace: &'a super::workspace::Workspace,
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
    pub records: Value,
    pub internals: BTreeMap<String, Value>,
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
    plan: &Plan<'_>,
    consent: Option<&dyn Fn(&Consent) -> bool>,
    on_mod: &dyn Fn(&str, usize, usize),
    cancellation: &Cancellation,
) -> Result<Built, Failure> {
    let mut command = command(rust, program);
    plan.workspace.isolate(&mut command).map_err(unrun)?;
    run(
        command,
        &plan_line(rust, plan),
        consent,
        on_mod,
        cancellation,
    )
}

/// Runs the tool's `command` on the plan line `plan`.
fn run(
    mut command: std::process::Command,
    plan: &str,
    consent: Option<&dyn Fn(&Consent) -> bool>,
    on_mod: &dyn Fn(&str, usize, usize),
    cancellation: &Cancellation,
) -> Result<Built, Failure> {
    command
        .arg("native-build")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    let (mut child, errors) = start(command, cancellation, true)?;
    let result = talk(&mut child, plan, consent, on_mod);
    let status = child.wait();
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

/// Sends the plan, answers consent questions and reads events until the last.
fn talk(
    child: &mut crate::process::owned::Owned,
    plan: &str,
    consent: Option<&dyn Fn(&Consent) -> bool>,
    on_mod: &dyn Fn(&str, usize, usize),
) -> std::io::Result<Option<Result<Built, Failure>>> {
    let mut input = child.child().stdin.take().expect("piped");
    writeln!(input, "{plan}")?;
    let output = child.child().stdout.take().expect("piped");
    for line in BufReader::new(output).lines() {
        let Ok(event) = serde_json::from_str::<Event>(&line?) else {
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
                writeln!(input, "{yes}")?;
            }
            Event::Built { built } => return Ok(Some(Ok(built))),
            Event::Failed { error, blame } => return Ok(Some(Err(Failure { error, blame }))),
        }
    }
    Ok(None)
}

/// Starts the tool in its own process group, tracked until it exits, with its stderr read into a capped tail.
fn start(
    mut command: std::process::Command,
    cancellation: &Cancellation,
    _compiler: bool,
) -> Result<
    (
        crate::process::owned::Owned,
        std::thread::JoinHandle<String>,
    ),
    String,
> {
    command.stderr(Stdio::piped());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    // Stop either prevents the spawn or sees the PID. It cannot fall between them.
    let mut state = cancellation.state();
    state.check()?;
    // The settings lock is held through the OS handoff, not while the SDK runs or asks.
    let permission = state.grant.map(NativeGrant::hold).transpose()?;
    let spawn = |command: &mut std::process::Command| match &cancellation.1 {
        Some(lease) => crate::process::owned::Owned::spawn_profile(command, lease),
        None => crate::process::owned::Owned::spawn(command),
    };
    #[cfg(unix)]
    let mut child = spawn(&mut command).map_err(unrun)?;
    #[cfg(windows)]
    let mut child = if _compiler {
        crate::process::owned::Owned::spawn_compiler(&command, cancellation.1.as_deref())
    } else {
        spawn(&mut command)
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
    Ok((child, std::thread::spawn(move || tail(stderr))))
}

/// Forgets a tool that ended and returns the tail of its stderr.
fn finish(cancellation: &Cancellation, errors: std::thread::JoinHandle<String>) -> String {
    {
        let mut state = cancellation.state();
        state.tool = None;
        #[cfg(windows)]
        {
            state.job = None;
        }
    }
    errors.join().unwrap_or_default()
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
) -> Result<Checked, String> {
    let value = ask(
        crate::process::command(program),
        "native-check",
        plan,
        cancellation,
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
    mut command: std::process::Command,
    name: &str,
    plan: &Value,
    cancellation: &Cancellation,
) -> Result<Value, String> {
    let mut plan = plan.clone();
    plan["contract"] = CONTRACT.into();
    command
        .arg(name)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    let (mut child, errors) = start(command, cancellation, false)?;
    let mut input = child.child().stdin.take().expect("piped");
    let sent = writeln!(input, "{plan}").map_err(unrun);
    drop(input);
    let output = child.child().stdout.take().expect("piped");
    let mut answer = Ok(None);
    for line in BufReader::new(output).lines() {
        match line {
            Ok(line) if line.trim().is_empty() => {}
            Ok(line) if matches!(answer, Ok(None)) => answer = Ok(Some(line)),
            Ok(_) if answer.is_ok() => answer = Err(invalid_check_reply()),
            Ok(_) => {}
            Err(error) => {
                answer = Err(unrun(error));
                break;
            }
        }
    }
    let status = child.wait().map_err(unrun);
    let errors = finish(cancellation, errors);
    cancellation.check()?;
    sent?;
    let status = status?;
    let result = answer.and_then(|line| match status.code() {
        code @ Some(2 | 3) => Err(ended(code)),
        _ => reply(line.as_deref()).and_then(|value| {
            if status.success() {
                Ok(value)
            } else {
                Err(ended(status.code()))
            }
        }),
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
fn reply(line: Option<&str>) -> Result<Value, String> {
    let value: Value = serde_json::from_str(line.ok_or_else(too_old)?).map_err(|_| too_old())?;
    if let Some(error) = value.get("error") {
        return Err(trf(
            "The native build tool refused: {error}",
            &[("error", error["message"].as_str().unwrap_or_default())],
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
