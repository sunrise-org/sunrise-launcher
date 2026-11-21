//! What the launcher reads or runs of a version's own preparer (`sunrise-prepare`, docs/api/prepare-extras.md), without linking it.

use serde_json::Value;
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

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
            return Ok(());
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
        }
    }

    pub fn spawn_attached(&self, command: Command) -> io::Result<crate::process::child::Child> {
        self.handoff(None).spawn(command, false)
    }
}

impl Handoff<'_> {
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

/// Runs `command` to its end, each stdout line to `out`; stderr is read beside it (so no pipe fills) and its lines returned.
fn run(
    mut command: Command,
    out: &mut dyn FnMut(String),
    handoff: Option<&Handoff<'_>>,
) -> Result<(ExitStatus, Vec<String>), String> {
    use crate::process::owned::Owned;
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
    drop(child.child().stdin.take());
    let stderr = child.child().stderr.take().expect("piped");
    let errors = std::thread::spawn(move || tail(stderr));
    let mut read = 0;
    for text in lines(child.child().stdout.take().expect("piped")) {
        let text = text.map_err(unrun)?;
        read += text.len();
        // Past the cap the rest is drained unread, so the preparer never blocks on a full pipe.
        if read <= MAX_OUTPUT {
            out(text);
        }
    }
    let status = child.wait().map_err(unrun)?;
    Ok((status, errors.join().unwrap_or_default()))
}

pub(crate) fn run_staged(
    command: Command,
    handoff: Handoff<'_>,
    line: &dyn Fn(String),
) -> Result<(), String> {
    let (status, errors) = run(command, &mut |text| line(text), Some(&handoff))?;
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
        _ => Err(errors.join("\n").trim_start_matches("error: ").to_owned()),
    }
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
}
