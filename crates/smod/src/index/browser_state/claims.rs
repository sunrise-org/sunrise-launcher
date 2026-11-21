//! Conservative comparison of game and launcher browser claims under the shared home lock.
use super::{DetailReason, render_english};
use serde_json::Value;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Component, Path, PathBuf};

#[derive(Debug)]
pub enum ClaimProblem {
    Destination {
        file: String,
        reason: DetailReason,
    },
    Lookup {
        row: String,
        file: String,
        reason: DetailReason,
    },
    Conflict {
        file: String,
        row: String,
        possible: bool,
    },
}

impl ClaimProblem {
    pub fn render(&self, format: impl Fn(&'static str, &[(&str, &str)]) -> String) -> String {
        match self {
            Self::Destination { file, reason } => format(
                "Couldn't use {file} as the mod's destination: {reason}",
                &[("file", file), ("reason", &reason.render(&format))],
            ),
            Self::Lookup { row, file, reason } => format(
                "Couldn't check the file {file} claimed by {row}: {reason}",
                &[
                    ("file", file),
                    ("row", row),
                    ("reason", &reason.render(&format)),
                ],
            ),
            Self::Conflict {
                file,
                row,
                possible,
            } => format(
                if *possible {
                    "{file} may conflict with {row}."
                } else {
                    "{file} belongs to {row}: remove it first."
                },
                &[("file", file), ("row", row)],
            ),
        }
    }
}

impl fmt::Display for ClaimProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render(render_english))
    }
}

#[cfg(unix)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct Identity(u64, u64);
#[cfg(windows)]
type Identity = same_file::Handle;

fn identity(path: &Path, meta: &fs::Metadata) -> io::Result<Identity> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = path;
        Ok(Identity(meta.dev(), meta.ino()))
    }
    #[cfg(windows)]
    {
        let _ = meta;
        Identity::from_path(path)
    }
}

fn is_link(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(unix)]
    {
        meta.file_type().is_symlink()
    }
}

fn portable(name: &str) -> bool {
    !name.is_empty()
        && !name.ends_with(['.', ' '])
        && !name
            .bytes()
            .any(|b| b < b' ' || b"<>:\"/\\|?*".contains(&b))
        && {
            let stem = name
                .split('.')
                .next()
                .unwrap_or_default()
                .trim_end_matches(' ')
                .to_ascii_uppercase();
            !matches!(
                stem.as_str(),
                "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
            ) && !["COM", "LPT"].into_iter().any(|prefix| {
                matches!(
                    stem.strip_prefix(prefix),
                    Some("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³")
                )
            })
        }
}

/// A component whose spelling has a known ASCII-folded key before it exists.
pub fn portable_ascii_component(name: &str) -> bool {
    name.is_ascii() && !name.contains('~') && portable(name)
}

fn folded(name: &OsStr) -> Option<String> {
    name.to_str()
        .filter(|name| portable_ascii_component(name))
        .map(str::to_ascii_lowercase)
}

fn destination_spelling(file: &str) -> bool {
    let mut parts = file
        .split(|c| c == '/' || (cfg!(windows) && c == '\\'))
        .peekable();
    let mut depth = 0;
    while let Some(name) = parts.next() {
        match name {
            "." if parts.peek().is_some() => (),
            ".." if depth > 0 && parts.peek().is_some() => depth -= 1,
            name if portable(name) => depth += 1,
            _ => return false,
        }
    }
    depth > 0
}

fn home_key(relative: &Path) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    for c in relative.components() {
        match c {
            Component::CurDir => (),
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::Normal(name) => parts.push(name),
            _ => return None,
        }
    }
    (!parts.is_empty()).then_some(())?;
    parts.into_iter().map(folded).collect()
}

struct Native {
    anchor: Identity,
    suffix: Vec<Option<String>>,
    leaf: Option<Identity>,
    crossed_link: bool,
}

fn walk(root: &Path, relative: &Path, is_destination: bool) -> Result<Native, DetailReason> {
    let mut dir = root.to_path_buf();
    let mut suffix = Vec::new();
    let mut via_link = Vec::new();
    let mut crossed_link = false;
    let mut leaf = None;
    let mut parts = relative.components().peekable();
    while let Some(component) = parts.next() {
        let last = parts.peek().is_none();
        match component {
            Component::CurDir => (),
            Component::ParentDir if !suffix.is_empty() => {
                if last {
                    suffix.push(None);
                } else {
                    suffix.pop();
                }
            }
            Component::ParentDir => {
                crossed_link |= via_link.pop().unwrap_or(false);
                // Native '..' must look through the actual parent, including at the home or root.
                dir = dir.join("..").canonicalize()?;
            }
            Component::Normal(name) if !suffix.is_empty() => suffix.push(folded(name)),
            Component::Normal(name) => {
                let file = dir.join(name);
                let meta = match fs::symlink_metadata(&file) {
                    Ok(meta) => meta,
                    Err(e) if e.kind() == ErrorKind::NotFound => {
                        suffix.push(folded(name));
                        continue;
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            ErrorKind::InvalidInput | ErrorKind::InvalidFilename
                        ) =>
                    {
                        if is_destination {
                            return Err(e.into());
                        }
                        suffix.push(None);
                        continue;
                    }
                    Err(e) => return Err(e.into()),
                };
                let link = is_link(&meta);
                if is_destination && last && (link || !meta.is_file()) {
                    return Err(DetailReason::NotPlainDestination);
                }
                let (file, meta) = if link {
                    // A dangling link is not an available name. Its location cannot be bounded.
                    let file = file.canonicalize()?;
                    let meta = fs::metadata(&file)?;
                    (file, meta)
                } else {
                    (file, meta)
                };
                if meta.is_dir() {
                    dir = file;
                    via_link.push(link);
                } else if meta.is_file() && last {
                    leaf = Some(identity(&file, &meta)?);
                    let native_name = file.file_name().and_then(folded);
                    // Even an unportable spelling has an identity if it resolves regularly.
                    suffix.push(folded(name).and(native_name));
                    if link {
                        dir = file.parent().expect("a file has a parent").to_path_buf();
                    }
                } else {
                    if is_destination {
                        return Err(DetailReason::NotDirectory);
                    }
                    suffix.push(None);
                }
            }
            _ => return Err(DetailReason::UnknownRoot),
        }
    }
    let anchor = identity(&dir, &fs::metadata(&dir)?)?;
    Ok(Native {
        anchor,
        suffix,
        leaf,
        crossed_link,
    })
}

// Split an absolute claim outside the home without normalizing the walk's '..' components.
fn filesystem_root(path: &Path) -> Option<(PathBuf, &Path)> {
    let mut components = path.components();
    let mut root = PathBuf::new();
    if let Some(Component::Prefix(prefix)) = components.clone().next() {
        root.push(prefix.as_os_str());
        components.next();
    }
    let Component::RootDir = components.next()? else {
        return None;
    };
    root.push(Component::RootDir.as_os_str());
    Some((root, components.as_path()))
}

fn locate_claim(
    home: &Path,
    actual_home: &Path,
    claim: &Path,
) -> Result<(Native, Option<Vec<String>>), DetailReason> {
    let joined;
    let (root, relative) = if matches!(
        claim.components().next(),
        Some(Component::Prefix(_) | Component::RootDir)
    ) {
        // Join root-relative Windows names onto the home's drive. Never join a plain relative
        // claim onto a verbatim path: Windows `PathBuf::push` would erase its '..' components.
        joined = home.join(claim);
        if let Ok(relative) = joined.strip_prefix(home) {
            (None, relative)
        } else {
            let (root, tail) = filesystem_root(&joined).ok_or(DetailReason::UnknownRoot)?;
            (Some(root), tail)
        }
    } else {
        (None, claim)
    };
    let native = walk(root.as_deref().unwrap_or(actual_home), relative, false)?;
    let key = (root.is_none() && !native.crossed_link)
        .then(|| home_key(relative))
        .flatten();
    Ok((native, key))
}

enum Conflict {
    Confirmed,
    Possible,
}

fn conflicts(
    left_key: Option<&[String]>,
    left: &Native,
    right_key: Option<&[String]>,
    right: &Native,
) -> Option<Conflict> {
    if left
        .leaf
        .as_ref()
        .is_some_and(|leaf| right.leaf.as_ref() == Some(leaf))
        || left_key
            .zip(right_key)
            .is_some_and(|(a, b)| a.starts_with(b) || b.starts_with(a))
    {
        return Some(Conflict::Confirmed);
    }
    if left.anchor != right.anchor {
        return None;
    }
    for (left, right) in left.suffix.iter().zip(&right.suffix) {
        match (left, right) {
            (Some(a), Some(b)) if a != b => return None,
            (Some(_), Some(_)) => (),
            _ => return Some(Conflict::Possible),
        }
    }
    Some(Conflict::Confirmed)
}

/// Check both views of every other manifest claim without opening the destination's bytes.
/// The caller holds the shared lock, and excludes the target project's own Sunrise row.
pub(super) fn check_destination<'a>(
    home: &Path,
    file: &str,
    rows: impl IntoIterator<Item = &'a Value>,
) -> Result<(), ClaimProblem> {
    let destination = |reason| ClaimProblem::Destination {
        file: file.to_owned(),
        reason,
    };
    if !destination_spelling(file) {
        return Err(destination(DetailReason::NonPortableDestination));
    }
    let home = std::path::absolute(home).map_err(|e| destination(e.into()))?;
    let actual_home = home.canonicalize().map_err(|e| destination(e.into()))?;
    let left = walk(&actual_home, Path::new(file), true).map_err(destination)?;
    let left_key = (!left.crossed_link)
        .then(|| home_key(Path::new(file)))
        .flatten();
    for entry in rows {
        let Some(other) = entry["file"].as_str() else {
            continue;
        };
        let row = || {
            entry["title"]
                .as_str()
                .or_else(|| entry["project"].as_str())
                .unwrap_or(other)
                .to_owned()
        };
        let lookup = |reason| ClaimProblem::Lookup {
            row: row(),
            file: other.to_owned(),
            reason,
        };
        let (right, right_key) =
            locate_claim(&home, &actual_home, Path::new(other)).map_err(lookup)?;
        if let Some(conflict) = conflicts(left_key.as_deref(), &left, right_key.as_deref(), &right)
        {
            return Err(ClaimProblem::Conflict {
                file: file.to_owned(),
                row: row(),
                possible: matches!(conflict, Conflict::Possible),
            });
        }
    }
    Ok(())
}
