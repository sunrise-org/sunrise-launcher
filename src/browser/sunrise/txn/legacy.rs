//! Legacy records provide much weaker evidence than v2. Retire only exact redundant
//! evidence; a version number, `existed` flag or missing file is not an undo preimage.
use super::{build, durable, read, switch_record::Identity, unsettled, write, Image, Record};
use crate::browser::{installed, sunrise, Kind, SourceId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
};
use sunrise_launcher_smod::{index, manifest};

const JOURNAL: &str = "mod-browser/install-journal.json";
const UNDO: &str = "mod-browser/build-undo.json";
const LEDGER: &str = "mod-browser/replaced.json";
const COPIES: &str = "mod-browser/replaced";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Kept {
    entry: installed::Entry,
    copy: PathBuf,
    #[serde(rename = "pid")]
    _pid: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    name: PathBuf,
    parent: Identity,
    image: Image,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Retirement {
    home: Identity,
    staging: String,
    stage_id: Identity,
    committed: bool,
    keeping: Vec<Evidence>,
    retiring: Vec<Evidence>,
}

fn at(path: &Path, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", path.display())
}

impl Evidence {
    fn capture(home: &Path, name: PathBuf, image: Image) -> Result<Self, String> {
        let path = home.join(&name);
        let parent = Identity::of(path.parent().ok_or_else(unsettled)?)?;
        if Image::at(&path)? != image {
            return Err(at(&path, unsettled()));
        }
        Ok(Self {
            name,
            parent,
            image,
        })
    }

    fn path(&self, home: &Path) -> Result<PathBuf, String> {
        let path = home.join(&self.name);
        if Identity::of(path.parent().ok_or_else(unsettled)?)? != self.parent {
            return Err(at(&path, unsettled()));
        }
        Ok(path)
    }
}

impl Retirement {
    pub(super) fn validate(&self) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        let has_ledger = self
            .retiring
            .iter()
            .any(|step| step.name == Path::new(LEDGER));
        if !self
            .staging
            .strip_prefix(".legacy-retire-")
            .is_some_and(|name| !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric()))
            || self.home.volume != self.stage_id.volume
            || self.retiring.is_empty()
            || self.retiring.iter().any(|step| {
                !(step.name == Path::new(JOURNAL)
                    || step.name == Path::new(LEDGER)
                    || has_ledger
                        && step
                            .name
                            .strip_prefix(COPIES)
                            .ok()
                            .and_then(Path::to_str)
                            .is_some_and(index::valid_id))
            })
            || self.keeping.iter().any(|step| {
                step.name != Path::new(installed::MANIFEST)
                    && !super::switch_record::valid_package_destination(&step.name)
                    && pack_kind(&step.name).is_none()
            })
            || self
                .retiring
                .iter()
                .any(|step| step.image == Image::Missing)
            || self.retiring.iter().chain(&self.keeping).any(|step| {
                step.parent.volume != self.home.volume || !seen.insert(destination_key(&step.name))
            })
        {
            return Err(unsettled());
        }
        Ok(())
    }
}

/// Called under the profile gate and the browser's file lock. An abandoned v2 Install/Switch
/// is handled by its own record before we examine any older evidence.
pub(super) fn reconcile(home: &Path) -> Result<(), String> {
    reconcile_at(home, |_| {})
}

fn reconcile_at(home: &Path, boundary: impl FnMut(usize)) -> Result<(), String> {
    let root = home.join("mod-browser");
    match fs::symlink_metadata(&root) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(at(&root, e)),
        Ok(meta) if meta.is_dir() && build::regular(&meta) => {}
        Ok(_) => return Err(at(&root, unsettled())),
    }
    if build::optional_bytes(&home.join(UNDO))?.is_some() {
        return Err(at(&home.join(UNDO), unsettled()));
    }
    let journal = build::optional_bytes(&home.join(JOURNAL))?;
    let ledger = build::optional_bytes(&home.join(LEDGER))?;
    if journal.is_none() && ledger.is_none() {
        return Ok(());
    }
    if read(home)?.is_some() {
        return Err(unsettled());
    }

    let mut keeping = Vec::new();
    let mut retiring = Vec::new();
    if let Some(bytes) = journal {
        let path = home.join(JOURNAL);
        let steps: Vec<serde_json::Value> =
            serde_json::from_slice(&bytes).map_err(|e| at(&path, e))?;
        if !steps.is_empty() {
            return Err(at(&path, unsettled()));
        }
        retiring.push(Evidence::capture(home, JOURNAL.into(), image(&bytes))?);
    }
    if let Some(bytes) = ledger {
        let path = home.join(LEDGER);
        let kept: Vec<Kept> = serde_json::from_slice(&bytes).map_err(|e| at(&path, e))?;
        let ledger_image = image(&bytes);
        if !kept.is_empty() {
            let browser = home.join(installed::MANIFEST);
            let bytes =
                build::optional_bytes(&browser)?.ok_or_else(|| at(&browser, unsettled()))?;
            let document: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|e| at(&browser, e))?;
            let rows = document
                .get("entries")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| at(&browser, unsettled()))?;
            build::require_dir(&home.join(COPIES))?;
            keeping.push(Evidence::capture(
                home,
                installed::MANIFEST.into(),
                image(&bytes),
            )?);
            for old in &kept {
                let id = old.entry.project.as_str();
                if !index::valid_id(id) || old.copy != Path::new(COPIES).join(id) {
                    return Err(at(&path, unsettled()));
                }
                let owned: Vec<_> = rows
                    .iter()
                    .filter(|row| {
                        row.get("project").and_then(serde_json::Value::as_str) == Some(id)
                    })
                    .collect();
                let [row] = owned.as_slice() else {
                    return Err(at(&path, unsettled()));
                };
                let current: installed::Entry =
                    serde_json::from_value((*row).clone()).map_err(|e| at(&browser, e))?;
                let current_key = destination_key(Path::new(&current.file));
                let old_key = destination_key(Path::new(&old.entry.file));
                if current.kind != old.entry.kind
                    || rows.iter().any(|other| {
                        !std::ptr::eq(*row, other)
                            && other
                                .get("file")
                                .and_then(serde_json::Value::as_str)
                                .is_some_and(|file| {
                                    let key = destination_key(Path::new(file));
                                    key == current_key || key == old_key
                                })
                    })
                {
                    return Err(at(&path, unsettled()));
                }
                // The old copy and the claimed live winner must both be exactly the files
                // described by their own typed rows. Same version with different bytes is
                // ambiguous even when both packages read correctly.
                let before = checked_package(&home.join(&old.copy), &old.entry)?;
                let live = checked_package(&home.join(&current.file), &current)?;
                if (old.entry.version == current.version) != (before == live) {
                    return Err(at(&path, unsettled()));
                }
                if current.file != old.entry.file {
                    if current.kind == Kind::Mod || old.entry.version == current.version {
                        return Err(at(&path, unsettled()));
                    }
                    keeping.push(Evidence::capture(
                        home,
                        old.entry.file.clone().into(),
                        Image::Missing,
                    )?);
                }
                keeping.push(Evidence::capture(home, current.file.into(), live)?);
                retiring.push(Evidence::capture(home, old.copy.clone(), before)?);
            }
        }
        // Keep the ledger until all its named redundant copies are durably retired.
        retiring.push(Evidence::capture(home, LEDGER.into(), ledger_image)?);
    }
    let stage = tempfile::Builder::new()
        .prefix(".legacy-retire-")
        .tempdir_in(&root)
        .map_err(|e| at(&root, e))?;
    let legacy = Retirement {
        home: Identity::of(home)?,
        staging: stage
            .path()
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .ok_or_else(unsettled)?
            .to_owned(),
        stage_id: Identity::of(stage.path())?,
        committed: false,
        keeping,
        retiring,
    };
    legacy.validate()?;
    let _stage = stage.keep();
    let mut record = Record {
        legacy: Some(Box::new(legacy)),
        ..Record::default()
    };
    write(home, &record)?;
    recover_at(home, &mut record, boundary)
}

fn destination_key(path: &Path) -> String {
    super::switch_record::destination_key(&path.components().collect::<PathBuf>())
}

fn image(bytes: &[u8]) -> Image {
    Image::Sha256(Sha256::digest(bytes).into())
}

fn pack_kind(path: &Path) -> Option<(Kind, index::Kind)> {
    use std::path::Component::Normal;
    let mut parts = path.components();
    let (Some(Normal(folder)), Some(Normal(file)), None) =
        (parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    crate::browser::safe_name(file.to_str()?)?;
    match folder.to_str()? {
        "resourcepacks" => Some((Kind::ResourcePack, index::Kind::ResourcePack)),
        "shaderpacks" => Some((
            Kind::Shader,
            if path.extension().is_some_and(|ext| ext == "sshader") {
                index::Kind::Shader
            } else {
                index::Kind::ShaderPack
            },
        )),
        _ => None,
    }
}

fn checked_package(path: &Path, entry: &installed::Entry) -> Result<Image, String> {
    let valid = || -> Result<Image, String> {
        if entry.source != SourceId::Sunrise {
            return Err(unsettled());
        }
        let relative = Path::new(&entry.file);
        let kind = if entry.kind == Kind::Mod
            && super::switch_record::valid_package_destination(relative)
            && sunrise::mod_id(entry) == Some(&entry.project)
        {
            None
        } else if let Some((_, kind)) = pack_kind(relative).filter(|(kind, _)| *kind == entry.kind)
        {
            Some(kind)
        } else {
            return Err(unsettled());
        };
        build::require_dir(path.parent().ok_or_else(unsettled)?)?;
        let Image::Sha256(digest) = Image::at(path)? else {
            return Err(unsettled());
        };
        let hash = format!("{:x}", sha2::digest::Output::<Sha256>::from(digest));
        if !entry
            .checksum
            .strip_prefix("SHA-256:")
            .is_some_and(|expected| expected.eq_ignore_ascii_case(&hash))
        {
            return Err(unsettled());
        }
        if let Some(kind) = kind {
            index::inspect(&kind, path)?;
        } else {
            let pkg = manifest::Package::read(path)?;
            let native = relative.starts_with(sunrise_launcher_smod::mod_list::NATIVE_DIR);
            if pkg.manifest.id != entry.project
                || pkg.manifest.version != entry.version
                || pkg.manifest.native.is_some() != native
            {
                return Err(unsettled());
            }
        }
        let expected = Image::Sha256(digest);
        if Image::at(path)? != expected {
            return Err(unsettled());
        }
        Ok(expected)
    };
    valid().map_err(|error| at(path, error))
}

pub(super) fn recover(home: &Path, record: &mut Record) -> Result<(), String> {
    recover_at(home, record, |_| {})
}

fn recover_at(
    home: &Path,
    record: &mut Record,
    mut boundary: impl FnMut(usize),
) -> Result<(), String> {
    let Some(legacy) = &mut record.legacy else {
        return Ok(());
    };
    let stage = home.join("mod-browser").join(&legacy.staging);
    if Identity::of(home)? != legacy.home || Identity::of(&stage)? != legacy.stage_id {
        return Err(unsettled());
    }
    for step in &legacy.keeping {
        let path = step.path(home)?;
        if Image::at(&path)? != step.image {
            return Err(at(&path, unsettled()));
        }
    }
    // The record may retire evidence only after the no-op decision itself is committed.
    if !legacy.committed {
        for step in &legacy.retiring {
            let path = step.path(home)?;
            if Image::at(&path)? != step.image {
                return Err(at(&path, unsettled()));
            }
        }
        legacy.committed = true;
        write(home, record)?;
    }
    let legacy = record.legacy.as_ref().expect("legacy record is committed");
    for (index, step) in legacy.retiring.iter().enumerate() {
        let path = step.path(home)?;
        let aside = stage.join(format!("retired-{index}"));
        match (Image::at(&path)?, Image::at(&aside)?) {
            (live, Image::Missing) if live == step.image => {
                durable::restore_move(&path, &aside).map_err(|error| at(&path, error))?
            }
            (Image::Missing, retired) if retired == step.image => {
                durable::sync_restored(&path, &aside, true).map_err(|error| at(&path, error))?
            }
            _ => return Err(at(&path, unsettled())),
        }
        boundary(index);
    }
    *record = Record::default();
    write(home, record)?;
    let _ = fs::remove_dir_all(stage);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::sunrise::txn::{crash_process, wait_to_be_killed, Lease, FILE};

    fn completed_update(home: &Path, corrupt_copy: bool) -> (PathBuf, Vec<u8>, Vec<u8>) {
        let path = home.join("sunrise-mods/lantern.smod");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::create_dir_all(home.join(COPIES)).unwrap();
        let package = |version: &str| {
            sunrise_launcher_smod::Package::write_files(
                &format!("id='lantern'\nname='Lantern'\nversion='{version}'\napi='{}'\ncapabilities=['registries']\n", manifest::API),
                &[],
            ).unwrap()
        };
        let old = package("1.0.0");
        let live = package("2.0.0");
        let row = |version: &str, bytes: &[u8]| {
            serde_json::json!({
                "source":"sunrise", "project":"lantern", "slug":"lantern", "title":"Lantern",
                "kind":"mod", "version":version, "number":version, "file":"sunrise-mods/lantern.smod",
                "checksum":format!("SHA-256:{}", index::sha256(bytes)), "page":"",
            })
        };
        fs::write(&path, &live).unwrap();
        fs::write(
            home.join(COPIES).join("lantern"),
            if corrupt_copy {
                b"wrong".as_slice()
            } else {
                &old
            },
        )
        .unwrap();
        let ledger = serde_json::to_vec(&serde_json::json!([{
            "entry":row("1.0.0", &old), "copy":"mod-browser/replaced/lantern", "pid":0,
        }]))
        .unwrap();
        fs::write(home.join(LEDGER), &ledger).unwrap();
        fs::write(
            home.join(installed::MANIFEST),
            serde_json::to_vec(&serde_json::json!({
                "entries":[row("2.0.0", &live), {"note":"opaque unrelated row"}],
            }))
            .unwrap(),
        )
        .unwrap();
        (path, live, ledger)
    }

    #[test]
    fn malformed_or_conflicting_evidence_never_changes_the_profile() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let (path, live, ledger) = completed_update(home, true);
        let copy = home.join(COPIES).join("lantern");
        let journal = home.join(JOURNAL);
        fs::write(&journal, b"[]").unwrap();
        for evidence in [&ledger[..], b"not json"] {
            fs::write(home.join(LEDGER), evidence).unwrap();
            assert!(Lease::admit(home).is_err());
            assert_eq!(fs::read(&path).unwrap(), live);
            assert_eq!(fs::read(&copy).unwrap(), b"wrong");
            assert_eq!(fs::read(&journal).unwrap(), b"[]");
            assert_eq!(fs::read(home.join(LEDGER)).unwrap(), evidence);
            assert!(!home.join(FILE).exists());
        }
    }

    #[test]
    fn a_foreign_row_cannot_alias_the_live_package() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let (path, live, ledger) = completed_update(home, false);
        let browser = home.join(installed::MANIFEST);
        let mut rows: serde_json::Value =
            serde_json::from_slice(&fs::read(&browser).unwrap()).unwrap();
        rows["entries"][1]["file"] = "sunrise-mods//lantern.smod".into();
        fs::write(&browser, serde_json::to_vec(&rows).unwrap()).unwrap();
        assert!(Lease::admit(home).is_err());
        assert_eq!(fs::read(&path).unwrap(), live);
        assert_eq!(fs::read(home.join(LEDGER)).unwrap(), ledger);
        assert!(home.join(COPIES).join("lantern").is_file());
        assert!(!home.join(FILE).exists());
    }

    #[test]
    fn a_replaced_pack_is_redundant_only_when_its_old_name_is_absent() {
        use std::io::{Cursor, Write};
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        fs::create_dir_all(home.join(COPIES)).unwrap();
        fs::create_dir_all(home.join("resourcepacks")).unwrap();
        let pack = |text: &str| {
            let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
            zip.start_file("pack.mcmeta", zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(
                format!(r#"{{"pack":{{"pack_format":12,"description":"{text}"}}}}"#).as_bytes(),
            )
            .unwrap();
            zip.finish().unwrap().into_inner()
        };
        let old = pack("old");
        let new = pack("new");
        let row = |version: &str, name: &str, bytes: &[u8]| {
            serde_json::json!({
                "source":"sunrise", "project":"lantern", "slug":"lantern", "title":"Lantern",
                "kind":"resource-pack", "version":version, "number":version,
                "file":format!("resourcepacks/{name}.zip"),
                "checksum":format!("SHA-256:{}", index::sha256(bytes)), "page":"",
            })
        };
        let old_path = home.join("resourcepacks/old.zip");
        let new_path = home.join("resourcepacks/new.zip");
        fs::write(&old_path, &old).unwrap();
        fs::write(&new_path, &new).unwrap();
        fs::write(home.join(COPIES).join("lantern"), &old).unwrap();
        fs::write(
            home.join(LEDGER),
            serde_json::to_vec(&serde_json::json!([{
                "entry":row("1", "old", &old), "copy":"mod-browser/replaced/lantern", "pid":0,
            }]))
            .unwrap(),
        )
        .unwrap();
        let browser = home.join(installed::MANIFEST);
        let rows =
            serde_json::to_vec(&serde_json::json!({ "entries":[row("2", "new", &new)] })).unwrap();
        fs::write(&browser, &rows).unwrap();
        assert!(Lease::admit(home).is_err());
        assert_eq!(fs::read(&old_path).unwrap(), old);
        assert!(home.join(LEDGER).is_file());
        fs::remove_file(&old_path).unwrap();
        let _owner = Lease::admit(home).unwrap();
        assert_eq!(fs::read(&new_path).unwrap(), new);
        assert_eq!(fs::read(&browser).unwrap(), rows);
        assert!(!home.join(LEDGER).exists());
        assert!(!home.join(FILE).exists());
    }

    #[test]
    fn retirement_worker() {
        let Some(home) = std::env::var_os("SUNRISE_TEST_INSTALL_OWNER") else {
            return;
        };
        let home = PathBuf::from(home);
        let _owner = Lease::acquire(&home).unwrap();
        let _guard = super::super::locked(&home).unwrap();
        reconcile_at(&home, |step| {
            if step == 0 {
                wait_to_be_killed();
            }
        })
        .unwrap();
    }

    #[test]
    fn a_committed_legacy_retirement_resumes_without_rewriting_live_packages_or_rows() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let (path, live, ledger) = completed_update(home, false);
        let current = fs::read(home.join(installed::MANIFEST)).unwrap();
        crash_process(
            "browser::sunrise::txn::legacy::tests::retirement_worker",
            home,
            None,
            || {
                assert!(Lease::admit(home).is_err());
                assert!(read(home).unwrap().unwrap().legacy.unwrap().committed);
                assert!(!home.join(COPIES).join("lantern").exists());
                assert_eq!(fs::read(home.join(LEDGER)).unwrap(), ledger);
            },
        );
        let owner = Lease::admit(home).unwrap();
        assert!(owner.active_build().unwrap().is_none());
        assert!(!home.join(LEDGER).exists());
        assert!(!home.join(FILE).exists());
        assert_eq!(fs::read(&path).unwrap(), live);
        assert_eq!(fs::read(home.join(installed::MANIFEST)).unwrap(), current);
        fs::write(home.join(JOURNAL), b"[]").unwrap();
        owner.recover_abandoned().unwrap();
        assert!(!home.join(JOURNAL).exists());
        assert!(!home.join(FILE).exists());
    }
}
