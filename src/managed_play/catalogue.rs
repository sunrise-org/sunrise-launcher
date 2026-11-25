use super::{scan_packages, ScannedPackage};
use crate::i18n::{tr, trf};
use std::{fs, path::Path, sync::Arc};
use sunrise_launcher_smod::index::{
    self,
    browser_state::{
        records::{Entry, Installed},
        Kind, SourceId,
    },
};
use sunrise_launcher_smod::index_v2::{Index, Kind as V2Kind, Project, Release, Revoked};

/// Check installed package bytes against the signed schema-2 registry index before Play.
/// Exact-digest takedowns apply even to unowned/local files; whole-project takedowns apply only
/// when project ownership is proved.
pub(super) fn inspect(
    home: &Path,
    load: impl FnOnce() -> Option<Arc<sunrise_launcher_smod::index_v2::Index>>,
) -> Result<Vec<String>, String> {
    let packages = scan_packages(home)?;
    let rows = Installed::read(home)?;
    if !rows
        .entries
        .iter()
        .any(|row| row.source == index::browser_state::SourceId::Sunrise)
        && !packages.iter().any(|package| {
            package.manifest.native.is_some()
                && sunrise_launcher_smod::mod_list::is_enabled(home, &package.manifest.id)
        })
    {
        return Ok(Vec::new());
    }
    let Some(index) = load() else {
        return Ok(vec![
            tr("Mods were not checked for registry takedowns.").into()
        ]);
    };
    for package in &packages {
        let id = &package.manifest.id;
        if package.directory || !sunrise_launcher_smod::mod_list::is_enabled(home, id) {
            continue;
        }
        let hash = index::sha256_file(&package.path)?;
        if let Some(revoked) = revoked_for_installed_package(&index, &rows, home, package, &hash) {
            return Err(trf(
                "{mod} was taken down. Turn it off before playing.\n{reason}",
                &[("mod", &package.manifest.name), ("reason", &revoked.reason)],
            ));
        }
    }
    let mut notices = Vec::new();
    for entry in rows.entries {
        if entry.source != index::browser_state::SourceId::Sunrise
            || !matches!(
                entry.kind,
                index::browser_state::Kind::ResourcePack | index::browser_state::Kind::Shader
            )
        {
            continue;
        }
        let revoked = if sunrise_launcher_smod::index_v2::valid_project_id(&entry.project) {
            index.revoked(&entry.project, entry.sha256())
        } else {
            entry
                .sha256()
                .and_then(|hash| index.revoked_by_sha256(hash))
        };
        if let Some(revoked) = revoked {
            notices.push(trf(
                "{mod} was taken down: {reason} Remove it.",
                &[("mod", &entry.title), ("reason", &revoked.reason)],
            ));
        }
    }
    Ok(notices)
}

/// Verified schema-2 ownership of an installed package on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum V2Ownership {
    /// Verified v2 receipt whose on-disk size and SHA-256 match a release in this project's signed
    /// v2 history.
    Verified {
        project_id: String,
        slug: String,
        version: String,
        repository: crate::managed_play::VerifiedRepository,
    },
    /// Legacy v1 receipt adopted ONLY because exact on-disk size, version, kind, and SHA-256
    /// uniquely matched one `(Project, Release)` in signed v2 history.
    AdoptedV1 {
        project_id: String,
        slug: String,
        version: String,
        repository: crate::managed_play::VerifiedRepository,
    },
    /// Manual file, GitHub source mod, unadopted v1 file, or tampered bytes: not owned by any v2
    /// project. Whole-project takedowns never apply, and updates/switches never replace it.
    Local,
}

impl V2Ownership {
    pub fn project_id(&self) -> Option<&str> {
        match self {
            Self::Verified { project_id, .. } | Self::AdoptedV1 { project_id, .. } => {
                Some(project_id.as_str())
            }
            Self::Local => None,
        }
    }

    pub fn repository(&self) -> Option<&crate::managed_play::VerifiedRepository> {
        match self {
            Self::Verified { repository, .. } | Self::AdoptedV1 { repository, .. } => {
                Some(repository)
            }
            Self::Local => None,
        }
    }

    pub fn is_owned(&self) -> bool {
        !matches!(self, Self::Local)
    }
}

/// True when `package` has a non-registry `.origin.json` sidecar (GitHub or local-folder install),
/// or when reading sidecar metadata fails (fail closed so source mods are never adopted as v2).
pub(crate) fn has_source_origin(home: &Path, package: &ScannedPackage) -> bool {
    if package.directory {
        return false;
    }
    let present = |path: &Path| {
        !matches!(
            fs::symlink_metadata(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
        )
    };
    let beside = package.path.with_extension("origin.json");
    let native = crate::github_mods::origin_path(home, &package.manifest.id);
    present(&beside) || present(&native)
}

/// Classify an installed package's v2 ownership strictly from verified evidence.
///
/// A legacy v1 `mod-browser.json` row is adopted ONLY when its on-disk kind, version, byte size,
/// and SHA-256 uniquely match one `(Project, Release)` in signed v2 history. Any manual file,
/// GitHub source mod, unmatched v1 row, tampered file, or ambiguous digest stays
/// [`V2Ownership::Local`].
pub fn classify_v2_ownership(
    index: &Index,
    home: &Path,
    entry: Option<&Entry>,
    package: &ScannedPackage,
    digest: &str,
    size: u64,
    has_github_origin: bool,
) -> V2Ownership {
    if package.directory || has_github_origin {
        return V2Ownership::Local;
    }
    let Some(row) = entry else {
        return V2Ownership::Local;
    };
    if row.source != SourceId::Sunrise
        || row.kind != Kind::Mod
        || row.path(home).as_deref() != Some(package.path.as_path())
    {
        return V2Ownership::Local;
    }
    let expected_kind = if package.manifest.native.is_some() {
        V2Kind::NativeMod
    } else {
        V2Kind::Mod
    };
    if row.is_v2_registry() && row.mod_id.as_deref() == Some(package.manifest.id.as_str()) {
        let Some(project) = index.project(&row.project) else {
            return V2Ownership::Local;
        };
        if project.mod_id != package.manifest.id || project.kind != expected_kind {
            return V2Ownership::Local;
        }
        let Some(release) = project.releases.iter().find(|release| {
            release.file.sha256.eq_ignore_ascii_case(digest) && release.file.size == size
        }) else {
            return V2Ownership::Local;
        };
        let Some(repository) = row.repository.as_ref() else {
            return V2Ownership::Local;
        };
        if repository.id != project.release_repository(release) {
            return V2Ownership::Local;
        }
        return V2Ownership::Verified {
            project_id: project.id.clone(),
            slug: project.slug.clone(),
            version: release.version.clone(),
            repository: repository.clone(),
        };
    }
    // Legacy v1 receipt: project field holds the in-game mod id, mod_id/repository are absent.
    if row.mod_id.is_none() && row.project == package.manifest.id {
        let matches: Vec<(&Project, &Release)> = index
            .releases_by_sha256(&package.manifest.id, digest)
            .into_iter()
            .filter(|(project, release)| {
                project.kind == expected_kind
                    && release.version == package.manifest.version
                    && release.file.size == size
            })
            .collect();
        if let [(project, release)] = matches.as_slice() {
            return V2Ownership::AdoptedV1 {
                project_id: project.id.clone(),
                slug: project.slug.clone(),
                version: release.version.clone(),
                repository: crate::managed_play::VerifiedRepository {
                    id: project.release_repository(release),
                    name: project.repository.name.clone(),
                    path: project.repository.path.clone(),
                    workflow: release.workflow.clone(),
                },
            };
        }
    }
    V2Ownership::Local
}

/// Returns the active takedown affecting an installed package, if any.
///
/// Exact-digest takedowns (`revoked[].sha256 = Some(...)`) apply even to local/unadopted files.
/// Whole-project takedowns (`revoked[].sha256 = None`) apply ONLY when ownership of that project
/// is proved ([`V2Ownership::Verified`] or [`V2Ownership::AdoptedV1`]).
pub fn revoked_for_package<'a>(
    index: &'a Index,
    ownership: &V2Ownership,
    digest: &str,
) -> Option<&'a Revoked> {
    if let Some(revoked) = index.revoked_by_sha256(digest) {
        return Some(revoked);
    }
    ownership
        .project_id()
        .and_then(|project_id| index.revoked(project_id, Some(digest)))
}

/// Convenience helper for pre-Play checks: classifies `package` against `rows` and checks v2
/// revocations for `digest`.
pub fn revoked_for_installed_package<'a>(
    index: &'a Index,
    rows: &Installed,
    home: &Path,
    package: &ScannedPackage,
    digest: &str,
) -> Option<&'a Revoked> {
    let size = fs::metadata(&package.path).ok()?.len();
    let entry = rows
        .entries
        .iter()
        .find(|row| row.in_game_mod_id() == Some(package.manifest.id.as_str()));
    let has_github = has_source_origin(home, package);
    let ownership = classify_v2_ownership(index, home, entry, package, digest, size, has_github);
    revoked_for_package(index, &ownership, digest)
}
