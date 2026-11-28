//! Installing builds: the versions folder, downloading, checking and unpacking an archive, and removing.
use super::{
    compare_game_versions, data_dir, dev_date, forget_prepared, get, in_use, installation,
    reuse_prepared_for_cancellable, InstallOutcome, Release,
};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub(crate) fn versions_dir() -> PathBuf {
    data_dir().join("versions")
}

pub fn version_dir(tag: &str) -> PathBuf {
    versions_dir().join(tag)
}

pub(super) const COMPLETE: &str = ".installed";

/// Installed versions, newest tag first.
pub fn installed() -> Vec<String> {
    installed_in(&versions_dir())
}

/// Like [`installed`], stopping early if `cancel` is set while enumerating `versions/`.
pub(crate) fn installed_until(cancel: &crate::storage::Cancel) -> Option<Vec<String>> {
    installed_in_until(&versions_dir(), || cancel.is_cancelled())
}

/// Installed versions and the newest installed Nightly known to be current, for Play.
pub fn play_installations(selected: Option<&str>) -> (Vec<String>, Option<String>) {
    play_installations_until(selected, &crate::storage::Cancel::default())
        .expect("uncancellable play installations")
}

/// Like [`play_installations`], stopping early when `cancel` is set during version discovery.
pub(crate) fn play_installations_until(
    selected: Option<&str>,
    cancel: &crate::storage::Cancel,
) -> Option<(Vec<String>, Option<String>)> {
    if cancel.is_cancelled() {
        return None;
    }
    let versions = versions_dir();
    let installed = installed_in_until(&versions, || cancel.is_cancelled())?;
    if cancel.is_cancelled() {
        return None;
    }
    let nightly = crate::nightly::known_install(&versions, &installed, selected).cloned();
    Some((installed, nightly))
}

/// Accept the current rolling Nightly as offline evidence when it is already installed on
/// the newest date. Do this on the release worker before the window can use that catalogue.
pub fn note_play_catalogue(releases: &[Release]) -> Result<(), String> {
    crate::nightly::note_published(releases);
    let Some(current) = crate::instances::Channel::Nightly.builds(releases).next() else {
        return Ok(());
    };
    let versions = versions_dir();
    if !versions.join(&current.tag).join(COMPLETE).is_file() {
        return Ok(());
    }
    let _publication =
        crate::run_leases::installation().map_err(crate::native_mods::cache_error)?;
    let installed = installed_in(&versions);
    let newest_date = installed
        .iter()
        .find_map(|tag| crate::nightly::date_of(tag));
    if newest_date.is_some()
        && newest_date == crate::nightly::date_of(&current.tag)
        && installed.contains(&current.tag)
    {
        crate::nightly::record_current(&versions, &current.tag).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(crate) fn installed_in(versions: &Path) -> Vec<String> {
    installed_in_until(versions, || false).expect("uncancellable version listing")
}

pub(super) fn installed_in_until(
    versions: &Path,
    cancelled: impl Fn() -> bool,
) -> Option<Vec<String>> {
    if cancelled() {
        return None;
    }
    let mut tags = Vec::new();
    if let Ok(entries) = fs::read_dir(versions) {
        for entry in entries {
            if cancelled() {
                return None;
            }
            let Ok(entry) = entry else {
                continue;
            };
            let Ok(marker) = entry.path().join(COMPLETE).metadata() else {
                continue;
            };
            let Ok(tag) = entry.file_name().into_string() else {
                continue;
            };
            let dev_installed_at = dev_date(&tag).and_then(|_| marker.modified().ok());
            tags.push((tag, dev_installed_at));
        }
    }
    if cancelled() {
        return None;
    }
    // Without the catalogue, same-day Dev builds follow the last local install, not the largest
    // commit hash. Rolling Nightly Play uses its receipt independently of the display order.
    tags.sort_by(|(a, a_installed), (b, b_installed)| {
        compare_game_versions(b, a)
            .then_with(|| b_installed.cmp(a_installed))
            .then_with(|| b.cmp(a))
    });
    Some(tags.into_iter().map(|(tag, _)| tag).collect())
}

/// Downloads `url` into `to`; the file is kept only if its SHA-256 is `sha256`, the signed (or
/// pinned) one. A `size` of 0 takes the server's Content-Length for the progress.
pub(crate) fn download(
    url: &str,
    size: u64,
    sha256: &str,
    to: &Path,
    progress: &impl Fn(f32),
) -> Result<(), String> {
    download_cancellable(url, size, sha256, to, None, &|done, total| {
        progress((done as f32 / total.max(1) as f32).min(1.0) * 0.9);
    })
}

pub(crate) fn download_cancellable(
    url: &str,
    size: u64,
    sha256: &str,
    to: &Path,
    stop: Option<&crate::native_mods::Cancellation>,
    progress: &impl Fn(u64, u64),
) -> Result<(), String> {
    if let Some(path) = crate::draft_releases::local(url)? {
        if let Some(stop) = stop {
            stop.check()?;
        }
        let file = fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let size = file.metadata().map_or(size, |meta| meta.len());
        let name = url.rsplit('/').next().unwrap_or(url);
        let saved = match stop {
            Some(stop) => save_checked_bytes(stop.reader(file), size, name, sha256, to, &|_, _| {}),
            None => save_checked_bytes(file, size, name, sha256, to, &|_, _| {}),
        };
        return saved.map_err(Into::into);
    }
    let Some(stop) = stop else {
        let response = get(url)?;
        let size = match size {
            0 => response
                .header("Content-Length")
                .and_then(|n| n.parse().ok())
                .unwrap_or(0),
            n => n,
        };
        let name = url.rsplit('/').next().unwrap_or(url);
        return save_checked_bytes(response.into_reader(), size, name, sha256, to, progress)
            .map_err(Into::into);
    };
    stop.check()?;
    // Run the blocking HTTP request and body reads on a worker thread so `stop` interrupts a
    // stalled header or body read without adding a transfer timeout.
    let (head_tx, head_rx) = std::sync::mpsc::sync_channel::<Result<u64, String>>(1);
    let (body_tx, body_rx) = std::sync::mpsc::sync_channel::<Result<Vec<u8>, String>>(4);
    let worker_url = url.to_owned();
    let worker_stop = stop.clone();
    std::thread::spawn(move || {
        let response = match get(&worker_url) {
            Ok(response) => response,
            Err(error) => {
                let _ = head_tx.send(Err(error));
                return;
            }
        };
        let resolved_size = match size {
            0 => response
                .header("Content-Length")
                .and_then(|n| n.parse().ok())
                .unwrap_or(0),
            n => n,
        };
        if head_tx.send(Ok(resolved_size)).is_err() {
            return;
        }
        let mut reader = response.into_reader();
        let mut buf = vec![0u8; 1 << 16];
        loop {
            if worker_stop.is_stopped() {
                return;
            }
            match reader.read(&mut buf) {
                Ok(0) => return,
                Ok(n) => {
                    if body_tx.send(Ok(buf[..n].to_vec())).is_err() {
                        return;
                    }
                }
                Err(error) => {
                    let _ = body_tx.send(Err(error.to_string()));
                    return;
                }
            }
        }
    });
    let size = loop {
        stop.check()?;
        match head_rx.recv_timeout(std::time::Duration::from_millis(50)) {
            Ok(Ok(resolved_size)) => break resolved_size,
            Ok(Err(error)) => {
                stop.check()?;
                return Err(error);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                stop.check()?;
                return Err(crate::i18n::tr("Download worker stopped unexpectedly").into());
            }
        }
    };
    struct InterruptibleReader<'a> {
        rx: std::sync::mpsc::Receiver<Result<Vec<u8>, String>>,
        stop: &'a crate::native_mods::Cancellation,
        buf: Vec<u8>,
        pos: usize,
    }
    impl Read for InterruptibleReader<'_> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            while self.pos >= self.buf.len() {
                self.stop.check().map_err(std::io::Error::other)?;
                match self.rx.recv_timeout(std::time::Duration::from_millis(50)) {
                    Ok(Ok(chunk)) => {
                        self.buf = chunk;
                        self.pos = 0;
                    }
                    Ok(Err(error)) => return Err(std::io::Error::other(error)),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(0),
                }
            }
            let n = (self.buf.len() - self.pos).min(out.len());
            out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }
    let name = url.rsplit('/').next().unwrap_or(url);
    let reader = InterruptibleReader {
        rx: body_rx,
        stop,
        buf: Vec::new(),
        pos: 0,
    };
    if let Err(error) = save_checked_bytes(reader, size, name, sha256, to, progress) {
        stop.check()?;
        return Err(error.into());
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) enum SaveCheckedError {
    Local(std::io::Error),
    Download(String),
}

impl From<SaveCheckedError> for String {
    fn from(error: SaveCheckedError) -> Self {
        match error {
            SaveCheckedError::Local(error) => error.to_string(),
            SaveCheckedError::Download(error) => error,
        }
    }
}

/// Saves `reader` as `to` once all of it has arrived and its SHA-256 is `sha256`: until then it
/// has a temporary name beside `to`, so `to` is never a partial or unchecked file.
pub(crate) fn save_checked(
    reader: impl Read,
    size: u64,
    name: &str,
    sha256: &str,
    to: &Path,
    progress: &impl Fn(f32),
) -> Result<(), SaveCheckedError> {
    save_checked_bytes(reader, size, name, sha256, to, &|done, total| {
        progress((done as f32 / total.max(1) as f32).min(1.0) * 0.9);
    })
}

pub(crate) fn save_checked_bytes(
    reader: impl Read,
    size: u64,
    name: &str,
    sha256: &str,
    to: &Path,
    progress: &impl Fn(u64, u64),
) -> Result<(), SaveCheckedError> {
    let mut temp = to.as_os_str().to_owned();
    temp.push(format!(".part-{}", std::process::id()));
    let temp = PathBuf::from(temp);
    let saved = write_checked(reader, size, name, sha256, &temp, progress)
        .and_then(|()| fs::rename(&temp, to).map_err(SaveCheckedError::Local));
    if saved.is_err() {
        let _ = fs::remove_file(&temp);
    }
    saved
}

fn write_checked(
    mut reader: impl Read,
    size: u64,
    name: &str,
    sha256: &str,
    to: &Path,
    progress: &impl Fn(u64, u64),
) -> Result<(), SaveCheckedError> {
    let mut file = fs::File::create(to).map_err(SaveCheckedError::Local)?;
    let mut hash = sunrise_launcher_release_verify::Sha256::default();
    let (mut done, mut buf) = (0u64, vec![0u8; 1 << 16]);
    loop {
        let n = reader.read(&mut buf).map_err(|e| {
            if crate::native_mods::is_cancelled_io(&e) {
                SaveCheckedError::Download(crate::native_mods::CANCELLED.into())
            } else {
                SaveCheckedError::Download(crate::i18n::trf(
                    "Download interrupted: {error}",
                    &[("error", &e.to_string())],
                ))
            }
        })?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(SaveCheckedError::Local)?;
        done += n as u64;
        progress(done, size);
    }
    sunrise_launcher_release_verify::check(name, sha256, &hash.hex())
        .map_err(SaveCheckedError::Download)
}

pub(super) fn unpack(
    archive: &Path,
    url: &str,
    to: &Path,
    stop: Option<&crate::native_mods::Cancellation>,
) -> Result<(), String> {
    if let Some(stop) = stop {
        stop.check()?;
    }
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    let result = if url.ends_with(".tar.gz") {
        match stop {
            Some(stop) => tar::Archive::new(flate2::read::GzDecoder::new(stop.reader(file)))
                .unpack(to)
                .map_err(|e| e.to_string()),
            None => tar::Archive::new(flate2::read::GzDecoder::new(file))
                .unpack(to)
                .map_err(|e| e.to_string()),
        }
    } else {
        // The library's extractor keeps each entry's Unix mode and symlinks (a macOS bundle's
        // second executable, `sunrise-prepare`, must stay runnable); Stop is checked on every
        // read of the archive underneath it.
        zip::ZipArchive::new(CancellableFile { file, stop })
            .and_then(|mut archive| archive.extract(to))
            .map_err(|e| e.to_string())
    };
    if let Err(error) = result {
        if let Some(stop) = stop {
            stop.check()?;
        }
        return Err(error);
    }
    Ok(())
}

/// The downloaded archive, failing its next read once `stop` is set.
struct CancellableFile<'a> {
    file: fs::File,
    stop: Option<&'a crate::native_mods::Cancellation>,
}

impl Read for CancellableFile<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if let Some(stop) = self.stop {
            stop.check().map_err(std::io::Error::other)?;
        }
        self.file.read(buf)
    }
}

impl std::io::Seek for CancellableFile<'_> {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.file.seek(pos)
    }
}

/// Installs one signed release or reuses its proved rolling Nightly image. Only a retry of
/// a known receipt failure skips the normal post-install actions.
pub fn install(release: &Release, progress: impl Fn(f32)) -> Result<InstallOutcome, String> {
    install_cancellable(release, None, |phase| {
        if let super::InstallPhase::Downloading(fraction) = phase {
            progress(fraction);
        }
    })
}

pub fn install_cancellable(
    release: &Release,
    stop: Option<&crate::native_mods::Cancellation>,
    progress: impl Fn(super::InstallPhase),
) -> Result<InstallOutcome, String> {
    if let Some(stop) = stop {
        stop.check()?;
    }
    progress(super::InstallPhase::Checking);
    let tag = &release.tag;
    installation::recover_replacement(tag)?;
    let download = installation::GameDownload::select(release)?;
    let proof = download.prove_cancellable(tag, stop)?;
    let outcome = download.repair_cancellable(tag, &proof, stop, &progress)?;
    if matches!(outcome, InstallOutcome::Installed(_)) {
        progress(super::InstallPhase::Preparing);
        // Preparation has its own admission. Never hold the cache publication lock here;
        // managed First Play instead prepares through its profile-owned staged handoff.
        reuse_prepared_for_cancellable(tag, stop);
        if let Some(stop) = stop {
            stop.check()?;
        }
    }
    Ok(outcome)
}

/// The build's facts a launcher reads first (sunrise-org/sunrise's docs/api/release-file.md), signed beside its archives.
pub(super) const RELEASE_FILE: &str = "sunrise-release.json";

/// Keeps `bytes` as the installed build's [`RELEASE_FILE`] in `dir` when they are the signed `sha256`'s.
pub(super) fn keep_release_file(dir: &Path, bytes: &[u8], sha256: &str) -> Result<(), String> {
    if sunrise_launcher_release_verify::sha256_hex(bytes) != sha256 {
        return Err(crate::i18n::trf(
            "{file}: not the signed file",
            &[("file", RELEASE_FILE)],
        ));
    }
    let file = dir.join(RELEASE_FILE);
    match fs::File::create_new(&file) {
        Ok(mut file) => file.write_all(bytes),
        Err(error)
            if error.kind() == std::io::ErrorKind::AlreadyExists
                && fs::symlink_metadata(&file)
                    .is_ok_and(|meta| meta.is_file() && !crate::native_cache::linked(&meta))
                && fs::read(&file).is_ok_and(|existing| existing == bytes) =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
    .map_err(|e| format!("{RELEASE_FILE}: {e}"))
}

/// The asset `release` installs from: the fallback when [`game_releases_with`] took it (its file
/// name ends the download address), so the signed checksum checked is that file's.
pub(super) fn asset_installed<'a>(
    release: &Release,
    asset: Option<&'a str>,
    fallback: Option<&'a str>,
) -> Option<&'a str> {
    let file = release
        .asset
        .as_ref()
        .and_then(|(url, _)| url.rsplit('/').next());
    if fallback.is_some_and(|name| file == Some(name)) {
        fallback
    } else if asset.is_some_and(|name| file == Some(name)) {
        asset
    } else {
        None
    }
}

pub fn remove(tag: &str) -> Result<(), String> {
    remove_observed(tag, || {})
}

pub(crate) fn remove_observed(tag: &str, selected: impl FnOnce()) -> Result<(), String> {
    let dir = version_dir(tag);
    let idle = !in_use(tag);
    selected();
    crate::cache_cleanup::delete_version(&dir, idle).map_err(crate::native_mods::cache_error)?;
    forget_prepared();
    Ok(())
}
