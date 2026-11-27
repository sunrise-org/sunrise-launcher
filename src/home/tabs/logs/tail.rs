//! The end of a profile's `latest.log`, read off the UI thread, for a profile with no run in
//! this session.
use crate::LOG_LINES;

/// A file's length and modification time: a change in either means it is read again.
pub(super) type Stamp = (u64, Option<std::time::SystemTime>);

/// How much of the end of `latest.log` is read.
const TAIL_BYTES: u64 = 4 << 20;

/// What the last run's log file holds, as far as it is known.
#[derive(Debug, PartialEq)]
pub(super) enum Tail {
    Loading,
    /// No file, or an empty one: the game has not logged yet.
    Absent,
    /// The file is there and could not be read, with the reason in plain words.
    Failed(String),
    Lines(Vec<String>),
}

pub(super) struct FileLog {
    pub(super) path: std::path::PathBuf,
    pub(super) stamp: Option<Stamp>,
    pub(super) tail: Tail,
}

/// Reads the last `LOG_LINES` lines from the end of the file; `None` when it still has the
/// `known` stamp.
fn read_tail(
    file: &mut std::fs::File,
    known: Option<Stamp>,
) -> std::io::Result<Option<(Stamp, Vec<String>)>> {
    use std::io::{Read, Seek, SeekFrom};
    let meta = file.metadata()?;
    let stamp = (meta.len(), meta.modified().ok());
    if known == Some(stamp) {
        return Ok(None);
    }
    let start = stamp.0.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.take(TAIL_BYTES).read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    // Reading from the middle of the file starts inside a line: drop that part.
    let skip = usize::from(start > 0);
    let all: Vec<&str> = text.lines().skip(skip).collect();
    let tail = all[all.len().saturating_sub(LOG_LINES)..]
        .iter()
        .map(|l| l.to_string())
        .collect();
    Ok(Some((stamp, tail)))
}

/// The state of the log file at `path`, or `None` when it has not changed since `known`.
pub(super) fn load_tail(
    path: &std::path::Path,
    known: Option<Stamp>,
) -> Option<(Option<Stamp>, Tail)> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some((None, Tail::Absent)),
        Err(e) => return Some((None, Tail::Failed(plain_error(&e)))),
    };
    match read_tail(&mut file, known) {
        Ok(None) => None,
        Ok(Some((stamp, lines))) if lines.is_empty() => Some((Some(stamp), Tail::Absent)),
        Ok(Some((stamp, lines))) => Some((Some(stamp), Tail::Lines(lines))),
        Err(e) => Some((None, Tail::Failed(plain_error(&e)))),
    }
}

/// An I/O error without its operating-system code: "Permission denied".
pub(super) fn plain_error(e: &std::io::Error) -> String {
    let text = e.to_string();
    text.split(" (os error").next().unwrap_or(&text).to_string()
}
