//! GitHub packages join the exact same-base candidate, including their origin sidecars.
use super::*;
use crate::github_mods::{Origin, Prepared};

#[derive(Debug)]
pub enum GithubInstallError {
    Policy(String),
    Fit(String),
    Build(String),
    Admission(String),
}

impl std::fmt::Display for GithubInstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Policy(why) | Self::Fit(why) | Self::Build(why) | Self::Admission(why) => {
                f.write_str(why)
            }
        }
    }
}

impl From<GithubInstallError> for String {
    fn from(error: GithubInstallError) -> Self {
        error.to_string()
    }
}

pub struct GithubInstall {
    ready: ReadyInstall,
    registry: String,
    origins: Vec<Origin>,
}

impl GithubInstall {
    pub fn question(&self) -> &worlds::SwitchQuestion {
        self.ready.question()
    }

    pub fn commit(self, approved: bool) -> Result<Vec<Origin>, GithubInstallError> {
        self.ready
            .commit(approved, || {
                let (_, current) = sunrise::authoritative_index()?;
                if current != self.registry {
                    return Err(tr("The mod registry changed while the install was being prepared. Nothing was installed.").into());
                }
                Ok(())
            })
            .map_err(GithubInstallError::Admission)?;
        Ok(self.origins)
    }
}

pub fn prepare_github_install(
    owner: Arc<Lease>,
    instance: &Instance,
    prepared: &Prepared,
    expected: Option<&Origin>,
    progress: &dyn Fn(&str, f32),
    registry_consent: &dyn Fn(&native_mods::Consent) -> bool,
) -> Result<GithubInstall, GithubInstallError> {
    use GithubInstallError::{Admission, Build, Fit, Policy};
    native_mods::require_consent().map_err(|error| Policy(error.to_string()))?;
    let (index, registry) = sunrise::authoritative_index().map_err(Admission)?;
    let plan = StagedInstall::for_instance(owner, instance, progress).map_err(Admission)?;
    let (origins, sidecars) = plan.stage_github(prepared, expected)?;
    let compiler = CandidateBuild {
        profile: instance.native_profile,
        cancellation: native_mods::Cancellation::for_profile(plan.owner.clone()),
        registry_consent: Some(registry_consent),
    };
    let after = plan.proposed_inventory(&index).map_err(Policy)?;
    let ready = plan
        .seal(after, sidecars, &[], compiler, |step| {
            progress(&step.label(), 0.99)
        })
        .map_err(|error| match &error {
            native_mods::Failed::NoConsent => Policy(error.to_string()),
            native_mods::Failed::Fit(why) => Fit(why.clone()),
            native_mods::Failed::Mods { left, .. }
                if left
                    .iter()
                    .any(|item| matches!(item.why, native_mods::Why::Unfit(_))) =>
            {
                Fit(error.to_string())
            }
            _ => Build(error.to_string()),
        })?;
    Ok(GithubInstall {
        ready,
        registry,
        origins,
    })
}

type Sidecars = Vec<(PathBuf, Option<PathBuf>)>;

impl StagedInstall {
    fn stage_github(
        &self,
        prepared: &Prepared,
        expected: Option<&Origin>,
    ) -> Result<(Vec<Origin>, Sidecars), GithubInstallError> {
        use GithubInstallError::{Admission, Policy};
        let stage = self.stage();
        let mods = prepared.mods();
        if mods.is_empty() || expected.is_some_and(|old| mods.len() != 1 || mods[0].id != old.id) {
            return Err(Policy(
                tr("Choose the GitHub mods you want to install.").into(),
            ));
        }
        let mut origins = Vec::new();
        let mut sidecars = Vec::new();
        for packed in mods {
            let id = &packed.id;
            let origin = packed.origin(prepared.source());
            let relative = Path::new(mod_list::NATIVE_DIR).join(format!("{id}.smod"));
            let relative_origin = github_mods::origin_path(Path::new(""), id);
            let target = stage.join(&relative);
            let sidecar = stage.join(&relative_origin);
            let collision = || {
                Policy(trf(
                "A mod with the id {mod} is already installed from another source. Remove it first.",
                &[("mod", id)],
            ))
            };
            let current = self.before.packages.get(id);
            if let Some(prior) = current {
                let old = github_mods::origin(stage, id).ok_or_else(collision)?;
                if prior.row.is_some()
                    || prior.relative != relative
                    || old.id != *id
                    || old.sha256 != prior.digest
                    || !old.owner.eq_ignore_ascii_case(&origin.owner)
                    || !old.repo.eq_ignore_ascii_case(&origin.repo)
                    || expected.is_some_and(|expected| expected != &old)
                {
                    return Err(collision());
                }
            } else if expected.is_some()
                || build::optional_bytes(&sidecar)
                    .map_err(Admission)?
                    .is_some()
                || stage.join(mod_list::DIR).join(id).exists()
                || target.exists()
            {
                return Err(collision());
            }
            let bytes = fs::read(&packed.file).map_err(|e| Admission(at(&packed.file, e)))?;
            if index::sha256(&bytes) != origin.sha256 {
                return Err(Admission(trf(
                    "The downloaded package for {mod} changed. Fetch it again.",
                    &[("mod", id)],
                )));
            }
            write(&target, &bytes).map_err(Admission)?;
            let origin_json =
                serde_json::to_vec(&origin).map_err(|error| Admission(error.to_string()))?;
            write(&sidecar, &origin_json).map_err(Admission)?;
            // An update preserves the profile's existing enabled bit. Only a new mod turns on.
            if current.is_none() {
                crate::browser::mod_list::set_enabled(stage, id, true).map_err(Admission)?;
            }
            origins.push(origin);
            sidecars.push((relative_origin, Some(sidecar)));
        }
        Ok((origins, sidecars))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github_mods::tests::prepared_update;
    use std::io::Write;
    use sunrise_launcher_nbt::{write_named, Limits, NamedTag, Tag};

    fn update_fixture(home: &Path) -> (Arc<Lease>, Active, PathBuf, PathBuf, Origin) {
        let old = prepared_update("v1", "{\"name\":\"Old block\"}", "");
        let packed = &old.mods()[0];
        let origin = packed.origin(old.source());
        let folder = home.join(mod_list::NATIVE_DIR);
        fs::create_dir(&folder).unwrap();
        let package = folder.join("zoom.smod");
        let sidecar = github_mods::origin_path(home, "zoom");
        fs::copy(&packed.file, &package).unwrap();
        fs::write(&sidecar, serde_json::to_vec(&origin).unwrap()).unwrap();
        crate::browser::mod_list::set_enabled(home, "zoom", true).unwrap();
        let owner = Arc::new(Lease::admit(home).unwrap());
        let prior = build::Bootstrap::bind(home, Target::legacy("nightly-original").unwrap())
            .unwrap()
            .test_active();
        build::publish(home, &prior).unwrap();
        (owner, prior, package, sidecar, origin)
    }

    #[test]
    fn github_update_changed_definitions_make_a_world_question_with_the_same_ids() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let (owner, prior, live, _, origin) = update_fixture(home);
        let stage = StagedInstall::copy_current(owner, |owner| {
            Ok((owner.reserve_install_set(&prior)?, Basis::Committed(prior)))
        })
        .unwrap();
        let update = prepared_update("v2", "{\"name\":\"New block\"}", "");
        let new = &update.mods()[0].file;
        assert!(crate::browser::sunrise::content_loss::dropped(&live, new)
            .unwrap()
            .is_empty());
        assert!(crate::browser::sunrise::content_loss::definitions_changed(&live, new).unwrap());
        assert!(!crate::browser::sunrise::content_loss::definitions_changed(&live, &live).unwrap());
        stage.stage_github(&update, Some(&origin)).unwrap();
        let after = switch::Inventory::read(stage.stage()).unwrap();
        let world = home.join("saves/Test world");
        fs::create_dir_all(&world).unwrap();
        let bytes = write_named(
            &NamedTag {
                name: "".into(),
                tag: Tag::compound([
                    (
                        "Data",
                        Tag::compound([("LevelName", Tag::String("Test world".into()))]),
                    ),
                    (
                        "sunrise",
                        Tag::compound([(
                            "BlockNamespaces",
                            Tag::List(vec![Tag::String("zoom".into())]),
                        )]),
                    ),
                ]),
            },
            Limits::default(),
        )
        .unwrap();
        let mut gz = flate2::write::GzEncoder::new(
            fs::File::create(world.join("level.dat")).unwrap(),
            flate2::Compression::fast(),
        );
        gz.write_all(&bytes).unwrap();
        gz.finish().unwrap();
        let (_, question) = stage.world_risk(&after, &[]).unwrap();
        assert!(question.needed());
        assert_eq!(question.affected[0].world, "Test world");
        assert_eq!(question.affected[0].mods, ["zoom"]);
    }

    #[test]
    fn failed_github_candidate_proof_keeps_the_installed_package_and_origin() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let (owner, prior, package, sidecar, origin) = update_fixture(home);
        let old_package = fs::read(&package).unwrap();
        let old_origin = fs::read(&sidecar).unwrap();
        let stage = StagedInstall::copy_current(owner.clone(), |owner| {
            Ok((owner.reserve_install_set(&prior)?, Basis::Committed(prior)))
        })
        .unwrap();
        let update = prepared_update("v2", "{}", "\n[dependencies]\nneeds_another_mod = '*'\n");
        let (_, sidecars) = stage.stage_github(&update, Some(&origin)).unwrap();
        let after = switch::Inventory::read(stage.stage()).unwrap();
        let compiler = CandidateBuild {
            profile: native_mods::Profile::default(),
            cancellation: native_mods::Cancellation::for_profile(owner.clone()),
            registry_consent: None,
        };
        assert!(matches!(
            stage.seal(after, sidecars, &[], compiler, |_| panic!(
                "the unfitted set must not start a build"
            )),
            Err(native_mods::Failed::Fit(_))
        ));
        assert_eq!(fs::read(package).unwrap(), old_package);
        assert_eq!(fs::read(sidecar).unwrap(), old_origin);
        assert!(!owner.outstanding());
    }
}
