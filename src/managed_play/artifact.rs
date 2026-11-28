//! Exact identity of an installed executable and the files it reads beside itself.
use super::{digest_file, plain_file, require_dir, unsettled, validate_tag};
use crate::{
    backend,
    native_mods::{sdk_cache::GameSlices, Cancellation},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read as _},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Artifact {
    pub(super) tag: String,
    executable: [u8; 32],
    runtime: [u8; 32],
}

impl Artifact {
    pub(super) fn capture(tag: &str) -> Result<Self, String> {
        validate_tag(tag)?;
        Self::capture_at(tag, &backend::version_dir(tag))
    }

    pub(crate) fn capture_at(tag: &str, dir: &Path) -> Result<Self, String> {
        Self::installed(dir, tag, None)
    }

    pub(crate) const ARCHIVE_RECEIPT: &str = ".sunrise-archive.json";
    pub(crate) const SDK_RECEIPT: &str = ".sunrise-sdk.json";

    /// Record the extracted image before its cache name and completion marker are published.
    pub(crate) fn record_archive(dir: &Path, tag: &str, archive: &str) -> Result<(), String> {
        let image = Self::image(dir, tag, Some(archive), None)?;
        let path = dir.join(Self::ARCHIVE_RECEIPT);
        let file = fs::File::create_new(&path).map_err(|error| at(&path, error))?;
        serde_json::to_writer(
            file,
            &ArchiveReceipt {
                archive: archive.to_owned(),
                image,
            },
        )
        .map_err(|error| at(&path, error.into()))
    }

    pub(crate) fn record_sdk(dir: &Path, tag: &str, sdk: GameSlices) -> Result<(), String> {
        let image = Self::image(dir, tag, None, Some(&sdk))?;
        let path = dir.join(Self::SDK_RECEIPT);
        let file = fs::File::create_new(&path).map_err(|error| at(&path, error))?;
        serde_json::to_writer(file, &SdkReceipt { sdk, image })
            .map_err(|error| at(&path, error.into()))
    }

    /// Accept a deferred archive only when its digest and the current executable/runtime
    /// match the image recorded after verifying and extracting that archive.
    pub(crate) fn matches_archive(dir: &Path, tag: &str, archive: &str) -> bool {
        Self::from_archive(dir, tag, archive).is_ok()
    }

    pub(crate) fn from_archive(dir: &Path, tag: &str, archive: &str) -> Result<Self, String> {
        Self::installed(dir, tag, Some(Source::Archive(archive)))
    }

    pub(crate) fn from_sdk(dir: &Path, tag: &str, index: &str) -> Result<Self, String> {
        Self::installed(dir, tag, Some(Source::Sdk(index)))
    }

    fn installed(dir: &Path, tag: &str, expected: Option<Source<'_>>) -> Result<Self, String> {
        require_dir(dir)?;
        let complete = dir.join(".installed");
        if super::open_if_file(&complete)?.is_none() {
            return Err(at(&complete, io::ErrorKind::NotFound.into()));
        }
        let path = dir.join(Self::ARCHIVE_RECEIPT);
        let archive: Option<ArchiveReceipt> = receipt(&path)?;
        let sdk: Option<SdkReceipt> = receipt(&dir.join(Self::SDK_RECEIPT))?;
        let (source, saved) = match (&archive, &sdk) {
            (Some(_), Some(_)) => return Err(unsettled!()),
            (Some(receipt), None) => (
                Some(Source::Archive(&receipt.archive)),
                Some(&receipt.image),
            ),
            (None, Some(receipt)) => (Some(Source::Sdk(&receipt.sdk.index)), Some(&receipt.image)),
            (None, None) => (None, None),
        };
        if expected.is_some_and(|expected| source != Some(expected))
            || source.is_some_and(|source| {
                let (Source::Archive(digest) | Source::Sdk(digest)) = source;
                !backend::installation::valid_archive_digest(digest)
            })
        {
            return Err(unsettled!());
        }
        let image = Self::image(
            dir,
            tag,
            archive.as_ref().map(|receipt| receipt.archive.as_str()),
            sdk.as_ref().map(|receipt| &receipt.sdk),
        )?;
        if saved.is_some_and(|saved| *saved != image) {
            return Err(unsettled!());
        }
        Ok(image)
    }

    fn image(
        dir: &Path,
        tag: &str,
        archive: Option<&str>,
        sdk: Option<&GameSlices>,
    ) -> Result<Self, String> {
        let path = dir.join(backend::CLIENT);
        let Some(executable) = digest_file(&path)? else {
            return Err(at(&path, io::ErrorKind::NotFound.into()));
        };
        let mut hash = Sha256::new();
        item(
            &mut hash,
            Self::ARCHIVE_RECEIPT.as_bytes(),
            archive.map(|archive| Sha256::digest(archive).into()),
        );
        if let Some(sdk) = sdk {
            let bytes = serde_json::to_vec(sdk).map_err(|error| error.to_string())?;
            item(
                &mut hash,
                Self::SDK_RECEIPT.as_bytes(),
                Some(Sha256::digest(bytes).into()),
            );
        }
        // Keep existing static receipts and Play journals valid when there is no deps/ tree.
        if let Some(deps) = tree(&dir.join("deps"), false)? {
            item(&mut hash, b"deps", Some(deps));
        }
        for file in [
            backend::GAME_BUILD_ARCHITECTURE,
            "sunrise-release.json",
            "research-lock.json",
            "native-mods.json",
            crate::preparer::BINARY,
        ] {
            item(&mut hash, file.as_bytes(), digest_file(&dir.join(file))?);
        }
        for name in [
            "native-data",
            "native-content",
            sunrise_launcher_smod::mod_list::NATIVE_DIR,
        ] {
            item(&mut hash, name.as_bytes(), tree(&dir.join(name), false)?);
        }
        for entry in entries(dir)? {
            let name = entry.file_name();
            let lower = name.to_string_lossy().to_ascii_lowercase();
            if lower.ends_with(".dll")
                || lower.ends_with(".dylib")
                || lower.ends_with(".so")
                || lower.contains(".so.")
            {
                let path = entry.path();
                let image = digest_file(&path)?;
                if image.is_none() {
                    return Err(at(&path, io::ErrorKind::NotFound.into()));
                }
                item(&mut hash, name.as_encoded_bytes(), image);
            }
        }
        Ok(Self {
            tag: tag.into(),
            executable,
            runtime: hash.finalize().into(),
        })
    }

    pub(super) fn verify_program(&self, path: &Path) -> Result<(), String> {
        if digest_file(path)? != Some(self.executable) {
            return Err(unsettled!());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchiveReceipt {
    archive: String,
    image: Artifact,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Source<'a> {
    Archive(&'a str),
    Sdk(&'a str),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SdkReceipt {
    sdk: GameSlices,
    image: Artifact,
}

fn receipt<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    super::optional_bytes(path)?
        .map(|bytes| serde_json::from_slice(&bytes).map_err(|error| at(path, error.into())))
        .transpose()
}

pub(super) fn tree(path: &Path, resources: bool) -> Result<Option<[u8; 32]>, String> {
    Ok(tree_with_workers(path, resources, super::pool::workers(), None)?.map(|(digest, _)| digest))
}

/// What a directory holds, in the order its digest folds it: files by the index of their
/// own digest in the list the workers hash.
enum Node {
    Dir(Vec<(std::ffi::OsString, Node)>),
    File(usize),
}

/// [`tree`] on `workers` threads, with the number of files it hashed. The walk and the fold
/// run in the one fixed order of the serial digest; only the file hashes run in parallel.
pub(super) fn tree_with_workers(
    path: &Path,
    resources: bool,
    workers: usize,
    stop: Option<&Cancellation>,
) -> Result<Option<([u8; 32], usize)>, String> {
    tree_with_progress(path, resources, workers, stop, None)
}

pub(super) fn tree_with_progress(
    path: &Path,
    resources: bool,
    workers: usize,
    stop: Option<&Cancellation>,
    on_done: Option<&(dyn Fn(usize, usize) + Sync)>,
) -> Result<Option<([u8; 32], usize)>, String> {
    let device = match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && plain_file(&meta) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                meta.dev()
            }
            #[cfg(not(unix))]
            {
                0
            }
        }
        Ok(_) => return Err(at(path, io::ErrorKind::InvalidData.into())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(at(path, e)),
    };
    let mut files = Vec::new();
    let root = directory(path, 0, device, resources, stop, &mut files)?;
    let digests = super::pool::run_with_progress(
        &files,
        workers,
        stop,
        on_done,
        || vec![0u8; 64 * 1024],
        |buffer, file| digest_open(file, buffer),
    )?;
    Ok(Some((fold(&root, &digests), files.len())))
}

fn directory(
    path: &Path,
    depth: usize,
    _device: u64,
    resources: bool,
    stop: Option<&Cancellation>,
    files: &mut Vec<PathBuf>,
) -> Result<Vec<(std::ffi::OsString, Node)>, String> {
    if depth > 64 {
        return Err(at(path, io::ErrorKind::InvalidData.into()));
    }
    if let Some(stop) = stop {
        stop.check()?;
    }
    let mut nodes = Vec::new();
    for entry in entries(path)? {
        let name = entry.file_name();
        if depth == 0 && resources && super::resources::generated_cache(&name) {
            continue;
        }
        let entry_path = entry.path();
        let meta = fs::symlink_metadata(&entry_path).map_err(|e| at(&entry_path, e))?;
        if !plain_file(&meta) {
            return Err(at(&entry_path, io::ErrorKind::InvalidData.into()));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if _device != meta.dev() {
                return Err(at(&entry_path, io::ErrorKind::InvalidData.into()));
            }
        }
        let node = if meta.is_dir() {
            Node::Dir(directory(
                &entry_path,
                depth + 1,
                _device,
                resources,
                stop,
                files,
            )?)
        } else if meta.is_file() {
            files.push(entry_path);
            Node::File(files.len() - 1)
        } else {
            return Err(at(&entry_path, io::ErrorKind::InvalidData.into()));
        };
        nodes.push((name, node));
    }
    Ok(nodes)
}

/// Hash one file through the handle it was opened as. The walk already refused links with
/// `symlink_metadata`; checking the open handle again covers the file being swapped since.
fn digest_open(path: &PathBuf, buffer: &mut [u8]) -> Result<[u8; 32], String> {
    let mut file = fs::File::open(path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => at(path, io::ErrorKind::NotFound.into()),
        _ => at(path, e),
    })?;
    let meta = file.metadata().map_err(|e| at(path, e))?;
    if !meta.is_file() || !plain_file(&meta) {
        return Err(at(path, io::ErrorKind::InvalidData.into()));
    }
    let mut hash = Sha256::new();
    loop {
        match file.read(buffer) {
            Ok(0) => break,
            Ok(n) => hash.update(&buffer[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(at(path, e)),
        }
    }
    Ok(hash.finalize().into())
}

fn fold(nodes: &[(std::ffi::OsString, Node)], digests: &[[u8; 32]]) -> [u8; 32] {
    let mut hash = Sha256::new();
    for (name, node) in nodes {
        let image = match node {
            Node::Dir(children) => {
                hash.update(b"d");
                fold(children, digests)
            }
            Node::File(index) => {
                hash.update(b"f");
                digests[*index]
            }
        };
        item(&mut hash, name.as_encoded_bytes(), Some(image));
    }
    hash.finalize().into()
}

fn entries(path: &Path) -> Result<Vec<fs::DirEntry>, String> {
    let read = || fs::read_dir(path)?.collect::<io::Result<Vec<_>>>();
    let mut entries = read().map_err(|e| at(path, e))?;
    entries.sort_unstable_by_key(fs::DirEntry::file_name);
    Ok(entries)
}

fn item(hash: &mut Sha256, name: &[u8], image: Option<[u8; 32]>) {
    hash.update((name.len() as u64).to_le_bytes());
    hash.update(name);
    match image {
        None => hash.update([0]),
        Some(bytes) => {
            hash.update([1]);
            hash.update(bytes);
        }
    }
}

fn at(path: &Path, error: io::Error) -> String {
    format!("{}: {error}", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_covers_library_native_payload_and_architecture_but_not_logs() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        fs::write(dir.join(backend::CLIENT), b"game").unwrap();
        fs::write(dir.join(".installed"), []).unwrap();
        fs::create_dir(dir.join("native-data")).unwrap();
        fs::create_dir_all(dir.join("deps/nested")).unwrap();
        let paths = [
            "native.dll",
            "deps/nested/libsdk.so",
            "native-data/data.json",
            backend::GAME_BUILD_ARCHITECTURE,
        ];
        for path in paths {
            fs::write(dir.join(path), b"original").unwrap();
        }
        let capture = || Artifact::capture_at("tag", dir).unwrap();
        let sealed = capture();
        fs::write(dir.join("latest.log"), b"mutable game log").unwrap();
        assert_eq!(sealed, capture());
        for path in paths {
            fs::write(dir.join(path), b"different").unwrap();
            assert_ne!(sealed, capture(), "{path}");
            fs::write(dir.join(path), b"original").unwrap();
            assert_eq!(sealed, capture(), "{path}");
        }
        fs::write(dir.join("extra.so.1"), b"extra library").unwrap();
        assert_ne!(sealed, capture());
    }

    /// The digest v1.3.4's serial `artifact::tree` gave this fixture, computed once from that
    /// algorithm and committed so no later change can move it.
    const GOLDEN: &str = "5d3eb036328857245589db22a45054920121f1b26a9690ff94b2759ac8189d07";
    #[cfg(target_os = "linux")]
    const GOLDEN_CASE: &str = "1a08ece88d725eca4c7b167f093c102a908c1e542dde2e9850d1089d2ad46af2";

    fn hex(digest: [u8; 32]) -> String {
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn fixture(dir: &Path) {
        for sub in ["empty-dir", "nested/deep/deeper", "nested/a/b"] {
            fs::create_dir_all(dir.join(sub)).unwrap();
        }
        let big: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        for (path, bytes) in [
            ("Alpha.txt", &b"alpha-data"[..]),
            ("Zebra.txt", b"zebra"),
            ("beta.txt", b""),
            ("caf\u{e9}.txt", b"latte"),
            ("nested/deep/deeper/leaf.bin", &big),
            ("nested/empty.txt", b""),
            ("nested/a/b/file.txt", b"x"),
        ] {
            fs::write(dir.join(path), bytes).unwrap();
        }
    }

    #[test]
    fn tree_digest_is_the_serial_v1_3_4_digest_for_any_worker_count() {
        let temp = tempfile::tempdir().unwrap();
        fixture(temp.path());
        for workers in [1, 2, 5] {
            let (digest, files) = tree_with_workers(temp.path(), false, workers, None)
                .unwrap()
                .unwrap();
            assert_eq!(hex(digest), GOLDEN, "{workers} workers");
            assert_eq!(files, 7);
        }
        assert_eq!(hex(tree(temp.path(), false).unwrap().unwrap()), GOLDEN);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn names_differing_only_in_case_keep_their_byte_order() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("Same.txt"), b"upper").unwrap();
        fs::write(temp.path().join("same.txt"), b"lower").unwrap();
        for workers in [1, 2] {
            let (digest, _) = tree_with_workers(temp.path(), false, workers, None)
                .unwrap()
                .unwrap();
            assert_eq!(hex(digest), GOLDEN_CASE);
        }
    }

    #[test]
    fn the_generated_cache_is_skipped_only_at_the_top_when_asked() {
        let temp = tempfile::tempdir().unwrap();
        fixture(temp.path());
        fs::write(temp.path().join("sunrise-data.bin"), b"cache").unwrap();
        fs::write(temp.path().join("sunrise-data.bin.42.tmp"), b"cache").unwrap();
        assert_eq!(hex(tree(temp.path(), true).unwrap().unwrap()), GOLDEN);
        assert_ne!(hex(tree(temp.path(), false).unwrap().unwrap()), GOLDEN);
        fs::write(temp.path().join("nested/sunrise-data.bin"), b"cache").unwrap();
        assert_ne!(hex(tree(temp.path(), true).unwrap().unwrap()), GOLDEN);
    }

    #[test]
    fn a_missing_root_is_none_and_too_deep_a_tree_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(tree(&temp.path().join("absent"), false), Ok(None));
        let mut deep = temp.path().to_path_buf();
        for _ in 0..66 {
            deep.push("d");
        }
        fs::create_dir_all(&deep).unwrap();
        for workers in [1, 2] {
            assert!(tree_with_workers(temp.path(), false, workers, None).is_err());
        }
    }

    #[test]
    fn a_stopped_attempt_hashes_nothing() {
        let temp = tempfile::tempdir().unwrap();
        fixture(temp.path());
        let stop = Cancellation::default();
        stop.stop();
        let error = tree_with_workers(temp.path(), false, 2, Some(&stop)).unwrap_err();
        assert_eq!(Some(error), stop.check().err());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_in_the_middle_of_the_tree_is_refused_by_every_worker_count() {
        let temp = tempfile::tempdir().unwrap();
        fixture(temp.path());
        let link = temp.path().join("nested/deep/link.txt");
        std::os::unix::fs::symlink(temp.path().join("Alpha.txt"), &link).unwrap();
        let expected = at(&link, io::ErrorKind::InvalidData.into());
        for workers in [1, 2] {
            assert_eq!(
                tree_with_workers(temp.path(), false, workers, None),
                Err(expected.clone())
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_file_in_the_middle_of_the_tree_is_an_error_not_a_hang() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        fixture(temp.path());
        let locked = temp.path().join("nested/a/b/file.txt");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o0)).unwrap();
        if fs::File::open(&locked).is_ok() {
            return; // Running as a user the mode does not bind.
        }
        for workers in [1, 2] {
            let error = tree_with_workers(temp.path(), false, workers, None).unwrap_err();
            assert!(error.starts_with(&locked.display().to_string()), "{error}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_hard_linked_file_is_hashed_as_it_was_before_the_pool() {
        let temp = tempfile::tempdir().unwrap();
        fixture(temp.path());
        let before = tree(temp.path(), false).unwrap().unwrap();
        fs::remove_file(temp.path().join("nested/empty.txt")).unwrap();
        fs::hard_link(
            temp.path().join("beta.txt"),
            temp.path().join("nested/empty.txt"),
        )
        .unwrap();
        assert_eq!(tree(temp.path(), false).unwrap().unwrap(), before);
    }
}
