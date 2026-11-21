//! Check a catalog pin's exact source bytes; never load or run repository code.
use super::*;
use crate::{
    browser::sunrise::sources::{self, Failure, SourceCommit, Tracker},
    native_mods::Cancellation,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceOrigin {
    pub repo_id: u64,
    pub manifest: String,
    pub track: SourceTrack,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceTrack {
    Branch(String),
    Release,
    Pinned,
}

impl SourceOrigin {
    pub(super) fn validate(&self) -> bool {
        self.repo_id != 0
            && safe_manifest(&self.manifest)
            && match &self.track {
                SourceTrack::Branch(branch) => ref_ok(branch) && self.published_at.is_none(),
                SourceTrack::Release => self.published_at.as_deref().and_then(date).is_some(),
                SourceTrack::Pinned => self.published_at.is_none(),
            }
    }
}

fn date(value: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::parse_from_rfc3339(value).ok()
}

fn safe_manifest(path: &str) -> bool {
    path.split('/').all(|part| {
        !matches!(part, "" | "." | "..")
            && !part
                .chars()
                .any(|c| c.is_control() || matches!(c, '\\' | '%' | ':' | '?' | '#'))
    }) && path.rsplit('/').next() == Some(sunrise_launcher_smod::manifest::MANIFEST)
}

fn archive_at(
    owner: &str,
    repo: &str,
    commit: &str,
    cancellation: &Cancellation,
) -> Result<(std::sync::Arc<tempfile::TempDir>, PathBuf), Failure> {
    let checked = Failure::Other;
    cancellation.check().map_err(checked)?;
    let scratch = std::sync::Arc::new(tempfile::tempdir().map_err(|e| checked(e.to_string()))?);
    let directory = scratch.path().join("source");
    let top = if let Some(fixture) = fixture_repo(owner, repo) {
        // The only local transport is an isolated debug fixture. Archive by object identity;
        // do not check out, run hooks, inspect the mutable working tree, or execute any mod.
        let archive = crate::process::command("git")
            .arg("-C")
            .arg(fixture)
            .args(["archive", "--format=tar", "--prefix=source/", commit])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| checked(e.to_string()))?;
        let mut archive = archive;
        let result = unpack_bounded(
            cancellation.reader(archive.stdout.take().expect("piped archive")),
            &directory,
            MAX_ARCHIVE,
            Some(cancellation),
        );
        if result.is_err() {
            let _ = archive.kill();
        }
        let status = archive.wait().map_err(|e| checked(e.to_string()))?;
        let top = result.map_err(checked)?;
        if !status.success() {
            return Err(Failure::RefUnavailable);
        }
        top
    } else {
        let body = sources::github_archive(&format!("{owner}/{repo}"), commit)?;
        unpack_bounded(
            cancellation.reader(flate2::read::GzDecoder::new(body)),
            &directory,
            MAX_ARCHIVE,
            Some(cancellation),
        )
        .map_err(checked)?
    };
    Ok((scratch, top))
}

/// Take a pinned snapshot by the catalog's manifest path, not the first ID in the repo.
pub(crate) fn pack_pin(
    pin: &SourceCommit,
    cancellation: &Cancellation,
) -> Result<(Packed, Origin), Failure> {
    let source = pin.source();
    let (owner, repo) = source
        .repo()
        .split_once('/')
        .expect("validated source repository");
    let (scratch, top) = archive_at(owner, repo, pin.sha(), cancellation)?;
    let checked = Failure::Other;
    cancellation.check().map_err(checked)?;
    let mut manifest = top;
    for component in source.manifest().split('/') {
        // Match the declared spelling even on a case-insensitive filesystem.
        manifest = fs::read_dir(&manifest)
            .map_err(|_| Failure::NoManifest)?
            .filter_map(Result::ok)
            .find(|entry| entry.file_name() == component)
            .ok_or(Failure::NoManifest)?
            .path();
    }
    let plain_file = |path: &Path| {
        fs::symlink_metadata(path)
            .is_ok_and(|meta| meta.is_file() && !meta.file_type().is_symlink())
    };
    let folder = manifest
        .parent()
        .expect("joined the declared manifest path");
    if !plain_file(&manifest) || !plain_file(&folder.join("Cargo.toml")) {
        return Err(Failure::NoManifest);
    }
    let bundle = native_mods::bundle_in(folder)
        .map_err(|_| Failure::NoManifest)?
        .filter(|b| b.manifest.id == source.id() && b.manifest.native.is_some())
        .ok_or(Failure::NoManifest)?;
    let bytes = pack(&bundle).map_err(checked)?;
    if bytes.len() as u64 > MAX_ASSET {
        return Err(checked(tr("The GitHub mod download is too large.").into()));
    }
    let mut package = super::checked(&bytes, scratch.path(), None).map_err(checked)?;
    package._scratch = Some(scratch);
    let origin = Origin {
        id: package.id.clone(),
        version: package.version.clone(),
        owner: owner.into(),
        repo: repo.into(),
        tag: pin.release_tag().map(str::to_owned),
        commit: pin.sha().to_owned(),
        asset: None,
        sha256: package.sha256.clone(),
        source: Some(SourceOrigin {
            repo_id: source.repo_id(),
            manifest: source.manifest().into(),
            track: match source.tracker() {
                Tracker::Branch(branch) => SourceTrack::Branch(branch.clone()),
                Tracker::Release => SourceTrack::Release,
            },
            published_at: pin.published_at().map(str::to_owned),
        }),
    };
    Ok((package, origin))
}

/// Parse a pasted source link once. All picker choices share its exact Git snapshot and identity.
pub(crate) fn pack_pasted(
    link: &Link,
    found: &Resolved,
    cancellation: &Cancellation,
) -> Result<Vec<(Packed, Origin)>, Failure> {
    let meta = sources::pasted_source(link, found)?;
    let (owner, repo) = meta
        .repo
        .split_once('/')
        .expect("verified GitHub repository");
    let (scratch, top) = archive_at(owner, repo, &found.commit, cancellation)?;
    let mut result = Vec::new();
    let mut remaining = MAX_ARCHIVE;
    for bundle in find_mods(&top).map_err(Failure::Other)? {
        cancellation.check().map_err(Failure::Other)?;
        if bundle.manifest.native.is_none() {
            continue;
        }
        let manifest = bundle
            .dir
            .strip_prefix(&top)
            .expect("source bundle is inside extracted snapshot")
            .join(sunrise_launcher_smod::manifest::MANIFEST)
            .to_str()
            .ok_or(Failure::NoManifest)?
            .replace('\\', "/");
        if !safe_manifest(&manifest) {
            return Err(Failure::NoManifest);
        }
        let bytes = pack(&bundle).map_err(Failure::Other)?;
        let size = bytes.len() as u64;
        if size > remaining.min(MAX_ASSET) {
            return Err(Failure::Other(
                tr("The GitHub mod download is too large.").into(),
            ));
        }
        remaining -= size;
        let mut package = checked(&bytes, scratch.path(), None).map_err(Failure::Other)?;
        package._scratch = Some(scratch.clone());
        let origin = Origin {
            id: package.id.clone(),
            version: package.version.clone(),
            owner: owner.into(),
            repo: repo.into(),
            tag: meta.tag.clone(),
            commit: found.commit.clone(),
            asset: None,
            sha256: package.sha256.clone(),
            source: Some(SourceOrigin {
                repo_id: meta.repo_id,
                manifest,
                track: meta.track.clone(),
                published_at: meta.published_at.clone(),
            }),
        };
        result.push((package, origin));
    }
    if result.is_empty() {
        return Err(Failure::NoManifest);
    }
    // The package finder sorts by ID. Reject an ambiguous paste before showing the picker.
    if let Some(pair) = result.windows(2).find(|pair| pair[0].1.id == pair[1].1.id) {
        return Err(Failure::Other(trf(
            "The GitHub download has more than one package for {mod}.",
            &[("mod", &pair[0].1.id)],
        )));
    }
    Ok(result)
}

/// Content alone cannot promote a changed repository, tracker, fork, rollback or tag rewrite.
pub(crate) fn forward(predecessor: &Origin, next: &Origin) -> Result<bool, Failure> {
    let Some((old, new)) = predecessor.source.as_ref().zip(next.source.as_ref()) else {
        return Ok(false);
    };
    if old.repo_id != new.repo_id
        || old.manifest != new.manifest
        || old.track != new.track
        || predecessor.owner != next.owner
        || predecessor.repo != next.repo
        || predecessor.id != next.id
        || predecessor.commit == next.commit
        || predecessor.sha256 == next.sha256
    {
        return Ok(false);
    }
    match &new.track {
        SourceTrack::Pinned => Ok(false),
        SourceTrack::Release => Ok(predecessor
            .tag
            .as_ref()
            .zip(next.tag.as_ref())
            .is_some_and(|(previous, current)| previous != current)
            && old
                .published_at
                .as_deref()
                .and_then(date)
                .zip(new.published_at.as_deref().and_then(date))
                .is_some_and(|(old, new)| new > old)),
        SourceTrack::Branch(_) => {
            if let Some(repo) = fixture_repo(&next.owner, &next.repo) {
                let status = crate::process::command("git")
                    .arg("-C")
                    .arg(repo)
                    .args([
                        "merge-base",
                        "--is-ancestor",
                        &predecessor.commit,
                        &next.commit,
                    ])
                    .status()
                    .map_err(|e| Failure::Other(e.to_string()))?;
                return match status.code() {
                    Some(0) => Ok(true),
                    Some(1) => Ok(false),
                    _ => Err(Failure::RefUnavailable),
                };
            }
            sources::branch_ahead(
                &format!("{}/{}", next.owner, next.repo),
                &predecessor.commit,
                &next.commit,
            )
        }
    }
}

fn fixture_repo(owner: &str, repo: &str) -> Option<PathBuf> {
    // Inputs come from validated catalog/receipt segments. Never honor this in signed builds.
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var_os("SUNRISE_SOURCE_FIXTURES")
        .map(|root| PathBuf::from(root).join("repos").join(owner).join(repo))
}
