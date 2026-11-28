//! Ordinary client Play keeps one admission on its worker through the actual spawn.
use super::{
    catalogue,
    first::{game_lock, RefreshError, VerifiedSnapshot},
    unsettled, Active, FirstBuild, Profile,
};
use crate::{
    account::{self, Accounts, NoOwner},
    backend::{self, installation::BoundVariant, RuntimeGuard},
    developer_offline, instances, native_mods,
    process::child::Child,
};
use std::{
    fs::File,
    io::{self, Write as _},
    path::Path,
    process::Command,
    sync::Arc,
};

pub enum Progress {
    Download(f32),
    Prepare,
    NativeBuild {
        base: String,
        mods: Vec<crate::native_status::BuildMod>,
    },
    Native(native_mods::Step),
    NativeLog(std::sync::Arc<str>),
    /// The installed files are being checked before the game starts.
    Verify,
    SignIn,
    Line(String),
    HookLine(String),
}

#[derive(Debug)]
pub enum StartError {
    Journal(super::GameInstallProblem),
    Account(NoOwner),
    Native(native_mods::Failed, Option<Arc<FailedBuild>>),
    Hook(crate::launch::PreLaunchError),
    UnknownSpawn(String),
    Snapshot(String),
    Other(String),
}

impl From<String> for StartError {
    fn from(error: String) -> Self {
        Self::Other(error)
    }
}

impl From<StartError> for String {
    fn from(error: StartError) -> Self {
        match error {
            StartError::Journal(error) => error.details(),
            StartError::Account(error) => match error.keyring.as_deref() {
                Some(why) => why.to_owned(),
                None => format!("{error} {}", crate::i18n::tr(backend::SIGN_IN_HINT)),
            },
            StartError::Native(error, _) => error.into(),
            StartError::Hook(error) => error.to_string(),
            StartError::UnknownSpawn(error)
            | StartError::Snapshot(error)
            | StartError::Other(error) => error,
        }
    }
}

/// The exact base and native-archive digests of the Play attempt that reported a registry conflict.
#[derive(Debug, PartialEq, Eq)]
pub struct FailedBuild {
    build: Build,
    digests: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, PartialEq, Eq)]
enum Build {
    Active(Active),
    First(FirstBuild),
}

#[derive(Clone)]
pub struct ModRecovery {
    pub build: Arc<FailedBuild>,
    pub mods: Vec<(String, String)>,
}

fn capture_native_digests(home: &Path) -> std::collections::BTreeMap<String, String> {
    let Ok(packages) = super::scan_packages(home) else {
        return std::collections::BTreeMap::new();
    };
    packages
        .into_iter()
        .filter(|pkg| pkg.manifest.native.is_some() && !pkg.directory)
        .filter_map(|pkg| {
            let digest = sunrise_launcher_smod::index::sha256_file(&pkg.path).ok()?;
            Some((pkg.manifest.id, digest))
        })
        .collect()
}

fn native_failure(
    home: &Path,
    error: native_mods::Failed,
    build: impl FnOnce() -> Option<Build>,
) -> StartError {
    let proof = if matches!(error, native_mods::Failed::RegistryConflict { .. }) {
        build().map(|build| {
            Arc::new(FailedBuild {
                build,
                digests: capture_native_digests(home),
            })
        })
    } else {
        None
    };
    StartError::Native(error, proof)
}

pub struct Client<'a> {
    pub instance: &'a instances::Instance,
    pub accepted_account: Option<&'a str>,
    /// An explicit command-line account still requires ownership in a developer build.
    pub explicit_account: bool,
    pub piped: bool,
    /// The verified registry catalogue's index, if one can be read before Play.
    pub registry: fn() -> Option<Arc<sunrise_launcher_smod::index_v2::Index>>,
}

pub struct Started {
    pub instance: instances::Instance,
    pub child: Child,
    pub runtime: RuntimeGuard,
    pub executable: String,
    pub notice: Option<String>,
    pub registry_notices: Vec<String>,
    pub owner: bool,
}

impl Client<'_> {
    pub fn start(
        self,
        choose: impl FnOnce() -> Result<FirstBuild, String>,
        arguments: impl FnOnce(&mut Command) -> Result<(), String>,
        prepare_jars: impl FnOnce(&Command) -> Result<(), String>,
        cancellation: &native_mods::Cancellation,
        progress: impl Fn(Progress),
    ) -> Result<Started, StartError> {
        self.start_with_recovery(
            None,
            choose,
            arguments,
            prepare_jars,
            cancellation,
            progress,
        )
    }

    /// The player accepted turning off these exact versions before resuming Play. Apply the
    /// change under admission for that same profile and before staging any build inputs.
    pub fn start_with_recovery(
        self,
        recovery: Option<ModRecovery>,
        choose: impl FnOnce() -> Result<FirstBuild, String>,
        arguments: impl FnOnce(&mut Command) -> Result<(), String>,
        prepare_jars: impl FnOnce(&Command) -> Result<(), String>,
        cancellation: &native_mods::Cancellation,
        progress: impl Fn(Progress),
    ) -> Result<Started, StartError> {
        // A fresh default profile may not have a game directory yet. Do not recreate
        // a named profile that disappeared while its Play awaited this worker.
        if self.instance.kind == instances::Kind::Pack {
            return Err(unsettled!().into());
        }
        if self.accepted_account.is_none() && !developer_offline::requested() {
            return Err(StartError::Account(NoOwner::default()));
        }
        if self.instance.kind == instances::Kind::Default && self.instance.incarnation.is_none() {
            std::fs::create_dir_all(&self.instance.home)
                .map_err(|error| super::at(&self.instance.home, error))?;
        }
        let mut profile =
            Profile::open_instance(&backend::game_home(), &backend::data_dir(), self.instance)?;
        let instance = profile.admitted_instance(self.instance);
        let mut choose = Some(choose);
        let selected = if let Some(recovery) = recovery {
            let _manifest = profile.lock_inputs()?;
            cancellation.check()?;
            let selected = recovery.verify(&profile, &mut choose)?;
            turn_off_mods(
                profile.home(),
                &recovery.mods,
                &recovery.build.digests,
                |mut file, bytes| {
                    file.write_all(bytes)?;
                    file.sync_all()
                },
            )?;
            selected
        } else {
            None
        };
        let check_build = |tag: &str, variant: Option<&BoundVariant>| {
            if self.accepted_account.is_none() && !developer_offline::enabled(tag) {
                return Err(StartError::Account(NoOwner::default()));
            }
            backend::managed_game_architecture_launch_error(tag, variant)
                .map_or(Ok(()), |error| Err(StartError::Other(error)))
        };
        let (active, registry_notices, proof) = self.ready(
            &mut profile,
            || selected.map_or_else(|| choose.expect("a first Play needs a selection")(), Ok),
            check_build,
            cancellation,
            &progress,
            self.registry,
        )?;
        let tag = active.executable();
        let offline = !self.explicit_account && developer_offline::enabled(tag);
        let mut cmd = backend::client_with_resources(tag, &active.resource_path());
        instances::apply(&mut cmd, self.instance)?;
        let vars = self.instance.vars();
        cmd.envs(vars.iter().map(|(key, value)| (key, value)));
        // The session is made last, just before the process starts (#542): the game refuses a
        // session file older than two minutes, and the start-up checks (every file of the
        // prepared resources is hashed) can take longer than that on a slow disk.
        let offline_notice = if offline {
            developer_offline::require_capability(tag)?;
            developer_offline::mark(&mut cmd);
            Some(crate::i18n::tr(developer_offline::NOTICE).into())
        } else {
            None
        };
        let session_file = std::cell::RefCell::new(None::<std::path::PathBuf>);
        let notice = std::cell::RefCell::new(offline_notice);
        let launch = &self.instance.launch;
        let started = (|| {
            arguments(&mut cmd)?;
            // The window says what the check of the files (their hashes can take minutes on a
            // slow disk) is doing; the session is made after it.
            progress(Progress::Verify);
            profile.spawn_checked_account(
                &active,
                cancellation,
                cmd,
                (
                    launch,
                    self.piped,
                    &|line| progress(Progress::HookLine(line)),
                    proof,
                ),
                prepare_jars,
                |cmd: &mut Command| {
                    if offline {
                        return Ok(());
                    }
                    // Account refusals stay `StartError::Account`: the window opens sign-in for
                    // them, and never reports them as a failed build.
                    let uuid = self
                        .accepted_account
                        .ok_or_else(|| StartError::Account(NoOwner::default()))?;
                    account::check_launch_account(&Accounts::load(), uuid)
                        .map_err(StartError::Account)?;
                    progress(Progress::SignIn);
                    let session = account::session_for_game_accepted(&backend::game_home(), uuid)
                        .map_err(StartError::Account)?;
                    // Signed out while the services were asked: that session never plays.
                    if let Err(refused) = account::check_launch_account(&Accounts::load(), uuid) {
                        account::discard_launch_session(session.file);
                        return Err(StartError::Account(refused));
                    }
                    account::retain_launch_session(session.file.as_deref());
                    *session_file.borrow_mut() = session.file.clone();
                    *notice.borrow_mut() = session
                        .apply(cmd)
                        .map(|reason| {
                            crate::i18n::trf("Playing offline: {reason}", &[("reason", &reason)])
                        })
                        .or_else(account::take_unsaved);
                    Ok(())
                },
            )
        })();
        let session_file = session_file.into_inner();
        let notice = notice.into_inner();
        match started {
            Ok((child, mut runtime)) => {
                runtime.managed_session = session_file;
                Ok(Started {
                    instance,
                    child,
                    runtime,
                    executable: tag.into(),
                    notice,
                    registry_notices,
                    owner: !offline,
                })
            }
            Err(error) => {
                if !matches!(error, StartError::UnknownSpawn(_)) {
                    account::discard_launch_session(session_file);
                }
                Err(error)
            }
        }
    }

    fn ready(
        &self,
        profile: &mut Profile,
        choose: impl FnOnce() -> Result<FirstBuild, String>,
        check_build: impl FnOnce(&str, Option<&BoundVariant>) -> Result<(), StartError>,
        cancellation: &native_mods::Cancellation,
        progress: impl Fn(Progress),
        load_index: impl FnOnce() -> Option<std::sync::Arc<sunrise_launcher_smod::index_v2::Index>>,
    ) -> Result<(Active, Vec<String>, Option<VerifiedSnapshot>), StartError> {
        let home = profile.home().to_owned();
        let mode = self.instance.native_profile;
        let started = std::cell::Cell::new(false);
        let native = |base: &str, step| {
            if !started.replace(true) {
                progress(Progress::NativeBuild {
                    base: base.into(),
                    mods: crate::native_status::build_mods(&home),
                });
            }
            progress(Progress::Native(step));
        };
        let log = |output| progress(Progress::NativeLog(output));
        if let Some(old) = profile.active()? {
            let checking = crate::launcher_log::step("Checking game files");
            check_build(old.base(), old.variant.as_deref())?;
            // Prepare without the game-manifest lock. The profile stays admitted and the
            // saved generation continues to reserve this base until refresh commits.
            let replacement = match old.resources.check_play_output(old.base()) {
                Ok(()) => None,
                Err(backend::Unprepared::Incomplete | backend::Unprepared::ServerOnly) => {
                    // The on-disk manifest is not proof of what the saved snapshot was.
                    // Verify both the old resources and the preparer before replacing them.
                    old.verify_binaries()?;
                    old.resources
                        .verify(old.base())
                        .map_err(StartError::Snapshot)?;
                    progress(Progress::Prepare);
                    Some(super::Resources::for_play_with(
                        old.base(),
                        profile.handoff(),
                        &|line| emit_prepare_line(&progress, line),
                        &|line| progress(Progress::Line(line)),
                    )?)
                }
                Err(issue) => return Err(issue.message(|| unsettled!()).into()),
            };
            // Legacy writers still use the game lock, not profile admission. Keep
            // them out until refresh has staged the exact set we checked.
            let manifest = game_lock(&home)?;
            checking.note("the profile's lock is held");
            super::recover_game_install(&home).map_err(StartError::Journal)?;
            let notices = catalogue::inspect(&home, load_index)?;
            let (replacement_resources, proven_by_seal) = match replacement {
                Some(prepared) => (Some(prepared.resources), prepared.proven_by_seal),
                None => (None, false),
            };
            let (active, verified_resources) = profile
                .refresh_for_play(
                    mode,
                    cancellation,
                    |step| native(old.base(), step),
                    Some(&log),
                    replacement_resources,
                )
                .map_err(|error| match error {
                    RefreshError::Native(error) => {
                        native_failure(&home, error, || Some(Build::Active(old)))
                    }
                    RefreshError::Snapshot(error) => StartError::Snapshot(error),
                })?
                .ok_or_else(|| unsettled!())?;
            let proof = (proven_by_seal || verified_resources)
                .then(|| VerifiedSnapshot::new(active.clone(), home.clone(), manifest));
            checking.done("");
            return Ok((active, notices, proof));
        }

        let ready = (|| {
            let first = profile.begin_first(choose, |fraction| {
                progress(Progress::Download(fraction));
            })?;
            check_build(first.base(), first.variant().as_ref())?;
            progress(Progress::Prepare);
            let prepared = first
                .prepare_with(&|line| emit_prepare_line(&progress, line), &|line| {
                    progress(Progress::Line(line))
                })?;
            let (image, notices) = {
                let _manifest = game_lock(&home)?;
                super::recover_game_install(&home).map_err(StartError::Journal)?;
                first.verify_inputs()?;
                let notices = catalogue::inspect(&home, load_index)?;
                let image = first
                    .exact_image(
                        mode,
                        cancellation,
                        |step| native(first.base(), step),
                        Some(&log),
                    )
                    .map_err(|error| {
                        native_failure(&home, error, || {
                            first.recovery_selection().ok().map(Build::First)
                        })
                    })?;
                (image, notices)
            };
            if !prepared.proven_by_seal {
                progress(Progress::Verify);
            }
            let first = first.commit_prepared(&image, prepared, cancellation)?;
            Ok((first.active.clone(), notices, Some(first)))
        })();
        if ready.is_err() {
            profile.release_failed_first_play()?;
        }
        ready
    }
}

impl ModRecovery {
    fn verify(
        &self,
        profile: &Profile,
        choose: &mut Option<impl FnOnce() -> Result<FirstBuild, String>>,
    ) -> Result<Option<FirstBuild>, String> {
        let changed = || {
            crate::i18n::tr("The profile's game build changed. No mods were turned off.").to_owned()
        };
        match (&self.build.build, profile.active()?) {
            (Build::Active(failed), Some(now))
                if now.profile == failed.profile
                    && now.generation() == failed.generation()
                    && now.same_base(failed) =>
            {
                now.verify_artifacts().map_err(|_| changed())?;
                Ok(None)
            }
            (Build::First(failed), None) if !profile.has_accepted_bootstrap()? => {
                let now = choose.take().expect("a first Play needs a selection")()
                    .map_err(|_| changed())?;
                if *failed == now {
                    Ok(Some(now))
                } else {
                    Err(changed())
                }
            }
            _ => Err(changed()),
        }
    }
}

/// The caller holds profile admission and the game-manifest lock through publication.
fn turn_off_mods(
    home: &Path,
    mods: &[(String, String)],
    digests: &std::collections::BTreeMap<String, String>,
    stage: impl FnOnce(&File, &[u8]) -> io::Result<()>,
) -> Result<(), String> {
    use sunrise_launcher_smod::mod_list::{ModList, FILE};
    let path = home.join(FILE);
    let original = super::optional_bytes(&path)?.unwrap_or_default();
    let text = std::str::from_utf8(&original).map_err(|error| super::at(&path, error))?;
    let enabled = ModList::parse(text).enabled;
    let packages = super::scan_packages(home)?;
    let turns_off = |id: &str| mods.iter().any(|(off, _)| off == id);
    if mods.is_empty()
        || !mods.iter().all(|(id, version)| {
            let Some(expected_digest) = digests.get(id) else {
                return false;
            };
            enabled.contains(id)
                && packages.iter().any(|pkg| {
                    pkg.manifest.native.is_some()
                        && !pkg.directory
                        && pkg.manifest.id == *id
                        && pkg.manifest.version == *version
                        && sunrise_launcher_smod::index::sha256_file(&pkg.path)
                            .is_ok_and(|actual| actual == *expected_digest)
                })
        })
    {
        return Err(crate::i18n::tr("The enabled mods changed. No mods were turned off.").into());
    }
    for pkg in &packages {
        let manifest = &pkg.manifest;
        if enabled.contains(&manifest.id) && !turns_off(&manifest.id) {
            if let Some((required, _)) = manifest
                .required_client_dependencies()
                .find(|(id, _)| turns_off(id))
            {
                let dependency = packages
                    .iter()
                    .find(|pkg| pkg.manifest.id == *required)
                    .expect("the exact mod to turn off was verified");
                return Err(crate::i18n::trf(
                    "{mod} needs {dependency}. No mods were turned off.",
                    &[
                        ("mod", &manifest.name),
                        ("dependency", &dependency.manifest.name),
                    ],
                ));
            }
        }
    }
    // Switch's review: neither level file can rule a mod out, since older chunks keep its blocks
    // and items, so every save in the profile counts as using the mods turned off.
    let suspects = mods
        .iter()
        .map(|(id, _)| (id.clone(), std::collections::BTreeSet::new()))
        .collect();
    let mut worlds = crate::worlds::review_risk(home, &suspects)
        .map_err(|error| {
            crate::elog!(
                "checking saved worlds before turning off mods: {}",
                crate::launcher_log::safe_error(&error.to_string())
            );
            crate::i18n::tr("Sunrise couldn't check the saved worlds. No mods were turned off.")
                .to_owned()
        })?
        .into_iter()
        .map(|world| world.name)
        .collect::<Vec<_>>();
    if !worlds.is_empty() {
        worlds.sort();
        worlds.dedup();
        return Err(crate::i18n::trf(
            "Turning off these mods could remove content from: {worlds}. No mods were turned off.",
            &[("worlds", &worlds.join(", "))],
        ));
    }
    // Keep the unrelated lines byte for byte, including key bindings and line endings.
    let mut next = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        if !line
            .split_once(':')
            .is_some_and(|(key, _)| key.trim().strip_prefix("mod.").is_some_and(turns_off))
        {
            next.push_str(line);
        }
    }
    let staged = tempfile::NamedTempFile::new_in(home).map_err(|error| super::at(&path, error))?;
    stage(staged.as_file(), next.as_bytes()).map_err(|error| super::at(&path, error))?;
    staged
        .persist(&path)
        .map_err(|error| super::at(&path, error))?;
    Ok(())
}

#[cfg(test)]
mod tests;

/// Resolve an ordinary headless first Play lazily. An existing committed or pending
/// build bypasses the catalogue completely; an explicit first selection remains exact.
pub fn headless_first(instance: &instances::Instance) -> Result<FirstBuild, String> {
    let installed = match instance.resolve_version(None) {
        Ok(tag) => return FirstBuild::installed(&tag),
        Err(error) => error,
    };
    let releases = backend::releases()?;
    let release = if let Some(tag) = instance.version.as_deref() {
        releases
            .iter()
            .find(|release| release.tag == tag && release.asset.is_some())
    } else {
        instance
            .channel
            .following(&releases)
            .builds(&releases)
            .next()
    };
    FirstBuild::download(release.ok_or(installed)?)
}

/// Logs a preparation line to `launcher.log` before forwarding it to the caller (both the
/// window and the CLI `play` path pass through here, so neither logs twice and both record
/// preparation progress in `launcher.log`).
fn emit_prepare_line(progress: &impl Fn(Progress), line: String) {
    crate::launcher_log::line(crate::launcher_log::preparer_line(&line));
    progress(Progress::Line(line));
}

#[cfg(test)]
mod cli_log_tests {
    use super::*;

    #[test]
    fn cli_play_prepare_and_meter_lines_land_in_launcher_log_without_ui_sink() {
        const CHILD: &str = "SUNRISE_TEST_CLI_PLAY_LOG_CHILD";
        const TEST: &str = "managed_play::client::cli_log_tests::cli_play_prepare_and_meter_lines_land_in_launcher_log_without_ui_sink";
        if std::env::var_os(CHILD).is_none() {
            let root = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST, "--nocapture"])
                .env(CHILD, "1")
                .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
                .env("SUNRISE_HOME", root.path().join("game"))
                .env(
                    "SUNRISE_LAUNCHER_LOCATION_FILE",
                    root.path().join("location"),
                )
                .env("SUNRISE_NO_KEYCHAIN", "1")
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success() && stdout.contains("test result: ok. 1 passed;"),
                "{stdout}\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        crate::launcher_log::hide("Alex_CliPlay");
        let before = crate::launcher_log::since(0).0;
        let seen = std::cell::RefCell::new(Vec::new());
        emit_prepare_line(
            &|p| {
                if let Progress::Line(line) = p {
                    seen.borrow_mut().push(line);
                }
            },
            "Downloading Minecraft's files: 5 of 40 for Alex_CliPlay".into(),
        );
        let meter =
            crate::launcher_log::PlayMeter::new(|d, t| format!("Checking cli files: {d} of {t}"));
        meter.tick(1, 50);
        assert_eq!(
            seen.into_inner(),
            vec!["Downloading Minecraft's files: 5 of 40 for Alex_CliPlay".to_string()]
        );
        let (_, lines) = crate::launcher_log::since(before);
        let prep: Vec<&String> = lines
            .iter()
            .filter(|l| l.contains("Preparing: Downloading Minecraft's files: 5 of 40"))
            .collect();
        assert_eq!(prep.len(), 1, "{lines:?}");
        assert!(!prep[0].contains("Alex_CliPlay"), "{prep:?}");
        assert!(
            lines
                .iter()
                .any(|l| l.contains("Checking cli files: 1 of 50")),
            "{lines:?}"
        );
    }

    /// The preparer names files in its progress lines and errors: such a line reaches the
    /// caller whole and `launcher.log` only as a fixed note (review of #615).
    #[test]
    fn a_preparer_line_naming_a_file_or_address_stays_out_of_the_log() {
        let named = [
            "Imported client 26.3 from /home/ada/Private Client Backup/.minecraft/versions/26.3/26.3.jar",
            r"C:\Users\Ada Lovelace\Private Client Backup\java\bin\javac.exe: The system cannot find the file specified. (os error 2)",
            r"\\nas\Private Client Backup\sounds\indexes\26.json: denied",
            "Downloading https://piston-data.mojang.com/v1/objects/0123/client.jar for Private Client Backup",
            "Missing versions/26.3/26.3.json in Private Client Backup",
        ];
        let before = crate::launcher_log::since(0).0;
        let seen = std::cell::RefCell::new(Vec::new());
        for line in named {
            emit_prepare_line(
                &|p| {
                    if let Progress::Line(line) = p {
                        seen.borrow_mut().push(line);
                    }
                },
                line.into(),
            );
        }
        assert_eq!(seen.into_inner(), named, "the window gets each line whole");
        let (_, lines) = crate::launcher_log::since(before);
        let left_out = lines
            .iter()
            .filter(|l| l.contains("Preparing: (a line that names a file or an address"))
            .count();
        assert_eq!(left_out, named.len(), "{lines:?}");
        for word in ["Private Client Backup", "/home/ada", "piston-data", "javac"] {
            assert!(lines.iter().all(|l| !l.contains(word)), "{word}: {lines:?}");
        }
        assert_eq!(
            crate::launcher_log::preparer_line("Preparing fixture resources (3 of 40)"),
            "Preparing: Preparing fixture resources (3 of 40)"
        );
    }
}
