//! Ordinary client Play keeps one admission on its worker through the actual spawn.
use super::{catalogue, first::game_lock, unsettled, Active, FirstBuild, Profile};
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
            StartError::UnknownSpawn(error) | StartError::Other(error) => error,
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
        let (active, registry_notices) = self.ready(
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
        progress(Progress::SignIn);
        let notice;
        let session_file = if offline {
            developer_offline::require_capability(tag)?;
            developer_offline::mark(&mut cmd);
            notice = Some(crate::i18n::tr(developer_offline::NOTICE).into());
            None
        } else {
            let uuid = self
                .accepted_account
                .ok_or_else(|| StartError::Account(NoOwner::default()))?;
            let session = account::session_for_game_accepted(&backend::game_home(), uuid)
                .map_err(StartError::Account)?;
            let session_file = session.file.clone();
            notice = if let Some(reason) = session.apply(&mut cmd) {
                Some(crate::i18n::trf(
                    "Playing offline: {reason}",
                    &[("reason", &reason)],
                ))
            } else {
                account::take_unsaved()
            };
            session_file
        };
        account::retain_launch_session(session_file.as_deref());
        let launch = &self.instance.launch;
        let started = (|| {
            arguments(&mut cmd)?;
            profile.spawn_checked_account(
                &active,
                cancellation,
                cmd,
                (launch, self.piped, &|line| {
                    progress(Progress::HookLine(line))
                }),
                prepare_jars,
                || {
                    if let Some(uuid) = self.accepted_account.filter(|_| !offline) {
                        account::check_launch_account(&Accounts::load(), uuid)
                            .map_err(|refused| refused.in_window())?;
                    }
                    Ok(())
                },
            )
        })();
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
    ) -> Result<(Active, Vec<String>), StartError> {
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
            check_build(old.base(), old.variant.as_deref())?;
            // Prepare without the game-manifest lock. The profile stays admitted and the
            // saved generation continues to reserve this base until refresh commits.
            let replacement = match old.resources.check_play_output(old.base()) {
                Ok(()) => None,
                Err(backend::Unprepared::Incomplete | backend::Unprepared::ServerOnly) => {
                    // The preparer is part of the saved image. Verify it before starting it.
                    old.verify_artifacts()?;
                    progress(Progress::Prepare);
                    Some(super::Resources::for_play(
                        old.base(),
                        profile.handoff(),
                        &|line| progress(Progress::Line(line)),
                    )?)
                }
                Err(issue) => return Err(issue.message(|| unsettled!()).into()),
            };
            // Legacy writers still use the game lock, not profile admission. Keep
            // them out until refresh has staged the exact set we checked.
            let _manifest = game_lock(&home)?;
            super::recover_game_install(&home).map_err(StartError::Journal)?;
            let notices = catalogue::inspect(&home, load_index)?;
            let active = profile
                .refresh(
                    mode,
                    cancellation,
                    |step| native(old.base(), step),
                    Some(&log),
                    replacement,
                )
                .map_err(|error| native_failure(&home, error, || Some(Build::Active(old))))?
                .ok_or_else(|| unsettled!())?;
            return Ok((active, notices));
        }

        let ready = (|| {
            let first = profile.begin_first(choose, |fraction| {
                progress(Progress::Download(fraction));
            })?;
            check_build(first.base(), first.variant().as_ref())?;
            progress(Progress::Prepare);
            first.prepare(&|line| progress(Progress::Line(line)))?;
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
            // Resource preparation takes a version lock. Don't take it under the game
            // manifest lock; commit rechecks the frozen input epoch before publication.
            Ok((first.commit(&image, cancellation)?, notices))
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
            eprintln!("checking saved worlds before turning off mods: {error}");
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
