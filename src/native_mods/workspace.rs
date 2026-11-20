//! SDK inputs and writable paths for one native compiler attempt, never reused by a retry.
use super::{sdk_cache::Package, tool, Path};
use std::{collections::BTreeSet, fs, io};

pub(super) struct Workspace(tempfile::TempDir);

impl Workspace {
    pub(super) fn new(parent: &Path) -> io::Result<Self> {
        let parent = crate::backend::plain(parent.canonicalize()?);
        tempfile::Builder::new()
            .prefix(".native-attempt-")
            .rand_bytes(24)
            .tempdir_in(parent)
            .map(Self)
    }

    pub(super) fn path(&self) -> &Path {
        self.0.path()
    }

    pub(super) fn isolate(&self, command: &mut std::process::Command) -> io::Result<()> {
        let path = self.path();
        for dir in ["cargo", "target", "tmp"] {
            fs::create_dir_all(path.join(dir))?;
        }
        let tmp = path.join("tmp");
        command
            .env("CARGO_HOME", path.join("cargo"))
            .env("CARGO_TARGET_DIR", path.join("target"))
            .env("TMPDIR", &tmp)
            .env("TMP", &tmp)
            .env("TEMP", &tmp);
        Ok(())
    }

    /// From here on an escaped descendant can outlive the launcher or SDK. It can keep its
    /// private paths; ordinary retry and recovery must not unlink or reuse them.
    pub(super) fn allow_worker(&mut self) {
        self.0.disable_cleanup(true);
    }

    /// Windows compiler success proves the kernel job was empty at SDK exit.
    #[cfg(windows)]
    pub(super) fn worker_finished(&mut self) {
        self.0.disable_cleanup(false);
    }

    /// Copy the installed SDK inputs, never giving a compiler links into the shared cache.
    pub(super) fn package(&self, package: &Package) -> io::Result<Package> {
        let root = self.path().join("package");
        fs::create_dir(&root)?;
        let private = Package::at(&root);
        for (from, to) in [
            (&package.runtime, &private.runtime),
            (&package.api, &private.api),
            (&package.engine, &private.engine),
        ] {
            match fs::symlink_metadata(from) {
                Ok(meta) if meta.is_dir() && !crate::native_cache::linked(&meta) => {
                    copy_slice(from, to)?;
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(err),
                Ok(_) => return Err(invalid()),
            }
        }
        Ok(private)
    }

    pub(super) fn import_log(&self) -> io::Result<String> {
        let work = self.path().join("work");
        let log = work.join("build.log");
        match regular_under(self.path(), &log) {
            Ok(()) => Ok(tool::tail(fs::File::open(log)?)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(String::new()),
            Err(err) => Err(err),
        }
    }

    /// The API route reuses the game's executable and may return runtime libraries from the SDK
    /// slice. Managed Rust and MinGW can also supply their own libraries. No other source path in
    /// a reply authorizes the launcher to publish that file.
    pub(super) fn verify_outputs<'a>(
        &self,
        built: &tool::Built,
        package: &Package,
        rust: &crate::rust_toolchain::Toolchain,
        ids: impl Iterator<Item = &'a str>,
    ) -> io::Result<()> {
        let work = self.path().join("work");
        if regular_under(&work, &built.binary).is_err()
            && (built.binary != package.runtime.join("bin").join(crate::backend::CLIENT)
                || regular_under(&package.runtime, &built.binary).is_err())
        {
            return Err(invalid());
        }
        let used: BTreeSet<_> = built.mods.iter().map(String::as_str).collect();
        if used.len() != built.mods.len() || used != ids.collect() || built.libraries.is_empty() {
            return Err(invalid());
        }
        let std_lib = rust.target_libdir();
        let mingw = rust.mingw_bin();
        let mut filenames = BTreeSet::new();
        for file in &built.libraries {
            let name = file
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(invalid)?;
            if ![".so", ".dll", ".dylib"]
                .iter()
                .any(|suffix| name.ends_with(suffix))
                || !filenames.insert(name.to_ascii_lowercase())
            {
                return Err(invalid());
            }
            let runtime = || regular_under(&package.runtime, file).is_ok();
            let standard = || {
                (name.starts_with("libstd-") || name.starts_with("std-"))
                    && std_lib.as_ref().is_some_and(|dir| {
                        file == &dir.join(name) && regular_under(dir, file).is_ok()
                    })
            };
            let gnu = || {
                matches!(name, "libgcc_s_seh-1.dll" | "libwinpthread-1.dll")
                    && file == &mingw.join(name)
                    && regular_under(&mingw, file).is_ok()
            };
            if regular_under(&work, file).is_err() && !runtime() && !standard() && !gnu() {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

fn copy_slice(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let meta = fs::symlink_metadata(entry.path())?;
        if crate::native_cache::linked(&meta) {
            return Err(invalid());
        }
        let to = to.join(entry.file_name());
        if meta.is_dir() {
            copy_slice(&entry.path(), &to)?;
        } else if meta.is_file() {
            crate::preparer::clone_or_copy(&entry.path(), &to)?;
        } else {
            return Err(invalid());
        }
    }
    Ok(())
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid native SDK output")
}

// Refuse reparse points and links at every component, including the root. The compiler never
// receives the installation destination; the launcher copies these bytes out of the work tree.
fn regular_under(root: &Path, file: &Path) -> io::Result<()> {
    let tail = file.strip_prefix(root).map_err(|_| invalid())?;
    let mut path = root.to_path_buf();
    let mut parts = tail.components().peekable();
    if parts.peek().is_none() {
        return Err(invalid());
    }
    let meta = fs::symlink_metadata(root)?;
    if !meta.is_dir() || crate::native_cache::linked(&meta) {
        return Err(invalid());
    }
    while let Some(std::path::Component::Normal(part)) = parts.next() {
        path.push(part);
        let meta = fs::symlink_metadata(&path)?;
        if crate::native_cache::linked(&meta) {
            return Err(invalid());
        }
        if parts.peek().is_none() {
            return if meta.is_file() {
                Ok(())
            } else {
                Err(invalid())
            };
        }
        if !meta.is_dir() {
            return Err(invalid());
        }
    }
    Err(invalid())
}

#[cfg(test)]
mod tests;
