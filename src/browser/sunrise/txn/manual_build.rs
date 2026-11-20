//! Explicit Build can refresh a derived executable, not change the selected build or mod set.
use super::{build, switch::CandidateBuild, unsettled, Active, Lease, StagedInstall};
use crate::{browser::sunrise, i18n::tr, native_mods};
use std::{path::Path, sync::Arc};

pub struct ManualBuild {
    owner: Arc<Lease>,
    prior: Active,
    inputs: [u8; 32],
    cancellation: native_mods::Cancellation,
}

impl ManualBuild {
    pub fn admit(home: &Path, requested_base: Option<&str>) -> Result<Self, String> {
        let owner = Arc::new(Lease::admit(home)?);
        let prior = owner
            .active_build()?
            .ok_or_else(|| tr("Play this profile once before building native mods.").to_owned())?;
        if requested_base.is_some_and(|tag| tag != prior.base()) {
            return Err(tr("Use Switch to choose another game build for this profile.").into());
        }
        prior.verify_artifacts()?;
        let inputs = build::inputs(home)?;
        let cancellation = native_mods::Cancellation::for_profile(owner.clone());
        Ok(Self {
            owner,
            prior,
            inputs,
            cancellation,
        })
    }

    pub fn home(&self) -> &Path {
        self.owner.home()
    }

    pub fn base(&self) -> &str {
        self.prior.base()
    }

    pub fn cancellation(&self) -> native_mods::Cancellation {
        self.cancellation.clone()
    }

    pub fn run(
        self,
        profile: native_mods::Profile,
        progress: impl Fn(native_mods::Step),
        registry_consent: &dyn Fn(&native_mods::Consent) -> bool,
    ) -> Result<String, native_mods::Failed> {
        if build::inputs(self.home())? != self.inputs {
            return Err(unsettled().into());
        }
        if crate::native_status::exact_enabled(self.home())?.is_empty() {
            build::verify_packages(self.home())?;
            return Err(native_mods::Failed::Nothing);
        }
        self.cancellation.admit_native_work()?;
        let (index, evidence) = sunrise::authoritative_index()?;
        let plan = StagedInstall::begin(self.owner, self.prior)?;
        if plan.inputs() != self.inputs {
            return Err(unsettled().into());
        }
        let compiler = CandidateBuild {
            profile,
            cancellation: self.cancellation,
            registry_consent: Some(registry_consent),
        };
        let ready = plan.prepare_with(&index, &[], compiler, progress)?;
        let image = ready.executable().to_owned();
        ready.commit(false, || {
            let (_, current) = sunrise::authoritative_index()?;
            if current != evidence {
                return Err(tr("The mod registry changed while the edit was being prepared. Nothing was changed.").into());
            }
            Ok(())
        })?;
        Ok(image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend;
    use std::{fs, process::Command};
    use sunrise_launcher_smod::{manifest, mod_list};

    #[test]
    fn manual_build_never_commits_another_base_cancelled_work_or_an_omission() {
        const ACTOR: &str = "SUNRISE_MANUAL_BUILD_TEST";
        let Some(root) = std::env::var_os(ACTOR) else {
            let dir = tempfile::tempdir().unwrap();
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "browser::sunrise::txn::manual_build::tests::manual_build_never_commits_another_base_cancelled_work_or_an_omission", "--nocapture"])
                .env(ACTOR, dir.path())
                .env("SUNRISE_LAUNCHER_HOME", dir.path())
                .env("SUNRISE_HOME", dir.path().join("game"))
                .output().unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        };
        let home = Path::new(&root).join("profile");
        let base = "nightly-manual";
        let image = backend::version_dir(base);
        let resources = backend::resources(base);
        fs::create_dir_all(&image).unwrap();
        fs::create_dir_all(&resources).unwrap();
        fs::write(image.join(backend::CLIENT), b"fixture binary").unwrap();
        fs::write(image.join(".installed"), b"").unwrap();
        fs::write(
            resources.join("sunrise-resources.json"),
            br#"{"data_version":42}"#,
        )
        .unwrap();
        fs::write(resources.join("blocks.json"), b"{}").unwrap();
        let owner = Lease::admit(&home).unwrap();
        let target = owner
            .play_target(|| super::super::Target::legacy(base))
            .unwrap();
        owner
            .play_image(&target, Default::default(), &Default::default(), |_| {})
            .unwrap();
        drop(owner);
        let pointer = home.join(build::FILE);
        let original = fs::read(&pointer).unwrap();
        assert!(ManualBuild::admit(&home, Some("alternate")).is_err());
        {
            let build = ManualBuild::admit(&home, Some(base)).unwrap();
            assert!(Lease::admit(&home).is_err());
            build.cancellation().stop();
            assert!(build
                .owner
                .adopt_image(&build.prior, base, build.inputs, &build.cancellation)
                .is_err());
        }
        let admitted = ManualBuild::admit(&home, None).unwrap();
        assert!(matches!(
            admitted.run(Default::default(), |_| panic!("no compiler"), &|_| panic!(
                "no question"
            )),
            Err(native_mods::Failed::Nothing)
        ));
        let admitted = ManualBuild::admit(&home, None).unwrap();
        let pin = "f".repeat(40);
        let archive = sunrise_launcher_smod::Package::write_files(
            &format!("id='native_fixture'\nname='Native fixture'\nversion='1.0.0'\napi='{}'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{pin}'\nmixins='{pin}'\n", manifest::API),
            &[("Cargo.toml", b"[package]\nname='fixture'\nversion='1.0.0'\n"), ("src/lib.rs", b"pub fn fixture() {}\n")],
        ).unwrap();
        let package = home.join(mod_list::NATIVE_DIR).join("native_fixture.smod");
        fs::create_dir_all(package.parent().unwrap()).unwrap();
        fs::write(&package, &archive).unwrap();
        let selection = b"mod.native_fixture:true\n";
        fs::write(home.join(mod_list::FILE), selection).unwrap();
        assert!(admitted
            .run(
                Default::default(),
                |_| panic!("changed inputs precede SDK work"),
                &|_| panic!("no question")
            )
            .is_err());
        let admitted = ManualBuild::admit(&home, None).unwrap();
        assert_eq!(crate::native_status::enabled(&home).len(), 1);
        assert!(admitted
            .owner
            .adopt_image(
                &admitted.prior,
                base,
                admitted.inputs,
                &admitted.cancellation
            )
            .is_err());
        assert_eq!(fs::read(pointer).unwrap(), original);
        assert_eq!(fs::read(package).unwrap(), archive);
        assert_eq!(fs::read(home.join(mod_list::FILE)).unwrap(), selection);
    }
}
