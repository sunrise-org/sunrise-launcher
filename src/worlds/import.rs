//! Bringing a world into a profile: a world folder, or a zip of one, copied into its `saves/`.
use super::*;
use crate::i18n::trf;

/// The most folders a zip may hold a world's level.dat inside: at its root, or in one folder.
const DEEPEST_LEVEL: usize = 2;

/// Copies the world at `source` (a folder with a level.dat, or a .zip holding one at its root or
/// inside a single folder) into the profile's `saves/`, under its own name, or "<name> (2)",
/// "<name> (3)"... when that folder is taken. The copy is staged beside the saves and moved in
/// whole, so a failed import leaves nothing behind.
pub fn import(instance: &crate::instances::Instance, source: &Path) -> Result<World, String> {
    let profile = crate::managed_play::Profile::open_selected(instance)?;
    import_into(profile.home(), source)
}

pub(crate) fn import_into(home: &Path, source: &Path) -> Result<World, String> {
    use crate::profile_txn::durable;
    let saves = home.join("saves");
    let failed = |e: &dyn std::fmt::Display| {
        trf(
            "Could not add the world {path}: {error}",
            &[
                ("path", &source.display().to_string()),
                ("error", &e.to_string()),
            ],
        )
    };
    fs::create_dir_all(&saves).map_err(|e| failed(&e))?;
    let meta = fs::symlink_metadata(source).map_err(|e| failed(&e))?;
    if crate::native_cache::linked(&meta) {
        return Err(failed(&io::Error::from(io::ErrorKind::InvalidData)));
    }
    let stage = tempfile::Builder::new()
        .prefix(".world-import-")
        .tempdir_in(&saves)
        .map_err(|e| failed(&e))?;
    let candidate = stage.path().join(".candidate");
    let base = if meta.is_dir() {
        if !has_readable_level_dat(source) {
            return Err(not_a_world(source));
        }
        copy_dir(source, &candidate).map_err(|e| failed(&e))?;
        file_stem(source, false)
    } else if meta.is_file() {
        let folder = unzip_world(source, &candidate).map_err(|e| match e {
            Unzip::NotAWorld => not_a_world(source),
            Unzip::Io(e) => failed(&e),
        })?;
        folder.unwrap_or_else(|| file_stem(source, true))
    } else {
        return Err(not_a_world(source));
    };
    if !has_readable_level_dat(&candidate) {
        return Err(not_a_world(source));
    }
    // The name made safe for every file system, as the stage is empty but for the candidate.
    let safe = free_name(stage.path(), &base, "", 0);
    let taken = saves.join(&safe).exists();
    let folder = if taken {
        free_name(&saves, &safe, "", 2)
    } else {
        safe
    };
    if taken {
        // Two worlds of one name would look the same in the game's list.
        rename_level(&candidate, &folder).map_err(|e| failed(&e))?;
    }
    let to = saves.join(&folder);
    durable::flush_tree(&candidate)
        .and_then(|()| durable::move_new_directory(&candidate, &to))
        .map_err(|e| failed(&e))?;
    read(&to).ok_or_else(|| not_a_world(source))
}

fn not_a_world(source: &Path) -> String {
    trf(
        "{path} is not a Minecraft world: it has no readable level.dat.",
        &[("path", &source.display().to_string())],
    )
}

/// The file name without its .zip when `zip`.
fn file_stem(path: &Path, zip: bool) -> String {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    match name.strip_suffix(".zip").filter(|_| zip) {
        Some(stem) if !stem.is_empty() => stem.to_owned(),
        _ => name.into_owned(),
    }
}

enum Unzip {
    NotAWorld,
    Io(io::Error),
}

impl From<io::Error> for Unzip {
    fn from(e: io::Error) -> Self {
        Unzip::Io(e)
    }
}

/// Extracts the world in the zip at `zip` into `to`: the one whose level.dat is at the zip's root
/// or in a single folder (that folder's name is returned). A folder the zip's tools add on the
/// side (`__MACOSX`) is skipped, and so is the world's session.lock.
fn unzip_world(zip: &Path, to: &Path) -> Result<Option<String>, Unzip> {
    let file = fs::File::open(zip)?;
    let mut archive =
        crate::zip_intake::open(io::BufReader::new(file)).map_err(|_| Unzip::NotAWorld)?;
    // Where level.dat is: the shortest way in wins, so a world holding another world's saved
    // copy deeper down is not taken for it.
    let mut prefix: Option<Vec<String>> = None;
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|_| Unzip::NotAWorld)?;
        let Some(path) = entry.enclosed_name().filter(|_| entry.is_file()) else {
            continue;
        };
        let parts = parts_of(&path);
        if parts.last().map(String::as_str) != Some("level.dat")
            || parts.len() > DEEPEST_LEVEL
            || parts.first().map(String::as_str) == Some("__MACOSX")
        {
            continue;
        }
        if prefix.as_ref().is_none_or(|p| parts.len() - 1 < p.len()) {
            prefix = Some(parts[..parts.len() - 1].to_vec());
        }
    }
    let prefix = prefix.ok_or(Unzip::NotAWorld)?;
    fs::create_dir(to)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(io::Error::other)?;
        let Some(path) = entry.enclosed_name() else {
            continue;
        };
        let parts = parts_of(&path);
        if parts.len() <= prefix.len() || parts[..prefix.len()] != prefix[..] {
            continue;
        }
        let rest = &parts[prefix.len()..];
        if rest.last().map(String::as_str) == Some(LOCK) {
            continue;
        }
        let target = rest.iter().fold(to.to_path_buf(), |p, part| p.join(part));
        if entry.is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let size = entry.size();
            let mut out = fs::File::create(&target)?;
            // Only what the entry says it holds: a lying size can't fill the disk.
            io::copy(&mut (&mut entry).take(size), &mut out)?;
        }
    }
    Ok(prefix.last().cloned())
}

fn parts_of(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|c| match c {
            std::path::Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests;
