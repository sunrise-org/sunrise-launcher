//! What the launcher reads or runs of a version's own preparer (`sunrise-prepare`, sunrise-org/sunrise's docs/api/prepare-extras.md), without linking it.

use serde_json::Value;
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// The game's cache of prepared data in a resources folder (`sunrise_core::prepared_files::FILE_NAME`, checked by tests/test_launcher_copies.py).
pub const DATA_FILE: &str = "sunrise-data.bin";

/// The `format` of the preparer's JSON replies this launcher reads.
const FORMAT: u64 = 1;

fn home() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Python's `Path.expanduser` for the forms players type: `~` and `~/...`.
fn expand_user(path: &Path) -> PathBuf {
    let mut components = path.components();
    match components.next() {
        Some(Component::Normal(first)) if first == "~" => home().join(components.as_path()),
        _ => path.to_path_buf(),
    }
}

/// The official launcher's game directory, which holds the player's own copy of the game (as `sunrise-prepare` finds it).
pub fn minecraft_dir(chosen: Option<&Path>) -> PathBuf {
    if let Some(dir) = chosen.filter(|d| !d.as_os_str().is_empty()) {
        return expand_user(dir);
    }
    if let Some(dir) = std::env::var_os("SUNRISE_MINECRAFT_DIR").filter(|d| !d.is_empty()) {
        return expand_user(Path::new(&dir));
    }
    if cfg!(target_os = "macos") {
        home().join("Library/Application Support/minecraft")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(home)
            .join(".minecraft")
    } else {
        home().join(".minecraft")
    }
}

fn clone_file(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        let source = fs::File::open(from)?;
        let target = fs::File::create(to)?;
        // SAFETY: FICLONE takes a source fd, both descriptors stay open for the call.
        if unsafe { libc::ioctl(target.as_raw_fd(), libc::FICLONE, source.as_raw_fd()) } == 0 {
            // The clone shares the bytes, not the mode: `create` made the file 0644 (after umask),
            // so an executable would stop being one. Match `fs::copy`, the fallback.
            return target.set_permissions(source.metadata()?.permissions());
        }
        Err(io::Error::last_os_error())
    }
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let path = |p: &Path| {
            CString::new(p.as_os_str().as_bytes())
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
        };
        let (source, target) = (path(from)?, path(to)?);
        // SAFETY: both paths are valid NUL-terminated strings for the duration of the call.
        if unsafe { libc::clonefile(source.as_ptr(), target.as_ptr(), 0) } == 0 {
            return Ok(());
        }
        Err(io::Error::last_os_error())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (from, to);
        Err(io::ErrorKind::Unsupported.into())
    }
}

/// A copy-on-write clone where the file system has one, else a plain copy.
pub fn clone_or_copy(from: &Path, to: &Path) -> io::Result<()> {
    clone_file(from, to).or_else(|_| fs::copy(from, to).map(|_| ()))
}

/// The version folder's `sunrise-prepare`, when it ships one.
pub(crate) const BINARY: &str = if cfg!(windows) {
    "sunrise-prepare.exe"
} else {
    "sunrise-prepare"
};

pub fn binary(dir: &Path) -> Option<PathBuf> {
    let binary = dir.join(BINARY);
    binary.is_file().then_some(binary)
}

/// Why a version's preparer can't be used: it is from before the commands this launcher runs.
pub fn too_old(version: &str) -> String {
    crate::i18n::trf(
        "Sunrise {version} is too old for this launcher: install a newer version.",
        &[("version", version)],
    )
}

fn unrun(error: std::io::Error) -> String {
    crate::i18n::trf(
        "The game's preparer couldn't run: {error}",
        &[("error", &error.to_string())],
    )
}

/// Exclusive access to a version's shared preparation tree. On Unix the flock follows
/// the inherited file description; on Windows the no-share open follows the handle.
pub struct Preparation {
    file: fs::File,
    #[cfg(unix)]
    handed_off: std::sync::atomic::AtomicBool,
}

#[cfg(unix)]
impl Drop for Preparation {
    fn drop(&mut self) {
        // An unrelated fork can still have a descriptor before exec. Only a deliberate
        // preparer handoff needs that descriptor to keep the lock after we drop ours.
        if !*self.handed_off.get_mut() {
            let _ = self.file.unlock();
        }
    }
}

pub(crate) struct Handoff<'a> {
    preparation: &'a Preparation,
    profile: Option<crate::process::leases::ProfileHandoff<'a>>,
    stop: Option<&'a crate::native_mods::Cancellation>,
}

impl Preparation {
    pub(crate) fn new(file: fs::File) -> Self {
        Self {
            file,
            #[cfg(unix)]
            handed_off: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub(crate) fn handoff<'a>(
        &'a self,
        profile: Option<crate::process::leases::ProfileHandoff<'a>>,
    ) -> Handoff<'a> {
        Handoff {
            preparation: self,
            profile,
            stop: None,
        }
    }

    pub(crate) fn handoff_cancellable<'a>(
        &'a self,
        profile: Option<crate::process::leases::ProfileHandoff<'a>>,
        stop: &'a crate::native_mods::Cancellation,
    ) -> Handoff<'a> {
        Handoff {
            preparation: self,
            profile,
            stop: Some(stop),
        }
    }

    pub fn spawn_attached(&self, command: Command) -> io::Result<crate::process::child::Child> {
        self.handoff(None).spawn(command, false)
    }
}

impl Handoff<'_> {
    pub(crate) fn stop(&self) -> Option<&crate::native_mods::Cancellation> {
        self.stop
    }

    pub(crate) fn spawn(
        &self,
        command: Command,
        piped: bool,
    ) -> io::Result<crate::process::child::Child> {
        let child = crate::process::leases::spawn_preparer(
            command,
            &self.preparation.file,
            self.profile.as_ref(),
            piped,
        )?;
        #[cfg(unix)]
        self.preparation
            .handed_off
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(child)
    }
}

/// Runs `command` to its end, each stdout line to `out`; stderr is read beside it (so no pipe
/// fills) and its lines returned.
///
/// Nothing here waits without a deadline: one loop takes the output, sees whether the child
/// ended and checks the stop, each with a timed wait (review of #670). The child's process group
/// is its own, and the child is reaped only at the very end, so until then its zombie keeps its
/// PID, and the group's ID, from going to another process: a kill of the group can't reach
/// anything else ([`next`], [`finish`]). After a stop the group gets SIGTERM, then SIGKILL once
/// the grace is over or as soon as the child has ended, whichever is first, so nothing it left in
/// the group keeps the preparation lock. The output is read through a bounded queue, so a child
/// that floods it is held back by its pipe rather than grow the launcher's memory.
fn run(
    mut command: Command,
    out: &mut dyn FnMut(String),
    handoff: Option<&Handoff<'_>>,
) -> Result<(ExitStatus, Vec<String>), String> {
    use crate::process::owned::Owned;
    use std::sync::mpsc::{sync_channel, RecvTimeoutError, TryRecvError};
    let stop = handoff.and_then(Handoff::stop);
    if let Some(stop) = stop {
        stop.check()?;
    }
    let mut child = if let Some(handoff) = handoff {
        handoff.spawn(command, true).map(Owned::start_preparer)
    } else {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        Owned::spawn(&mut command)
    }
    .map_err(unrun)?;
    let pid = child.child().id();
    let tracked = match stop {
        Some(stop) => Some(stop.track_process(pid)?),
        None => None,
    };
    drop(child.child().stdin.take());
    let stderr = child.child().stderr.take().expect("piped");
    let (errors_tx, errors_rx) = sync_channel(1);
    std::thread::spawn(move || {
        let _ = errors_tx.send(tail(stderr));
    });
    let stdout = child.child().stdout.take().expect("piped");
    let (lines_tx, lines_rx) = sync_channel::<std::io::Result<String>>(QUEUE);
    std::thread::spawn(move || {
        let mut read = 0;
        for text in lines(stdout) {
            let text = match text {
                Ok(text) => text,
                Err(error) => {
                    let _ = lines_tx.send(Err(error));
                    break;
                }
            };
            read += text.len();
            // Past the cap the rest is drained unread, so the preparer never blocks on a full
            // pipe and nothing past it is queued.
            if read > MAX_OUTPUT {
                continue;
            }
            if lines_tx.send(Ok(text)).is_err() {
                break;
            }
        }
    });
    let stopped = || stop.is_some_and(crate::native_mods::Cancellation::is_stopped);
    let mut watch = Watch::default();
    let (mut stdout_open, mut errors, mut exited, mut failed) = (true, None, false, None);
    loop {
        if stdout_open {
            match lines_rx.recv_timeout(POLL) {
                Ok(Ok(text)) => out(text),
                Ok(Err(error)) => {
                    failed = Some(unrun(error));
                    stdout_open = false;
                }
                Err(RecvTimeoutError::Disconnected) => stdout_open = false,
                Err(RecvTimeoutError::Timeout) => {}
            }
        } else {
            std::thread::sleep(POLL);
        }
        if errors.is_none() {
            match errors_rx.try_recv() {
                Ok(lines) => errors = Some(lines),
                Err(TryRecvError::Disconnected) => errors = Some(Vec::new()),
                Err(TryRecvError::Empty) => {}
            }
        }
        if !exited {
            exited = ended_unreaped(&mut child).map_err(unrun)?;
        }
        let pipes_open = stdout_open || errors.is_none();
        match next(&mut watch, stopped(), exited, pipes_open, Instant::now()) {
            Next::Wait => {}
            Next::Kill => crate::native_mods::kill_group(pid),
            // Given up only once the stop's wait is over, or once the child had ended and its
            // group was killed: whatever still holds its output left the group.
            Next::Done | Next::GiveUp => break,
        }
    }
    let status = finish(move || drop(tracked), move || reap(child, exited));
    if stopped() {
        return Err(crate::native_mods::CANCELLED.into());
    }
    if let Some(failed) = failed {
        return Err(failed);
    }
    // Without a stop the loop ends only once the child has, so it was reaped.
    let status = status.ok_or_else(|| unrun(std::io::Error::other("it did not end")))?;
    Ok((status, errors.unwrap_or_default()))
}

/// How often a running child's stop is checked while its output is awaited.
const POLL: Duration = Duration::from_millis(100);
/// The longest a stopped child, or the output of one that ended, is waited for: the grace,
/// then a margin for the kill to land.
const STOPPED_WAIT: Duration =
    crate::native_mods::STOP_GRACE.saturating_add(Duration::from_secs(3));
/// The most stdout lines queued between the reader and [`run`]: a child that writes faster
/// than they are taken waits on its pipe. Each line is at most [`MAX_LINE`].
const QUEUE: usize = 64;

/// What [`run`] has seen of a child's end so far.
#[derive(Default)]
struct Watch {
    stopped_at: Option<Instant>,
    ended_at: Option<Instant>,
    killed: bool,
}

/// What [`run`] does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Next {
    Wait,
    /// SIGKILL to the child's group, which is safe while the child is not reaped.
    Kill,
    /// Reap the child and return.
    Done,
    /// Stop waiting: reap the child if it ended, and return.
    GiveUp,
}

/// The next step of [`run`], given whether a stop came, whether the child has ended (it is not
/// reaped yet) and whether its output is still open. A stopped child's group is killed before
/// the child is reaped, as soon as the child has ended or the grace is over. So a process it left
/// in the group, that ignores SIGTERM or holds the lock, is ended too.
fn next(watch: &mut Watch, stopped: bool, ended: bool, pipes_open: bool, now: Instant) -> Next {
    if stopped {
        let at = *watch.stopped_at.get_or_insert(now);
        if !watch.killed && (ended || now.duration_since(at) >= crate::native_mods::STOP_GRACE) {
            watch.killed = true;
            return Next::Kill;
        }
        if ended && !pipes_open {
            return Next::Done;
        }
        if now.duration_since(at) > STOPPED_WAIT {
            return Next::GiveUp;
        }
        return Next::Wait;
    }
    if ended {
        if !pipes_open {
            return Next::Done;
        }
        // Something it started still holds its output: its group is killed after the grace.
        let at = *watch.ended_at.get_or_insert(now);
        if !watch.killed && now.duration_since(at) >= crate::native_mods::STOP_GRACE {
            watch.killed = true;
            return Next::Kill;
        }
        if now.duration_since(at) > STOPPED_WAIT {
            return Next::GiveUp;
        }
    }
    Next::Wait
}

/// The end of a run, in the order that keeps a signal from reaching another process: the stop
/// forgets the child's PID first, then the child is reaped and its PID can go to another one.
fn finish<T>(untrack: impl FnOnce(), reap: impl FnOnce() -> T) -> T {
    untrack();
    reap()
}

/// Reaps a child that has ended. One that hasn't, after a stop and a kill, is left to a thread
/// to reap, so the run never waits on it.
fn reap(mut child: crate::process::owned::Owned, ended: bool) -> Option<ExitStatus> {
    if ended {
        return child.wait().ok();
    }
    std::thread::spawn(move || drop(child));
    None
}

/// Whether the child has ended, without reaping it: on Unix its zombie stays, keeping its PID.
#[cfg(unix)]
fn ended_unreaped(child: &mut crate::process::owned::Owned) -> std::io::Result<bool> {
    // SAFETY: a zeroed siginfo_t is valid for waitid to fill, and the PID is this run's child.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let pid = child.child().id() as libc::id_t;
    let flags = libc::WEXITED | libc::WNOWAIT | libc::WNOHANG;
    if unsafe { libc::waitid(libc::P_PID, pid, &mut info, flags) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: waitid filled `info`; si_pid is 0 when the child has not ended.
    Ok(unsafe { info.si_pid() } != 0)
}

/// Whether the child has ended. On Windows the child's handle keeps its PID from reuse until it
/// is dropped, and its job object ends the whole tree.
#[cfg(windows)]
fn ended_unreaped(child: &mut crate::process::owned::Owned) -> std::io::Result<bool> {
    Ok(child.child().try_wait()?.is_some())
}

pub(crate) fn run_staged(
    command: Command,
    handoff: &Handoff<'_>,
    line: &dyn Fn(String),
) -> Result<(), String> {
    let (status, errors) = run(command, &mut |text| line(text), Some(handoff))?;
    for error in errors {
        line(error);
    }
    if !status.success() {
        return Err(crate::i18n::trf(
            "The game's preparer stopped ({status}).",
            &[("status", &status.to_string())],
        ));
    }
    Ok(())
}

/// The most a preparer's standard output is read for, and the most of its error output kept (its last lines).
const MAX_OUTPUT: usize = 8 << 20;
const MAX_ERRORS: usize = 16 << 10;
/// The longest line kept whole; a longer one is cut.
const MAX_LINE: usize = 64 << 10;

/// `stream`'s lines, each read lossily (bytes that aren't UTF-8 become U+FFFD) and cut at [`MAX_LINE`].
fn lines(stream: impl std::io::Read) -> impl Iterator<Item = std::io::Result<String>> {
    BufReader::new(stream).split(b'\n').map(|line| {
        line.map(|mut bytes| {
            bytes.truncate(MAX_LINE);
            let text = String::from_utf8_lossy(&bytes);
            text.trim_end_matches('\r').to_owned()
        })
    })
}

/// The last lines of `stream`, at most [`MAX_ERRORS`] bytes of them.
fn tail(stream: impl std::io::Read) -> Vec<String> {
    let mut kept = std::collections::VecDeque::new();
    let mut size = 0;
    for line in lines(stream).map_while(Result::ok) {
        size += line.len();
        kept.push_back(line);
        while size > MAX_ERRORS {
            size -= kept.pop_front().map_or(0, |line| line.len());
        }
    }
    kept.into()
}

/// The JSON reply on the last stdout line of `command`, when the preparer has the subcommand (exit 2 means it doesn't); its log lines go to `line`.
fn reply(
    command: Command,
    line: &dyn Fn(String),
    handoff: Option<&Handoff<'_>>,
) -> Result<Option<Value>, String> {
    let mut last = None;
    let (status, errors) = run(
        command,
        &mut |text| {
            let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            match value["message"].as_str() {
                Some(message) => line(message.to_owned()),
                None if value.is_object() => last = Some(value),
                None => {}
            }
        },
        handoff,
    )?;
    if status.code() == Some(2) {
        return Ok(None);
    }
    for text in errors {
        line(text);
    }
    let value = last.ok_or_else(|| {
        crate::i18n::trf(
            "The game's preparer stopped ({status}) without saying why.",
            &[("status", &status.to_string())],
        )
    })?;
    if value["format"]
        .as_u64()
        .is_none_or(|format| format > FORMAT)
    {
        return Err(crate::i18n::tr(
            "This Sunrise version's preparer is newer than the launcher understands: update the launcher.",
        )
        .into());
    }
    if let Some(error) = value.get("error") {
        return Err(error["message"]
            .as_str()
            .unwrap_or(crate::i18n::tr(
                "The game's preparer failed without saying why.",
            ))
            .to_owned());
    }
    Ok(Some(value))
}

/// Copies the sounds into `root`'s resources (`sounds`), each line to `line`; None when the preparer has no such command.
pub(crate) fn sounds(
    binary: &Path,
    root: &Path,
    minecraft: &Path,
    line: &dyn Fn(String),
    handoff: Option<&Handoff<'_>>,
) -> Result<Option<()>, String> {
    let mut command = crate::process::command(binary);
    command
        .current_dir(root)
        .arg("sounds")
        .arg("--root")
        .arg(root)
        .arg("--minecraft-dir")
        .arg(minecraft);
    let (status, errors) = run(command, &mut |text| line(text), handoff)?;
    match status.code() {
        Some(0) => Ok(Some(())),
        Some(2) => Ok(None),
        _ => Err(errors
            .iter()
            .map(|error| {
                let error = error.trim_start_matches("error: ");
                missing_version_advice(error).unwrap_or_else(|| error.to_owned())
            })
            .collect::<Vec<_>>()
            .join("\n")),
    }
}

/// Older and current preparers print their own command-line flags in this diagnostic.
/// Keep the path the child actually tried, but replace its advice with the launcher's.
fn missing_version_advice(error: &str) -> Option<String> {
    let (path, _) = error
        .strip_prefix("Missing ")?
        .rsplit_once(". Sunrise does not download the game: ")?;
    let path = Path::new(path);
    let parent = path.parent()?;
    if path.extension()? != "json"
        || path.file_stem()? != parent.file_name()?
        || parent.parent()?.file_name()? != "versions"
    {
        return None;
    }
    Some(crate::sources::missing_version_file(path) + &crate::sources::prepare_advice())
}

fn json_command(binary: &Path, root: &Path, subcommand: &str) -> Command {
    let mut command = crate::process::command(binary);
    command
        .current_dir(root)
        .arg(subcommand)
        .arg("--root")
        .arg(root)
        .arg("--json");
    command
}

/// What preparing `root` needs beyond the player's Minecraft folder (`needs --json`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Needs {
    /// No verified import of the pinned jars is there to reuse (a sibling's is linked in when found).
    pub server_jar: bool,
    /// The version ships no matching exporters, so preparing compiles them with a JDK.
    pub jdk: bool,
}

/// [`Needs`] from the version's own preparer; `None` when it is too old to answer.
pub(crate) fn needs(
    binary: &Path,
    root: &Path,
    line: &dyn Fn(String),
    handoff: Option<&Handoff<'_>>,
) -> Result<Option<Needs>, String> {
    Ok(
        reply(json_command(binary, root, "needs"), line, handoff)?.map(|value| Needs {
            server_jar: value["server_jar"].as_bool().unwrap_or(true),
            jdk: value["jdk"].as_bool().unwrap_or(true),
        }),
    )
}

/// Each extra's kind and what it is, as the preparer names them.
pub type Extras = Vec<(String, String)>;

fn extras(value: &Value) -> Extras {
    value
        .as_array()
        .into_iter()
        .flatten()
        .map(|extra| {
            (
                extra["kind"].as_str().unwrap_or_default().to_owned(),
                extra["what"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

/// The extras the prepared `resources` of `root` lack (`missing-extras --json`); `None` when the preparer is too old to answer.
pub(crate) fn missing_extras(
    binary: &Path,
    root: &Path,
    resources: &Path,
    handoff: Option<&Handoff<'_>>,
) -> Result<Option<Extras>, String> {
    let mut command = json_command(binary, root, "missing-extras");
    command.arg("--output").arg(resources);
    Ok(reply(command, &|_| {}, handoff)?.map(|value| extras(&value["missing"])))
}

/// Writes the prepared `resources`' cache of prepared data (`data-file --json`): whether it wrote one; `None` when the preparer is too old to.
pub(crate) fn data_file(
    binary: &Path,
    root: &Path,
    resources: &Path,
    handoff: Option<&Handoff<'_>>,
) -> Result<Option<bool>, String> {
    let mut command = json_command(binary, root, "data-file");
    command.arg("--output").arg(resources);
    Ok(reply(command, &|_| {}, handoff)?.map(|value| value["written"].as_bool().unwrap_or(false)))
}

/// What `extras` did: the kinds added, each that failed with why, and what still lacks.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Added {
    pub added: Vec<String>,
    pub failed: Vec<(String, String)>,
    pub missing: Extras,
}

/// Adds the `only` extras `resources` lack, each from the first of `folders` that has it (`extras --json`); `None` when the preparer is too old.
pub(crate) fn add_extras(
    binary: &Path,
    root: &Path,
    resources: &Path,
    only: &[String],
    folders: &[PathBuf],
    line: &dyn Fn(String),
    handoff: Option<&Handoff<'_>>,
) -> Result<Option<Added>, String> {
    let mut command = json_command(binary, root, "extras");
    command
        .arg("--output")
        .arg(resources)
        .arg("--only")
        .arg(only.join(","));
    for folder in folders {
        command.arg("--minecraft-dir").arg(folder);
    }
    Ok(reply(command, line, handoff)?.map(|value| Added {
        added: value["added"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|kind| kind.as_str().map(str::to_owned))
            .collect(),
        failed: value["failed"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|failure| {
                (
                    failure["kind"].as_str().unwrap_or_default().to_owned(),
                    failure["error"].as_str().unwrap_or_default().to_owned(),
                )
            })
            .collect(),
        missing: extras(&value["missing"]),
    }))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A native build copies the SDK's `sunrise-client` with this, then runs it. Checked in the
    /// system temp folder (often tmpfs: the plain copy) and beside the test binary in Cargo's
    /// target directory (a reflink where the disk has them, such as btrfs and XFS).
    #[test]
    fn a_cloned_or_copied_executable_stays_executable() {
        use std::os::unix::fs::PermissionsExt;
        let exe = std::env::current_exe().unwrap();
        let target = exe.parent().unwrap();
        for dir in [
            tempfile::tempdir().unwrap(),
            tempfile::tempdir_in(target).unwrap(),
        ] {
            let from = dir.path().join("sunrise-client");
            fs::write(&from, b"#!/bin/sh\n").unwrap();
            fs::set_permissions(&from, fs::Permissions::from_mode(0o755)).unwrap();
            let to = dir.path().join("copy");
            clone_or_copy(&from, &to).unwrap();
            let mode = fs::metadata(&to).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755, "{}", dir.path().display());
        }
    }

    #[test]
    fn an_unhanded_preparation_unlocks_despite_an_incidental_descriptor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".preparing.lock");
        let file = fs::File::create(&path).unwrap();
        file.try_lock().unwrap();
        let incidental = file.try_clone().unwrap();
        let preparation = Preparation::new(file);
        let next = fs::OpenOptions::new().write(true).open(&path).unwrap();
        assert!(matches!(next.try_lock(), Err(fs::TryLockError::WouldBlock)));
        drop(preparation);
        next.try_lock().unwrap();
        drop(incidental);
    }

    #[test]
    fn a_handed_off_preparation_stays_locked_until_the_preparer_exits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".preparing.lock");
        let file = fs::File::create(&path).unwrap();
        file.try_lock().unwrap();
        let preparation = Preparation::new(file);
        let binary = fake(
            dir.path(),
            "printf 'ready\\n'\nif IFS= read -r line; then exit 1; fi",
        );
        let mut child = preparation
            .handoff(None)
            .spawn(Command::new(binary), true)
            .unwrap();
        drop(preparation);

        let mut ready = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut ready)
            .unwrap();
        let next = fs::OpenOptions::new().write(true).open(&path).unwrap();
        let while_running = next.try_lock();
        // wait closes the stdin pipe, letting the mock leave its blocking read and exit.
        assert!(child.wait().unwrap().success());
        assert_eq!(ready, "ready\n");
        assert!(matches!(while_running, Err(fs::TryLockError::WouldBlock)));
        next.try_lock().unwrap();
    }

    /// A preparer that runs `script`.
    fn fake(dir: &Path, script: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        fs::create_dir_all(dir).unwrap();
        let binary = dir.join("sunrise-prepare");
        fs::write(&binary, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        binary
    }

    #[test]
    #[ignore = "daily"]
    fn a_preparers_log_lines_reach_the_log_and_an_old_one_answers_nothing() {
        let dir =
            std::env::temp_dir().join(format!("sunrise-launcher-preparer-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let binary = fake(
            &dir,
            r#"case "$1" in
needs) echo 'linked the import of v1' >&2; printf '%s\n' '{"format": 1, "server_jar": false, "jdk": true}' ;;
sounds) echo 'copying 2 sounds'; echo 'error: no sound index' >&2; exit 1 ;;
*) exit 2 ;;
esac"#,
        );
        let lines = std::cell::RefCell::new(Vec::new());
        let log = |line: String| lines.borrow_mut().push(line);
        let answered = needs(&binary, &dir, &log, None).unwrap();
        assert_eq!(
            answered,
            Some(Needs {
                server_jar: false,
                jdk: true
            })
        );
        let error = sounds(&binary, &dir, &dir, &log, None).unwrap_err();
        assert_eq!(error, "no sound index");
        assert_eq!(
            *lines.borrow(),
            ["linked the import of v1", "copying 2 sounds"]
        );

        let old = fake(&dir.join("old"), "exit 2");
        assert_eq!(needs(&old, &dir, &log, None).unwrap(), None);
        assert_eq!(sounds(&old, &dir, &dir, &log, None).unwrap(), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn preparer_missing_version_file_has_launcher_instructions() {
        let error = missing_version_advice(
            "Missing /game with spaces/versions/26.3/26.3.json. Sunrise does not download the game: pass --minecraft-dir or set SUNRISE_MINECRAFT_DIR.",
        )
        .unwrap();
        assert!(error.starts_with(
            "Minecraft version file not found: /game with spaces/versions/26.3/26.3.json\n"
        ));
        assert!(error.contains("--minecraft PATH"));
        assert!(error.contains("Minecraft folder › Choose…"));
        assert!(error.contains(
            "sunrise-launcher account sign-in, then sunrise-launcher prepare --download"
        ));
        assert!(!error.contains("--minecraft-dir"));
        assert!(missing_version_advice(
            "Missing /game/versions/26.3/26.3.jar. Sunrise does not download the game: pass --minecraft-dir."
        ).is_none());
    }

    #[test]
    fn output_is_read_lossily_and_only_the_last_errors_are_kept() {
        let mixed = b"plain\n\xff\xfe broken\r\nlast".as_slice();
        let read: Vec<String> = lines(mixed).map(Result::unwrap).collect();
        assert_eq!(read, ["plain", "\u{fffd}\u{fffd} broken", "last"]);
        let long = format!("{}\n", "x".repeat(MAX_LINE + 10));
        assert_eq!(
            lines(long.as_bytes()).next().unwrap().unwrap().len(),
            MAX_LINE
        );
        let many: String = (0..2000)
            .map(|n| format!("line {n:04} {}\n", "y".repeat(40)))
            .collect();
        let kept = tail(many.as_bytes());
        assert_eq!(
            kept.last().map(String::as_str),
            Some(&*format!("line 1999 {}", "y".repeat(40)))
        );
        assert!(kept.iter().map(String::len).sum::<usize>() <= MAX_ERRORS);
    }

    #[test]
    fn cancelling_a_running_preparer_stops_its_process_group() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::{Duration, Instant};
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("child.pid");
        let binary = dir.path().join("sunrise-prepare");
        fs::write(
            &binary,
            format!(
                "#!/bin/sh\nsleep 30 &\necho $! > \"{}\"\necho '{{\"message\":\"started\"}}'\nwait\n",
                pid_file.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();

        let lock_file = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(dir.path().join(".prepare.lock"))
            .unwrap();
        lock_file.lock().unwrap();
        let preparation = Preparation::new(lock_file);
        let stop = crate::native_mods::Cancellation::default();
        let stop_bg = stop.clone();
        let started_tx = std::sync::mpsc::channel();
        let (tx, rx) = started_tx;
        let stopper = std::thread::spawn(move || {
            rx.recv_timeout(Duration::from_secs(5)).unwrap();
            stop_bg.stop();
        });
        let started = Instant::now();
        let handoff = preparation.handoff_cancellable(None, &stop);
        let result = needs(
            &binary,
            dir.path(),
            &|line| {
                if line == "started" {
                    let _ = tx.send(());
                }
            },
            Some(&handoff),
        );
        stopper.join().unwrap();
        assert_eq!(result.unwrap_err(), crate::native_mods::CANCELLED);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "cancellation waited for sleep instead of terminating the preparer"
        );
        let child_pid: libc::pid_t = fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            // SAFETY: signal 0 checks liveness without delivering a signal.
            let alive = unsafe { libc::kill(child_pid, 0) } == 0;
            if !alive {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("preparer child {child_pid} survived process-group cancellation");
    }

    /// A stopped child's group is killed before the child is reaped, as soon as the child has
    /// ended, so nothing it left in the group survives; and the run's last steps forget the PID
    /// before the reap that frees it (review of #670).
    #[test]
    fn a_stopped_childs_group_is_killed_before_it_is_reaped_and_untracked_before_the_reap() {
        let start = Instant::now();
        let grace = crate::native_mods::STOP_GRACE;
        // The child ended at once on SIGTERM and its output closed: the group is killed first.
        let mut watch = Watch::default();
        assert_eq!(next(&mut watch, true, true, false, start), Next::Kill);
        assert_eq!(next(&mut watch, true, true, false, start), Next::Done);
        // Still running after a stop: it gets the grace, then the kill, then a bounded wait.
        let mut watch = Watch::default();
        assert_eq!(next(&mut watch, true, false, true, start), Next::Wait);
        assert_eq!(
            next(&mut watch, true, false, true, start + grace),
            Next::Kill
        );
        assert_eq!(
            next(&mut watch, true, false, true, start + grace),
            Next::Wait
        );
        assert_eq!(
            next(&mut watch, true, false, true, start + STOPPED_WAIT + POLL),
            Next::GiveUp
        );
        // No stop: a child that ended with its output closed is done.
        let mut watch = Watch::default();
        assert_eq!(next(&mut watch, false, true, false, start), Next::Done);
        // Ended, but something it started holds its output: its group is killed after the grace.
        let mut watch = Watch::default();
        assert_eq!(next(&mut watch, false, true, true, start), Next::Wait);
        assert_eq!(
            next(&mut watch, false, true, true, start + grace),
            Next::Kill
        );
        // The stop's record of the PID goes before the reap.
        let order = std::cell::RefCell::new(Vec::new());
        finish(
            || order.borrow_mut().push("untrack"),
            || order.borrow_mut().push("reap"),
        );
        assert_eq!(order.into_inner(), ["untrack", "reap"]);
    }

    /// A child that floods its stdout while the caller is slow to take it is held back by its
    /// pipe: the launcher's memory doesn't grow with what the child writes (review of #670).
    #[cfg(target_os = "linux")]
    #[test]
    fn a_child_flooding_its_output_does_not_grow_the_launchers_memory() {
        const CHILD: &str = "SUNRISE_PREPARER_FLOOD_TEST";
        if std::env::var_os(CHILD).is_none() {
            let status = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "preparer::tests::a_child_flooding_its_output_does_not_grow_the_launchers_memory",
                ])
                .env(CHILD, "1")
                .status()
                .unwrap();
            assert!(
                status.success(),
                "the flood test failed in its child process"
            );
            return;
        }
        let peak = || {
            let status = fs::read_to_string("/proc/self/status").unwrap();
            let line = status
                .lines()
                .find(|l| l.starts_with("VmHWM:"))
                .unwrap()
                .to_owned();
            line.split_whitespace()
                .nth(1)
                .unwrap()
                .parse::<u64>()
                .unwrap()
                * 1024
        };
        let before = peak();
        // 200 MB of 1000-byte lines, as fast as awk writes them.
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "awk 'BEGIN { s = sprintf(\"%1000s\", \"\"); for (i = 0; i < 200000; i++) print s }'",
        ]);
        let mut first = true;
        let (status, _) = run(
            command,
            &mut |_| {
                // A slow caller: everything the child writes meanwhile would pile up unbounded.
                if std::mem::take(&mut first) {
                    std::thread::sleep(Duration::from_secs(3));
                }
            },
            None,
        )
        .unwrap();
        assert!(status.success());
        let grew = peak().saturating_sub(before);
        assert!(
            grew < 64 << 20,
            "the launcher's peak memory grew by {} MiB",
            grew >> 20
        );
    }
}
