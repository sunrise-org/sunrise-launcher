//! Copying what an import keeps from another launcher's game folder into a new profile's.
//! The source is only read. A link is never followed (a world or pack folder linked to
//! somewhere else is left out), and nothing already in the new folder is overwritten except the
//! player's own options file it started with, which the other launcher's replaces on purpose.
use super::*;
use std::io::{self, Read as _, Write as _};
use std::sync::atomic::{AtomicU64, Ordering};

/// The game's options files: the vanilla ones, then OptiFine's and Iris/Oculus' settings.
const OPTION_FILES: &[&str] = &[crate::paths::OPTIONS, "optionsof.txt", "optionsshaders.txt"];

/// How far a copy is, in bytes, shared between the worker and whoever shows it.
#[derive(Default)]
pub struct Progress {
    done: AtomicU64,
    total: AtomicU64,
}

impl Progress {
    /// Between 0 and 1, or None until the files to copy are counted (or when there are none).
    pub fn fraction(&self) -> Option<f32> {
        let total = self.total.load(Ordering::Relaxed);
        (total > 0).then(|| (self.done.load(Ordering::Relaxed) as f64 / total as f64) as f32)
    }
}

/// What is to be copied: the folders to make and the files, each with its place under the
/// source and its size.
#[derive(Default)]
struct Plan {
    dirs: Vec<PathBuf>,
    files: Vec<(PathBuf, u64)>,
}

fn stopped(cancel: &Cancellation) -> Result<(), String> {
    if cancel.is_stopped() {
        return Err(crate::i18n::tr("The import was stopped.").to_owned());
    }
    Ok(())
}

fn failed(path: &Path, error: io::Error) -> String {
    crate::i18n::trf(
        "Could not import {path}: {error}",
        &[
            ("path", &path.display().to_string()),
            ("error", &error.to_string()),
        ],
    )
}

/// The top-level names of the game folder `options` bring.
fn wanted(options: &CopyOptions) -> Vec<&'static str> {
    let mut names = Vec::new();
    for (on, list) in [
        (options.options, OPTION_FILES),
        (options.worlds, &["saves"][..]),
        (options.resource_packs, &["resourcepacks"][..]),
        (options.shader_packs, &["shaderpacks"][..]),
        (options.screenshots, &["screenshots"][..]),
        (options.servers, &["servers.dat"][..]),
    ] {
        if on {
            names.extend_from_slice(list);
        }
    }
    names
}

/// Collects everything under `from`'s `name` (a file or a folder) into `plan`.
fn collect(from: &Path, name: &str, plan: &mut Plan, cancel: &Cancellation) -> Result<(), String> {
    let mut pending = vec![PathBuf::from(name)];
    while let Some(rel) = pending.pop() {
        stopped(cancel)?;
        let path = from.join(&rel);
        let meta = match path.symlink_metadata() {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(failed(&path, e)),
        };
        if crate::native_cache::linked(&meta) {
            continue;
        }
        if meta.is_file() {
            // A running game's lock on a world: the copy is not that session.
            let is_lock = rel.file_name().is_some_and(|n| n == crate::worlds::LOCK)
                && rel.components().count() == 3;
            if !is_lock {
                plan.files.push((rel, meta.len()));
            }
        } else if meta.is_dir() {
            let children = fs::read_dir(&path).map_err(|e| failed(&path, e))?;
            for child in children {
                let child = child.map_err(|e| failed(&path, e))?;
                pending.push(rel.join(child.file_name()));
            }
            plan.dirs.push(rel);
        }
    }
    Ok(())
}

/// Copies `file` to `to`, which must not exist yet.
fn copy_file(file: &Path, to: &Path, cancel: &Cancellation, progress: &Progress) -> io::Result<()> {
    let mut input = fs::File::open(file)?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(to)?;
    let mut buffer = vec![0u8; 256 << 10];
    loop {
        if cancel.is_stopped() {
            return Err(io::Error::other(crate::i18n::tr("The import was stopped.")));
        }
        let n = input.read(&mut buffer)?;
        if n == 0 {
            return output.flush();
        }
        output.write_all(&buffer[..n])?;
        progress.done.fetch_add(n as u64, Ordering::Relaxed);
    }
}

/// Copies the parts of `from` that `options` keep into `to`, a new profile's game folder.
pub(super) fn copy_into(
    from: &Foreign,
    options: &CopyOptions,
    to: &Path,
    cancel: &Cancellation,
    progress: &Progress,
) -> Result<(), String> {
    let mut plan = Plan::default();
    for name in wanted(options) {
        collect(&from.folder, name, &mut plan, cancel)?;
    }
    let total = plan.files.iter().map(|(_, len)| *len).sum();
    progress.done.store(0, Ordering::Relaxed);
    progress.total.store(total, Ordering::Relaxed);
    for dir in &plan.dirs {
        let target = to.join(dir);
        fs::create_dir_all(&target).map_err(|e| failed(&target, e))?;
    }
    for (rel, _) in &plan.files {
        stopped(cancel)?;
        let target = to.join(rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| failed(parent, e))?;
        }
        // The player's own options start a new profile; the imported ones replace them.
        if rel.components().count() == 1 && OPTION_FILES.iter().any(|n| rel.as_os_str() == *n) {
            let _ = fs::remove_file(&target);
        }
        let source = from.folder.join(rel);
        copy_file(&source, &target, cancel, progress).map_err(|e| failed(&source, e))?;
    }
    stopped(cancel)
}
