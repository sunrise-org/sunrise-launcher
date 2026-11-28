//! What the launcher reads or runs of a version's own preparer (`sunrise-prepare`, sunrise-org/sunrise's docs/api/prepare-extras.md), without linking it.

use serde_json::Value;
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus};
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
            Some(&self.preparation.file),
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
/// fills) and its last lines returned.
///
/// # A preparer child's life (review of #670)
///
/// Every preparer child runs through [`run_streams`]. The one other spawn is the CLI's
/// terminal-attached `prepare` ([`Preparation::spawn_attached`]): the terminal owns it, with no
/// Cancel, no pipes and no quit entry, and the CLI waits for it and returns its exit code.
///
/// 1. **Spawn** ([`crate::process::leases::spawn_preparer`], piped, with the lock or without):
///    on Unix it leads a new process group, on Windows it runs in a job of its own. Its quit
///    entry is made with it: the group, or the job
///    ([`crate::process::owned::Owned::start_preparer`]). From here a [`Held`] owns it, so a
///    return before the reap, a `?` included, kills its tree and leaves it to the reaper. A
///    spawn error returns before there is a child.
/// 2. **Track**: a stop learns its PID and job ([`crate::native_mods::Cancellation`]). A stop
///    that came first kills the tree at once, on Windows through the job alone.
/// 3. **Run**: one loop takes both streams through one bounded queue (stdout past [`MAX_OUTPUT`]
///    reaches a capped run's caller no more, [`Cap`]), sees whether the child
///    ended without reaping it (`waitid(WNOWAIT)`; on Windows its handle) and checks the stop,
///    each with a timed wait.
/// 4. **Stop and kill**: Cancel sends SIGTERM to the group (on Windows it ends the job). The run
///    kills the group as soon as the child ended or the grace is over ([`next`]). A read error
///    on either stream kills it at once. A child that ended while something still holds its
///    output has its group killed after the grace, and the run is an error.
/// 5. **Untrack**: the stop forgets the PID under its lock; a stop that came just before has the
///    group killed now ([`finish`]).
/// 6. **Reap**: an ended child is reaped here. One that hasn't ended goes to the reaper thread,
///    which reaps each child once it ended and never waits on one. A run whose reaper can't start
///    fails before its spawn; if the reaper has gone, the run waits a bounded time, then leaves the
///    child unreaped ([`abandon`]). Before every reap its quit entry is
///    removed, under the lock a quit signals under ([`crate::process::owned::Owned`]).
/// 7. **Forget**: once reaped, nothing looks at or signals its PID again;
///    [`crate::process::owned::Owned`] keeps its status.
///
/// Until the reap the PID, and the group's ID, are the child's: its zombie keeps them on Unix,
/// its handle on Windows. Until then the run, the stop and a quit may signal the group or end
/// the job; after it, nothing may. A quit sends SIGTERM to each group still tracked and ends each
/// job. An abrupt exit of the launcher ends nothing: the child finishes under the locks it holds,
/// as before #670. As before #670 too, what a child that ended on its own left running with its
/// output closed runs on: only a stop, an error or a quit before the reap ends the group or job.
fn run(
    command: Command,
    out: &mut dyn FnMut(String),
    handoff: Option<&Handoff<'_>>,
) -> Result<(ExitStatus, Vec<String>), String> {
    run_with(command, out, handoff, handoff.and_then(Handoff::stop))
}

/// [`run`], stopped by `stop`, which is the handoff's own outside tests.
fn run_with(
    command: Command,
    out: &mut dyn FnMut(String),
    handoff: Option<&Handoff<'_>>,
    stop: Option<&crate::native_mods::Cancellation>,
) -> Result<(ExitStatus, Vec<String>), String> {
    let mut stdout = |stream: Stream, text: String| {
        if stream == Stream::Out {
            out(text);
        }
    };
    run_streams(command, &mut stdout, handoff, stop, Cap::Output)
        .map(|ran| (ran.status, ran.errors))
}

/// Which of a child's streams a line came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stream {
    Out,
    Err,
}

/// How much of a child's stdout reaches the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cap {
    /// The first [`MAX_OUTPUT`] bytes, as the subcommands with a JSON reply had on the release
    /// base; the rest is read, but only for the run's last line.
    Output,
    /// Every line, as the backend's monitor forwarded a preparation's on the release base: the
    /// caller keeps its own bounded ring (review of #670).
    Off,
}

/// What a run's readers hand its loop, through one bounded queue.
enum Read {
    Line(Stream, String),
    /// A stdout line past [`MAX_OUTPUT`] of a capped run: not for the caller, but it may be the
    /// last one.
    Skipped(String),
    Failed(std::io::Error),
}

/// [`run_with`], with both streams' lines to `out` as they come, each through the same bounded
/// queue. The last lines of stderr are also returned.
fn run_streams(
    command: Command,
    out: &mut dyn FnMut(Stream, String),
    handoff: Option<&Handoff<'_>>,
    stop: Option<&crate::native_mods::Cancellation>,
    cap: Cap,
) -> Result<Ran, String> {
    use crate::process::owned::Owned;
    use std::sync::mpsc::RecvTimeoutError;
    if let Some(stop) = stop {
        stop.check()?;
    }
    // The reaper starts here, before the child: no destructor of a run starts a thread, and a
    // run whose reaper can't start starts no child (review of #670).
    reaper::start().map_err(unrun)?;
    let spawned = match handoff {
        Some(handoff) => handoff.spawn(command, true),
        // A preparer without the preparation's lock, such as a probe, starts the same way.
        None => crate::process::leases::spawn_preparer(command, None, None, true),
    }
    .map(Owned::start_preparer)
    .map_err(unrun)?;
    // From here the child is the guard's: whatever way this returns, a `?` included, a child not
    // reaped here is killed and left to the reaper thread (review of #670).
    let mut child = Held::new(spawned);
    #[cfg(test)]
    test_hooks::hook(&test_hooks::BEFORE_TRACK);
    let tracked = match stop {
        Some(stop) => {
            match stop.track_preparer(
                child.pid,
                #[cfg(windows)]
                child.get().child().preparer_job(),
            ) {
                Ok(guard) => Some(guard),
                Err(cancelled) => {
                    // The stop came between the spawn and here, and has ended the child's tree:
                    // the guard only leaves it to the reaper.
                    child.killed = true;
                    return Err(cancelled);
                }
            }
        }
        None => None,
    };
    let queue = read_output(child.get(), cap).map_err(unrun)?;
    let stopped = || stop.is_some_and(crate::native_mods::Cancellation::is_stopped);
    let mut watch = Watch::default();
    let mut errors = Tail::default();
    // The last line the child wrote, on either stream, including stdout past the cap.
    let mut last = None;
    let (mut open, mut exited, mut failed) = (true, false, None);
    // The time spent in `out`: the deadlines run on a clock without it, so they judge the child
    // and its pipes, never how fast the caller takes its lines. A slow caller can't drop the
    // final reply queued behind the lines before it (review of #670).
    let mut in_out = Duration::ZERO;
    loop {
        let ending = stopped() || failed.is_some();
        if open {
            match queue.recv_timeout(POLL) {
                Ok(Read::Skipped(text)) => note_last(&mut last, &text),
                Ok(Read::Line(stream, text)) => {
                    note_last(&mut last, &text);
                    if stream == Stream::Err {
                        errors.keep(text.clone());
                    }
                    // Once the run is ending its lines are only drained: they go nowhere.
                    if !ending {
                        let took = Instant::now();
                        out(stream, text);
                        in_out += took.elapsed();
                    }
                }
                Ok(Read::Failed(error)) => {
                    // A read error ends the run at once, as the base's return did: the child's
                    // tree is killed now, and every wait from here has the stop's bound.
                    failed.get_or_insert(unrun(error));
                    watch.killed = true;
                    child.kill();
                }
                // Both streams ended.
                Err(RecvTimeoutError::Disconnected) => open = false,
                Err(RecvTimeoutError::Timeout) => {}
            }
        } else {
            std::thread::sleep(POLL);
        }
        if !exited {
            exited = probe(child.get()).map_err(unrun)?;
        }
        let ending = stopped() || failed.is_some();
        let now = Instant::now()
            .checked_sub(in_out)
            .unwrap_or_else(Instant::now);
        match next(&mut watch, ending, exited, open, now) {
            Next::Wait => {}
            Next::Kill => child.kill(),
            // Given up only once the stop's wait is over, or once the child had ended and its
            // group was killed: whatever still holds its output left the group.
            Next::Done | Next::GiveUp => break,
        }
    }
    // Without a stop or a failure, a kill means the child ended while something it started
    // still held its output: that was killed, and what it would have written was not read.
    let cut_off = watch.killed;
    #[cfg(test)]
    test_hooks::hook(&test_hooks::BEFORE_UNTRACK);
    let status = finish(Ending {
        child,
        tracked,
        ended: exited,
    });
    if stopped() {
        return Err(crate::native_mods::CANCELLED.into());
    }
    if let Some(failed) = failed {
        return Err(failed);
    }
    // Never a clean success with output cut off (review of #670).
    if cut_off {
        return Err(crate::i18n::tr(
            "The game's preparer ended, but a process it started kept its output open, so the launcher stopped that process. Some of the output was not read.",
        )
        .into());
    }
    // Without a stop the loop ends only once the child has, so it was reaped.
    let status = status.ok_or_else(|| unrun(std::io::Error::other("it did not end")))?;
    Ok(Ran {
        status,
        errors: errors.into(),
        last,
    })
}

/// How a run's child ended, with the last lines of its stderr and the last line it wrote.
struct Ran {
    status: ExitStatus,
    errors: Vec<String>,
    last: Option<String>,
}

/// Keeps `text` as the last line, unless it is blank.
fn note_last(last: &mut Option<String>, text: &str) {
    if !text.trim().is_empty() {
        *last = Some(text.to_owned());
    }
}

/// Starts the threads that read `child`'s output, both streams' lines into one queue of at
/// most [`QUEUE`]: a child that writes faster than its lines are taken waits on its pipe.
fn read_output(
    child: &mut crate::process::owned::Owned,
    cap: Cap,
) -> std::io::Result<std::sync::mpsc::Receiver<Read>> {
    drop(child.child().stdin.take());
    let (lines_tx, queue) = std::sync::mpsc::sync_channel::<Read>(QUEUE);
    let stderr = child.child().stderr.take().expect("piped");
    let errors_tx = lines_tx.clone();
    reader(move || pump_errors(stderr, &errors_tx))?;
    let stdout = child.child().stdout.take().expect("piped");
    #[cfg(test)]
    let mut fails_after = test_hooks::READ_FAILS_AFTER.with(std::cell::Cell::take);
    reader(move || {
        let mut read = 0;
        for text in lines(stdout) {
            #[cfg(test)]
            let text = match fails_after {
                Some(0) => Err(std::io::Error::other("a test's read error")),
                Some(left) => {
                    fails_after = Some(left - 1);
                    text
                }
                None => text,
            };
            let text = match text {
                Ok(text) => text,
                Err(error) => {
                    let _ = lines_tx.send(Read::Failed(error));
                    break;
                }
            };
            read += text.len();
            // Past the cap the rest is still read, so the preparer never blocks on a full pipe,
            // but goes only to the run's last line, never to the caller.
            let line = if cap == Cap::Output && read > MAX_OUTPUT {
                Read::Skipped(text)
            } else {
                Read::Line(Stream::Out, text)
            };
            if lines_tx.send(line).is_err() {
                break;
            }
        }
    })?;
    Ok(queue)
}

/// Hands on `stream`'s lines as stderr's. A read error ends it as one on stdout does: the run
/// gets [`Read::Failed`] and starts its bounded end, rather than taking the error for the end of
/// the stream (follow-up to #670).
fn pump_errors(stream: impl std::io::Read, to: &std::sync::mpsc::SyncSender<Read>) {
    for text in lines(stream) {
        let (line, failed) = match text {
            Ok(text) => (Read::Line(Stream::Err, text), false),
            Err(error) => (Read::Failed(error), true),
        };
        if to.send(line).is_err() || failed {
            break;
        }
    }
}

/// Starts one of a run's reader threads. A thread that can't start is an error like any other:
/// the run's guard ends the child.
fn reader(read: impl FnOnce() + Send + 'static) -> std::io::Result<()> {
    #[cfg(test)]
    if test_hooks::READERS_FAIL.with(std::cell::Cell::take) {
        return Err(std::io::Error::other("a test's failed thread start"));
    }
    std::thread::Builder::new()
        .name("preparer output".into())
        .spawn(read)
        .map(drop)
}

/// A preparer child from its spawn to its reap. Dropped before it was reaped, an early `?` or
/// a stop that gave up, it kills the child's tree, which is not reaped yet, and leaves it to the
/// reaper ([`abandon`]), so the run's thread never waits long on a child that may be stuck
/// (review of #670).
struct Held {
    child: Option<crate::process::owned::Owned>,
    pid: u32,
    /// Its tree was killed, by the run or by the stop: one kill decides.
    killed: bool,
}

impl Held {
    fn new(mut child: crate::process::owned::Owned) -> Self {
        let pid = child.child().id();
        Self {
            child: Some(child),
            pid,
            killed: false,
        }
    }

    /// Kills the child's tree, which is not reaped yet, once.
    fn kill(&mut self) {
        if !std::mem::replace(&mut self.killed, true) {
            let pid = self.pid;
            kill_tree(self.get(), pid);
        }
    }

    fn get(&mut self) -> &mut crate::process::owned::Owned {
        self.child
            .as_mut()
            .expect("a held child stays held until it is reaped")
    }

    /// Reaps the child if it has ended. One that hasn't, after a stop and a kill, or whose reap
    /// failed, is left to the reaper, so the run never waits on it.
    fn reap(mut self, ended: bool) -> Option<ExitStatus> {
        if !ended {
            return None;
        }
        let status = self.get().wait().ok();
        if status.is_some() {
            self.child = None;
        }
        status
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        if self.child.is_some() {
            self.kill();
        }
        if let Some(child) = self.child.take() {
            abandon(child);
        }
    }
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
///
/// The stop's record goes under its lock, and the stop's decision with it: a stop that came in
/// the moment between the loop's last look and here may have sent only SIGTERM, so the group is
/// killed before the reap (review of #670).
fn finish<E: End>(mut end: E) -> E::Status {
    if end.untrack() {
        end.kill();
    }
    end.reap()
}

/// The last steps of a run, apart so [`finish`]'s order is testable.
trait End {
    type Status;
    /// Forgets the child's PID for the stop; true when a stop came first.
    fn untrack(&mut self) -> bool;
    /// Ends the child's group, which is not reaped yet.
    fn kill(&mut self);
    fn reap(self) -> Self::Status;
}

/// A run's child at its end.
struct Ending {
    child: Held,
    tracked: Option<crate::native_mods::ProcessGuard>,
    ended: bool,
}

impl End for Ending {
    type Status = Option<ExitStatus>;

    fn untrack(&mut self) -> bool {
        self.tracked
            .take()
            .is_some_and(crate::native_mods::ProcessGuard::release)
    }

    fn kill(&mut self) {
        self.child.kill();
    }

    fn reap(self) -> Option<ExitStatus> {
        self.child.reap(self.ended)
    }
}

/// Leaves a child that may not have ended, and was killed, to the reaper thread. With no reaper
/// (it has gone, as a run whose reaper can't start never spawns), the child is waited for here,
/// but no longer than a stop's wait: one that still hasn't ended is left unreaped, so its PID
/// stays its own. Nothing here starts a thread or can panic.
fn abandon(child: crate::process::owned::Owned) {
    #[cfg(test)]
    test_hooks::ABANDONED.with(|count| count.set(count.get() + 1));
    let Some(mut child) = reaper::reap(child) else {
        #[cfg(test)]
        test_hooks::REAPER_TOOK.with(|count| count.set(count.get() + 1));
        return;
    };
    let waiting = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if waiting.elapsed() < STOPPED_WAIT => std::thread::sleep(POLL),
            _ => break,
        }
    }
    std::mem::forget(child);
}

/// The one thread that reaps the children runs gave up on, fed by a channel. It starts in a run
/// before the run's child does, never in a destructor, so a launcher out of threads gets an
/// error from the run, not a panic in a drop (review of #670). It never waits on one child: it
/// looks at each in turn and reaps those that ended, so one stuck in the file system holds up
/// no other.
mod reaper {
    use crate::process::owned::Owned;
    use std::sync::mpsc::{Receiver, Sender};
    use std::sync::{Mutex, PoisonError};

    /// The launcher's reaper.
    static REAPER: Reaper = Reaper::new();

    /// Starts the launcher's reaper, once; an error when it can't start, so the run spawns
    /// nothing.
    pub(super) fn start() -> std::io::Result<()> {
        REAPER.start()
    }

    /// Hands `child` to the launcher's reaper; gives it back when there is none.
    pub(super) fn reap(child: Owned) -> Option<Owned> {
        REAPER.reap(child)
    }

    /// A reaper thread's sender, once it started. A test keeps its own.
    pub(super) struct Reaper(Mutex<Option<Sender<Owned>>>);

    impl Reaper {
        pub(super) const fn new() -> Self {
            Self(Mutex::new(None))
        }

        pub(super) fn start(&self) -> std::io::Result<()> {
            #[cfg(test)]
            if super::test_hooks::NO_REAPER.with(std::cell::Cell::get) {
                return Err(std::io::Error::other("a test's missing reaper"));
            }
            let mut reaper = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            if reaper.is_some() {
                return Ok(());
            }
            let (send, children) = std::sync::mpsc::channel::<Owned>();
            std::thread::Builder::new()
                .name("preparer reaper".into())
                .spawn(move || run(&children))?;
            *reaper = Some(send);
            Ok(())
        }

        pub(super) fn reap(&self, child: Owned) -> Option<Owned> {
            #[cfg(test)]
            if super::test_hooks::NO_REAPER.with(std::cell::Cell::get) {
                return Some(child);
            }
            let mut reaper = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            let unsent = match reaper.as_ref() {
                Some(send) => send.send(child).err().map(|unsent| unsent.0),
                None => Some(child),
            };
            // A reaper that has gone is started again by the next run, before its child.
            if unsent.is_some() {
                *reaper = None;
            }
            unsent
        }

        /// Puts in a sender whose reaper has gone, as when its thread ended.
        #[cfg(all(test, target_os = "linux"))]
        pub(super) fn gone(&self) {
            let (send, children) = std::sync::mpsc::channel::<Owned>();
            drop(children);
            *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(send);
        }
    }

    fn run(children: &Receiver<Owned>) {
        let mut pending = Vec::new();
        loop {
            if pending.is_empty() {
                match children.recv() {
                    Ok(child) => pending.push(child),
                    Err(_) => return,
                }
            } else if let Ok(child) = children.recv_timeout(super::POLL) {
                pending.push(child);
            }
            pending.extend(children.try_iter());
            reap_ended(&mut pending);
        }
    }

    /// Reaps each of `pending` that has ended, whatever its place; the others stay.
    pub(super) fn reap_ended(pending: &mut Vec<Owned>) {
        let mut index = 0;
        while index < pending.len() {
            match pending[index].try_wait() {
                Ok(None) => index += 1,
                Ok(Some(_)) => drop(pending.swap_remove(index)),
                // Its end can't be seen, so it is never waited on and never reaped: its PID, and
                // its entry for a quit, stay its own.
                Err(_) => std::mem::forget(pending.swap_remove(index)),
            }
        }
    }
}

/// Ends the child's whole tree, which is not reaped yet: SIGKILL to its process group on Unix,
/// its job object on Windows (where the leader's PID alone doesn't reach a child it left).
fn kill_tree(child: &mut crate::process::owned::Owned, pid: u32) {
    #[cfg(unix)]
    {
        let _ = child;
        crate::native_mods::kill_group(pid);
    }
    #[cfg(windows)]
    {
        let _ = pid;
        let _ = child.child().kill();
    }
}

/// Whether the child has ended, without reaping it.
fn probe(child: &mut crate::process::owned::Owned) -> std::io::Result<bool> {
    #[cfg(test)]
    if test_hooks::PROBE_FAILS.with(std::cell::Cell::take) {
        return Err(std::io::Error::other("a test's failed probe"));
    }
    child.child().ended_unreaped(false)
}

/// Runs a preparation's preparer, both streams' lines to `line` as they come (review of #670),
/// its stdout to the first [`MAX_OUTPUT`] bytes, as managed play had on the release base. When it
/// fails, its error ends with the last line it wrote, as a preparer's exit status alone says
/// little.
pub(crate) fn run_staged(
    command: Command,
    handoff: &Handoff<'_>,
    line: &dyn Fn(String),
) -> Result<(), String> {
    staged(command, handoff, line, Cap::Output)
}

/// [`run_staged`] with every line to `line`, as the backend's monitor forwarded Prepare's and a
/// switch's on the release base, to a ring that keeps the newest (review of #670).
pub(crate) fn run_staged_every_line(
    command: Command,
    handoff: &Handoff<'_>,
    line: &dyn Fn(String),
) -> Result<(), String> {
    staged(command, handoff, line, Cap::Off)
}

fn staged(
    command: Command,
    handoff: &Handoff<'_>,
    line: &dyn Fn(String),
    cap: Cap,
) -> Result<(), String> {
    let ran = run_streams(
        command,
        &mut |_, text| line(text),
        Some(handoff),
        handoff.stop(),
        cap,
    )?;
    if ran.status.success() {
        return Ok(());
    }
    let status = ran.status.to_string();
    Err(match ran.last {
        Some(last) => crate::i18n::trf(
            "The game's preparer stopped ({status}): {line}",
            &[("status", &status), ("line", &last)],
        ),
        None => crate::i18n::trf(
            "The game's preparer stopped ({status}).",
            &[("status", &status)],
        ),
    })
}

/// The most a preparer's standard output is read for, and the most of its error output kept (its last lines).
const MAX_OUTPUT: usize = 8 << 20;
const MAX_ERRORS: usize = 16 << 10;
/// The longest line kept whole; a longer one is cut.
const MAX_LINE: usize = 64 << 10;

/// `stream`'s lines, each read lossily (bytes that aren't UTF-8 become U+FFFD) and cut at [`MAX_LINE`].
fn lines(stream: impl std::io::Read) -> impl Iterator<Item = std::io::Result<String>> {
    let mut reader = BufReader::new(stream);
    let mut ended = false;
    std::iter::from_fn(move || {
        if ended {
            return None;
        }
        let mut bytes = Vec::new();
        match capped_line(&mut reader, &mut bytes) {
            Ok(true) => {
                let text = String::from_utf8_lossy(&bytes);
                Some(Ok(text.trim_end_matches('\r').to_owned()))
            }
            Ok(false) => {
                ended = true;
                None
            }
            Err(error) => {
                ended = true;
                Some(Err(error))
            }
        }
    })
}

/// Reads one line into `bytes`, its newline left out, keeping at most [`MAX_LINE`] bytes of it.
/// The rest of an overlong line is read and dropped as it comes, so it is never held whole
/// (follow-up to #670). False at the stream's end with nothing read.
fn capped_line(reader: &mut impl BufRead, bytes: &mut Vec<u8>) -> io::Result<bool> {
    let mut read = false;
    loop {
        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if available.is_empty() {
            return Ok(read);
        }
        read = true;
        let newline = available.iter().position(|&byte| byte == b'\n');
        let line = &available[..newline.unwrap_or(available.len())];
        let room = MAX_LINE.saturating_sub(bytes.len());
        bytes.extend_from_slice(&line[..line.len().min(room)]);
        let used = newline.map_or(available.len(), |at| at + 1);
        reader.consume(used);
        if newline.is_some() {
            return Ok(true);
        }
    }
}

/// The last lines of a stream, at most [`MAX_ERRORS`] bytes of them.
#[derive(Default)]
struct Tail {
    kept: std::collections::VecDeque<String>,
    size: usize,
}

impl Tail {
    fn keep(&mut self, line: String) {
        self.size += line.len();
        self.kept.push_back(line);
        while self.size > MAX_ERRORS {
            self.size -= self.kept.pop_front().map_or(0, |line| line.len());
        }
    }
}

impl From<Tail> for Vec<String> {
    fn from(tail: Tail) -> Self {
        tail.kept.into()
    }
}

/// The last lines of `stream`, at most [`MAX_ERRORS`] bytes of them.
#[cfg(all(test, unix))]
fn tail(stream: impl std::io::Read) -> Vec<String> {
    let mut tail = Tail::default();
    lines(stream)
        .map_while(Result::ok)
        .for_each(|line| tail.keep(line));
    tail.into()
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

/// Hooks for tests to put a stop at exact points of [`run_with`], on every platform.
#[cfg(test)]
mod test_hooks {
    use std::cell::{Cell, RefCell};

    type Hook = RefCell<Option<Box<dyn FnOnce()>>>;

    thread_local! {
        /// Runs in `run_with` between the spawn and the stop's registration of the child.
        pub(crate) static BEFORE_TRACK: Hook = const { RefCell::new(None) };
        /// Runs in `run_with` after its loop, before `finish`.
        pub(crate) static BEFORE_UNTRACK: Hook = const { RefCell::new(None) };
        /// How many children this thread began to leave to the reaper (`abandon`).
        pub(crate) static ABANDONED: Cell<usize> = const { Cell::new(0) };
        /// How many of them the reaper thread took; the rest were waited for in place.
        pub(crate) static REAPER_TOOK: Cell<usize> = const { Cell::new(0) };
        /// The next probe of whether the child ended fails.
        pub(crate) static PROBE_FAILS: Cell<bool> = const { Cell::new(false) };
        /// The next reader thread fails to start.
        pub(crate) static READERS_FAIL: Cell<bool> = const { Cell::new(false) };
        /// This thread's runs find no reaper thread, as when none could start.
        pub(crate) static NO_REAPER: Cell<bool> = const { Cell::new(false) };
        /// The next run's stdout fails to read after this many lines.
        pub(crate) static READ_FAILS_AFTER: Cell<Option<usize>> = const { Cell::new(None) };
    }

    /// Runs this thread's hook `which`, once, if a test set it.
    pub(crate) fn hook(which: &'static std::thread::LocalKey<Hook>) {
        if let Some(hook) = which.with(|hook| hook.borrow_mut().take()) {
            hook();
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::test_hooks::{ABANDONED, BEFORE_TRACK, REAPER_TOOK};
    // Their tests run on Linux only.
    #[cfg(target_os = "linux")]
    use super::test_hooks::{
        BEFORE_UNTRACK, NO_REAPER, PROBE_FAILS, READERS_FAIL, READ_FAILS_AFTER,
    };
    use super::*;
    use std::cell::Cell;

    /// A stop between the spawn and the child's registration: the child is killed at once and
    /// left to a reaping thread, so this thread never waits on a child that might be stuck in
    /// the file system (review of #670). A test can't make a child that ignores SIGKILL, so it
    /// proves the hand-off itself.
    #[cfg(unix)]
    #[test]
    fn a_stop_before_the_child_is_tracked_leaves_it_to_a_reaping_thread() {
        use std::os::unix::process::CommandExt;
        let stop = crate::native_mods::Cancellation::default();
        let later = stop.clone();
        BEFORE_TRACK.with(|hook| *hook.borrow_mut() = Some(Box::new(move || later.stop())));
        let before = ABANDONED.with(Cell::get);
        let taken = REAPER_TOOK.with(Cell::get);
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30"]).process_group(0);
        let started = Instant::now();
        let result = run_with(command, &mut |_| {}, None, Some(&stop));
        assert_eq!(result.unwrap_err(), crate::native_mods::CANCELLED);
        assert_eq!(
            ABANDONED.with(Cell::get),
            before + 1,
            "the child went to cleanup, not dropped on this one"
        );
        assert_eq!(
            REAPER_TOOK.with(Cell::get),
            taken + 1,
            "the reaper thread took the child, rather than this thread waiting on it"
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    /// A child that ended on its own, leaving a quiet process in its group that ignores SIGTERM,
    /// with a stop in the moment between the loop's last look and the untrack: the stop's decision
    /// is taken with the untrack, so the group is still killed before the reap (review of #670).
    #[cfg(target_os = "linux")]
    #[test]
    fn a_stop_in_the_moment_before_the_untrack_still_ends_the_group() {
        use std::os::unix::process::CommandExt;
        let dir = tempfile::tempdir().unwrap();
        let quiet = dir.path().join("quiet.pid");
        let stop = crate::native_mods::Cancellation::default();
        let later = stop.clone();
        BEFORE_UNTRACK.with(|hook| *hook.borrow_mut() = Some(Box::new(move || later.stop())));
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "sh -c 'trap \"\" TERM; exec >/dev/null 2>&1; echo $$ > \"$1\"; exec sleep 30' quiet \"$1\" & while [ ! -s \"$1\" ]; do sleep 0.05; done",
                "leader",
            ])
            .arg(&quiet)
            .process_group(0);
        let result = run_with(command, &mut |_| {}, None, Some(&stop));
        assert_eq!(result.unwrap_err(), crate::native_mods::CANCELLED);
        let pid = fs::read_to_string(&quiet).unwrap();
        let runs = || {
            fs::read_to_string(format!("/proc/{}/stat", pid.trim()))
                .ok()
                .and_then(|stat| {
                    stat.rsplit(')')
                        .next()
                        .map(|rest| rest.trim_start().to_owned())
                })
                .is_some_and(|rest| !rest.starts_with('Z'))
        };
        let waiting = Instant::now();
        while runs() && waiting.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            !runs(),
            "the quiet process the child left in its group still runs"
        );
    }

    /// Whether a process whose command line holds `marker` still runs (a zombie doesn't).
    #[cfg(target_os = "linux")]
    fn runs_with(marker: &str) -> bool {
        fs::read_dir("/proc").unwrap().flatten().any(|entry| {
            let dir = entry.path();
            let named = fs::read(dir.join("cmdline"))
                .is_ok_and(|line| String::from_utf8_lossy(&line).contains(marker));
            let zombie = fs::read_to_string(dir.join("stat")).is_ok_and(|stat| {
                stat.rsplit(')')
                    .next()
                    .is_some_and(|rest| rest.trim_start().starts_with('Z'))
            });
            named && !zombie
        })
    }

    /// Waits up to two seconds for every process with `marker` to end.
    #[cfg(target_os = "linux")]
    fn ends_soon(marker: &str) -> bool {
        let waiting = Instant::now();
        while runs_with(marker) && waiting.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(50));
        }
        !runs_with(marker)
    }

    /// A child, in its own group, that leaves a process ignoring SIGTERM there, says "ready" once
    /// both run, and doesn't end. `marker` names both.
    #[cfg(target_os = "linux")]
    fn stubborn(marker: &str) -> Command {
        use std::os::unix::process::CommandExt;
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "trap '' TERM; sh -c 'sleep 30; :' \"$1\" & echo ready; exec sh -c 'sleep 30; :' \"$1\"",
                "stubborn",
                marker,
            ])
            .process_group(0);
        command
    }

    /// A failed look at whether the child ended returns through `?`: the child's guard still ends
    /// its whole group, and leaves the reap to a thread (review of #670).
    #[cfg(target_os = "linux")]
    #[test]
    fn a_failed_probe_of_the_child_still_ends_its_group_off_this_thread() {
        let marker = format!("probe-{}", std::process::id());
        let before = ABANDONED.with(Cell::get);
        let started = Instant::now();
        let result = run_with(
            stubborn(&marker),
            &mut |line| {
                if line == "ready" {
                    PROBE_FAILS.with(|fails| fails.set(true));
                }
            },
            None,
            None,
        );
        assert!(result.unwrap_err().contains("a test's failed probe"));
        assert_eq!(
            ABANDONED.with(Cell::get),
            before + 1,
            "reaped off this thread"
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(
            ends_soon(&marker),
            "what the child left in its group still runs"
        );
    }

    /// A reader thread that can't start returns through `?` too, and the guard ends the child.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_reader_that_cannot_start_leaves_no_child_behind() {
        let marker = format!("reader-{}", std::process::id());
        READERS_FAIL.with(|fails| fails.set(true));
        let before = ABANDONED.with(Cell::get);
        let result = run_with(stubborn(&marker), &mut |_| {}, None, None);
        assert!(result.unwrap_err().contains("a test's failed thread start"));
        assert_eq!(
            ABANDONED.with(Cell::get),
            before + 1,
            "reaped off this thread"
        );
        assert!(ends_soon(&marker), "the child still runs");
    }

    /// A read error on the child's stdout ends the child at once, as the base did, rather than
    /// waiting for a child that may never end (review of #670).
    #[cfg(target_os = "linux")]
    #[test]
    fn a_read_error_ends_the_child_at_once() {
        let marker = format!("read-{}", std::process::id());
        READ_FAILS_AFTER.with(|after| after.set(Some(0)));
        let started = Instant::now();
        let result = run_with(stubborn(&marker), &mut |_| {}, None, None);
        assert!(result.unwrap_err().contains("a test's read error"));
        assert!(
            started.elapsed() < STOPPED_WAIT,
            "it waited {:?}",
            started.elapsed()
        );
        assert!(ends_soon(&marker), "the child still runs");
    }

    /// With no reaper to hand it to, a dropped child's guard still ends it: it kills the child and
    /// waits for it here, bounded, with no thread started and no panic in a destructor (review of
    /// #670).
    #[cfg(target_os = "linux")]
    #[test]
    fn with_no_reaper_a_dropped_child_is_ended_here_within_a_bound() {
        use crate::process::owned::{Registry, CHILDREN};
        // The test's own children: the launcher's quit registry never holds them (review of
        // #681).
        static OWN: Registry = Registry::new();
        let marker = format!("no-reaper-{}", std::process::id());
        let mut child = crate::process::leases::spawn_preparer(stubborn(&marker), None, None, true)
            .map(|child| OWN.track_preparer(child))
            .unwrap();
        let pid = child.child().id();
        assert!(OWN.holds(pid) && !CHILDREN.holds(pid));
        let held = Held::new(child);
        NO_REAPER.with(|none| none.set(true));
        let started = Instant::now();
        drop(held);
        NO_REAPER.with(|none| none.set(false));
        assert!(
            started.elapsed() < STOPPED_WAIT + Duration::from_secs(1),
            "it waited {:?}",
            started.elapsed()
        );
        assert!(ends_soon(&marker), "the child still runs");
    }

    /// A run whose reaper can't start fails before it starts its child, so no child it couldn't
    /// hand on is ever forgotten (review of #670).
    #[cfg(target_os = "linux")]
    #[test]
    fn a_run_whose_reaper_cannot_start_starts_no_child() {
        let dir = tempfile::tempdir().unwrap();
        let ran = dir.path().join("ran");
        let mut command = Command::new("sh");
        command.args(["-c", "touch \"$1\"", "child"]).arg(&ran);
        NO_REAPER.with(|none| none.set(true));
        let result = run_with(command, &mut |_| {}, None, None);
        NO_REAPER.with(|none| none.set(false));
        let error = result.expect_err("a run with no reaper is an error");
        assert!(error.contains("a test's missing reaper"), "{error}");
        std::thread::sleep(Duration::from_millis(300));
        assert!(!ran.exists(), "the child was started");
    }

    /// A leader that exits with success while a helper it started keeps its output open: the
    /// helper is killed after the grace, and the run is an error, as what the helper held was not
    /// read to its end (review of #670).
    #[test]
    fn a_leader_that_succeeds_while_a_helper_holds_its_output_is_not_a_success() {
        use std::os::unix::process::CommandExt;
        let mut command = Command::new("sh");
        command
            .args(["-c", "sleep 30 & echo done; exit 0"])
            .process_group(0);
        let started = Instant::now();
        let error = run_with(command, &mut |_| {}, None, None)
            .expect_err("a run whose output was cut off is not a clean success");
        assert!(error.contains("kept its output open"), "{error}");
        assert!(started.elapsed() < STOPPED_WAIT + Duration::from_secs(1));
    }

    /// A preparation's stderr reaches its caller while the preparer runs, as the backend's monitor
    /// forwarded it (review of #670): this preparer goes on only once its caller has seen its
    /// line. A failed one's error ends with the last line it wrote.
    #[test]
    fn a_preparations_stderr_reaches_its_caller_while_it_runs() {
        let dir = tempfile::tempdir().unwrap();
        let seen = dir.path().join("seen");
        let lock = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(dir.path().join(".prepare.lock"))
            .unwrap();
        lock.lock().unwrap();
        let preparation = Preparation::new(lock);
        let mut live = Command::new("sh");
        live.args([
            "-c",
            "echo working >&2; i=0; while [ ! -e \"$1\" ] && [ $i -lt 50 ]; do sleep 0.1; i=$((i+1)); done; [ -e \"$1\" ]",
            "live",
        ])
        .arg(&seen);
        let result = run_staged(live, &preparation.handoff(None), &|line| {
            if line == "working" {
                fs::write(&seen, "").unwrap();
            }
        });
        assert_eq!(
            result,
            Ok(()),
            "its stderr reached the caller only after it gave up waiting"
        );
        let mut failing = Command::new("sh");
        failing.args([
            "-c",
            "echo copying; sleep 0.2; echo 'the sounds index is missing' >&2; exit 3",
        ]);
        let error = run_staged(failing, &preparation.handoff(None), &|_| {}).unwrap_err();
        assert!(error.ends_with(": the sounds index is missing"), "{error}");
    }

    /// A preparer started without the preparation's lock, such as developer-install's probe, leads
    /// its own group too: when it exits leaving a helper on its output, the run's kill of its
    /// group ends the helper, and no group the launcher didn't create is signalled (review of
    /// #670).
    #[cfg(target_os = "linux")]
    #[test]
    fn a_preparer_without_the_lock_leads_its_own_group_too() {
        let marker = format!("unlocked-{}", std::process::id());
        let mut command = Command::new("sh");
        command
            .args(["-c", "sh -c 'sleep 30; :' \"$1\" & exit 0", "unlocked"])
            .arg(&marker);
        let error = run_with(command, &mut |_| {}, None, None)
            .expect_err("a run whose output was cut off is not a clean success");
        assert!(error.contains("kept its output open"), "{error}");
        assert!(
            ends_soon(&marker),
            "the helper it left on its output still runs"
        );
    }

    /// The reaper reaps a child that ended even when one before it in its queue hasn't, as one
    /// stuck in the file system wouldn't: it never waits on one child (review of #670).
    #[test]
    fn the_reaper_reaps_an_ended_child_queued_behind_one_still_running() {
        use crate::process::owned::{Registry, CHILDREN};
        // The test's own children: the launcher's quit registry never holds them (review of
        // #681).
        static OWN: Registry = Registry::new();
        let mut running = OWN.spawn(Command::new("sleep").arg("30")).unwrap();
        let running_pid = running.child().id();
        let mut ended = OWN.spawn(&mut Command::new("true")).unwrap();
        let ended_pid = ended.child().id();
        for pid in [running_pid, ended_pid] {
            assert!(OWN.holds(pid) && !CHILDREN.holds(pid));
        }
        while !ended.child().ended_unreaped(false).unwrap() {
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut pending = vec![running, ended];
        reaper::reap_ended(&mut pending);
        assert_eq!(
            pending.len(),
            1,
            "the ended child, queued second, was reaped"
        );
        assert_eq!(pending[0].child().id(), running_pid);
        let _ = pending[0].child().kill();
    }

    /// The reaper thread itself, not only its policy: with a child that stays running already
    /// queued, a child that ends after it was queued is reaped within a bound. A reaper that
    /// blocked on its queue while children wait would never see it end (review of #670).
    #[cfg(target_os = "linux")]
    #[test]
    fn the_reaper_thread_reaps_a_child_that_ends_while_another_still_runs() {
        use crate::process::owned::{Registry, CHILDREN};
        // The test's own children: the launcher's quit registry never holds them (review of
        // #681).
        static OWN: Registry = Registry::new();
        let unreaped = |pid: u32| Path::new(&format!("/proc/{pid}")).exists();
        reaper::start().unwrap();
        let mut running = OWN.spawn(Command::new("sleep").arg("30")).unwrap();
        let running_pid = running.child().id();
        assert!(OWN.holds(running_pid) && !CHILDREN.holds(running_pid));
        assert!(reaper::reap(running).is_none(), "the reaper took it");
        let mut later = OWN.spawn(Command::new("sleep").arg("0.3")).unwrap();
        let later_pid = later.child().id();
        assert!(OWN.holds(later_pid) && !CHILDREN.holds(later_pid));
        assert!(reaper::reap(later).is_none(), "the reaper took it");
        let waiting = Instant::now();
        while unreaped(later_pid) && waiting.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(50));
        }
        let reaped = !unreaped(later_pid);
        let still_pending = unreaped(running_pid);
        // SAFETY: the reaper holds this child unreaped, so its PID is still its own.
        unsafe {
            libc::kill(running_pid as libc::pid_t, libc::SIGKILL);
        }
        assert!(
            reaped,
            "the child that ended after it was queued is still unreaped"
        );
        assert!(still_pending, "the running child is still the reaper's");
    }

    /// Prepare's and a switch's preparation hand every line on, past [`MAX_OUTPUT`] too, as the
    /// backend's monitor did on the release base: the window's ring keeps the newest (review of
    /// #670).
    #[test]
    fn a_preparations_every_line_reaches_its_caller_past_the_output_cap() {
        let dir = tempfile::tempdir().unwrap();
        let lock = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(dir.path().join(".prepare.lock"))
            .unwrap();
        lock.lock().unwrap();
        let preparation = Preparation::new(lock);
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "awk 'BEGIN { s = sprintf(\"%1000s\", \"\"); for (i = 0; i < 9000; i++) print \"early\" s; for (i = 0; i < 50; i++) print \"late \" i }'",
        ]);
        let late = Cell::new(0);
        run_staged_every_line(command, &preparation.handoff(None), &|line| {
            if line.starts_with("late ") {
                late.set(late.get() + 1);
            }
        })
        .unwrap();
        assert_eq!(late.get(), 50, "lines past the cap were dropped");
    }

    /// A failed preparation's error ends with the last line it wrote, also when that came after
    /// the output cap, past which lines no longer reach the caller (review of #670).
    #[test]
    fn a_failed_preparations_error_ends_with_its_last_line_even_past_the_output_cap() {
        let dir = tempfile::tempdir().unwrap();
        let lock = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(dir.path().join(".prepare.lock"))
            .unwrap();
        lock.lock().unwrap();
        let preparation = Preparation::new(lock);
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "awk 'BEGIN { s = sprintf(\"%1000s\", \"\"); for (i = 0; i < 9000; i++) print \"early\" s }'; echo 'the final failure'; exit 1",
        ]);
        let error = run_staged(command, &preparation.handoff(None), &|_| {}).unwrap_err();
        assert!(
            error.ends_with(": the final failure"),
            "{}",
            &error[..200.min(error.len())]
        );
    }

    /// A preparer that ended with its final reply queued behind many log lines, taken by a slow
    /// caller: the time the caller takes is not the child's, so the reply still arrives (review of
    /// #670). The lines take longer than any of the run's deadlines.
    #[test]
    fn a_slow_caller_still_gets_the_final_reply_queued_behind_its_lines() {
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "for n in $(seq 40); do echo '{\"message\":\"line\"}'; done; echo '{\"format\":1,\"done\":true}'",
        ]);
        let taken = Cell::new(0);
        let started = Instant::now();
        let reply = reply(
            command,
            &|_| {
                std::thread::sleep(Duration::from_millis(150));
                taken.set(taken.get() + 1);
            },
            None,
        );
        assert!(
            started.elapsed() > STOPPED_WAIT,
            "the lines outlast the deadlines"
        );
        assert_eq!(taken.get(), 40);
        assert_eq!(reply.unwrap().unwrap()["done"], true);
    }

    /// A reaper whose thread has gone gives the child back and drops its sender, so the next start
    /// puts in a working reaper, which then reaps (follow-up to #670).
    #[cfg(target_os = "linux")]
    #[test]
    fn a_gone_reaper_gives_the_child_back_and_the_next_start_replaces_it() {
        use crate::process::owned::{Registry, CHILDREN};
        // The test's own children: the launcher's quit registry never holds them (review of
        // #681).
        static OWN: Registry = Registry::new();
        let reaper = reaper::Reaper::new();
        reaper.gone();
        let mut first = OWN.spawn(&mut Command::new("true")).unwrap();
        let first_pid = first.child().id();
        assert!(OWN.holds(first_pid) && !CHILDREN.holds(first_pid));
        let mut back = reaper
            .reap(first)
            .expect("a gone reaper gives the child back");
        assert_eq!(back.child().id(), first_pid);
        drop(back);
        reaper.start().unwrap();
        let mut later = OWN.spawn(&mut Command::new("true")).unwrap();
        let pid = later.child().id();
        assert!(OWN.holds(pid) && !CHILDREN.holds(pid));
        assert!(
            reaper.reap(later).is_none(),
            "the next start put in a working reaper"
        );
        let waiting = Instant::now();
        while Path::new(&format!("/proc/{pid}")).exists()
            && waiting.elapsed() < Duration::from_secs(3)
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "the new reaper reaped it"
        );
    }

    /// Runs this binary's test `name` alone in a child process with `env` set, and checks that
    /// the child really ran it and it passed. A name that no longer matches a test runs nothing
    /// and exits with success, so that alone would skip the test's assertions unseen (review of
    /// #681).
    #[cfg(target_os = "linux")]
    fn in_child(name: &str, env: &str) {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name])
            .env(env, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{name} failed in its child process:\n{stdout}{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            stdout.contains("test result: ok. 1 passed"),
            "the child process ran no test named {name}:\n{stdout}"
        );
    }

    /// A line far longer than [`MAX_LINE`] is cut as it is read: the reader never holds more of
    /// it than that, so a preparer writing one huge line can't grow the launcher's memory
    /// (follow-up to #670).
    #[cfg(target_os = "linux")]
    #[test]
    fn an_overlong_line_is_cut_as_it_is_read() {
        const CHILD: &str = "SUNRISE_PREPARER_LONG_LINE_TEST";
        if std::env::var_os(CHILD).is_none() {
            in_child(
                "preparer::tests::an_overlong_line_is_cut_as_it_is_read",
                CHILD,
            );
            return;
        }
        /// `left` bytes of `x`, with no newline.
        struct Long(usize);
        impl std::io::Read for Long {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                let n = buf.len().min(self.0);
                buf[..n].fill(b'x');
                self.0 -= n;
                Ok(n)
            }
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
        let stream = std::io::Read::chain(Long(128 << 20), &b"\nafter\n"[..]);
        let read: Vec<String> = lines(stream).map(Result::unwrap).collect();
        let grew = peak().saturating_sub(before);
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].len(), MAX_LINE);
        assert_eq!(read[1], "after");
        assert!(
            grew < 32 << 20,
            "reading the line grew the peak memory by {} MiB",
            grew >> 20
        );
    }

    /// A read error on stderr is handed on to the run, as one on stdout is, rather than taken for
    /// the stream's end (follow-up to #670).
    #[test]
    fn a_stderr_read_error_is_handed_on_not_taken_for_its_end() {
        struct Broken(bool);
        impl std::io::Read for Broken {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if std::mem::replace(&mut self.0, true) {
                    return Err(io::Error::other("a test's stderr read error"));
                }
                buf[..6].copy_from_slice(b"first\n");
                Ok(6)
            }
        }
        let (to, from) = std::sync::mpsc::sync_channel(8);
        pump_errors(Broken(false), &to);
        drop(to);
        let got: Vec<Read> = from.iter().collect();
        assert!(
            matches!(got.first(), Some(Read::Line(Stream::Err, text)) if text == "first"),
            "the first line came through"
        );
        assert!(
            matches!(got.get(1), Some(Read::Failed(_))),
            "the read error was taken for the end of stderr"
        );
    }

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
        // A two-byte character split by the cut: its first byte is kept alone and read as U+FFFD,
        // its second is dropped with the rest of the line, and the next line comes whole (review
        // of #681).
        let split = format!("{}é and more\nnext line\n", "a".repeat(MAX_LINE - 1));
        let read: Vec<String> = lines(split.as_bytes()).map(Result::unwrap).collect();
        assert_eq!(
            read,
            [
                format!("{}\u{fffd}", "a".repeat(MAX_LINE - 1)),
                "next line".into()
            ]
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
        // The stop's record of the PID goes before the reap, and a stop that came first has the
        // group killed between the two.
        struct Steps(Vec<&'static str>, bool);
        impl End for Steps {
            type Status = Vec<&'static str>;
            fn untrack(&mut self) -> bool {
                self.0.push("untrack");
                self.1
            }
            fn kill(&mut self) {
                self.0.push("kill");
            }
            fn reap(mut self) -> Vec<&'static str> {
                self.0.push("reap");
                self.0
            }
        }
        assert_eq!(finish(Steps(Vec::new(), false)), ["untrack", "reap"]);
        assert_eq!(finish(Steps(Vec::new(), true)), ["untrack", "kill", "reap"]);
    }

    /// A child that floods its stdout while the caller is slow to take it is held back by its
    /// pipe: the launcher's memory doesn't grow with what the child writes (review of #670).
    #[cfg(target_os = "linux")]
    #[test]
    fn a_child_flooding_its_output_does_not_grow_the_launchers_memory() {
        const CHILD: &str = "SUNRISE_PREPARER_FLOOD_TEST";
        if std::env::var_os(CHILD).is_none() {
            in_child(
                "preparer::tests::a_child_flooding_its_output_does_not_grow_the_launchers_memory",
                CHILD,
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
