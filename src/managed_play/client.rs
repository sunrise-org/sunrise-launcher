//! Ordinary client Play keeps one admission on its worker through the actual spawn.
use super::{
    catalogue,
    first::{assert_requested, game_lock},
    unsettled, Active, FirstBuild, Profile,
};
use crate::{
    account::{self, Accounts, NoOwner},
    backend::{self, installation::BoundVariant, RuntimeGuard},
    browser::installed::GameInstallProblem,
    developer_offline, instances, native_mods,
    process::child::Child,
};
use std::process::Command;

pub enum Progress {
    Download(f32),
    Prepare,
    Native(native_mods::Step),
    SignIn,
    Line(String),
    HookLine(String),
}

#[derive(Debug)]
pub enum StartError {
    Journal(GameInstallProblem),
    Account(NoOwner),
    Native(native_mods::Failed),
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
            StartError::Native(error) => error.into(),
            StartError::Hook(error) => error.to_string(),
            StartError::UnknownSpawn(error) | StartError::Other(error) => error,
        }
    }
}

type NativeConsent<'a> = Option<&'a dyn Fn(&native_mods::Consent) -> bool>;

pub struct Client<'a> {
    pub instance: &'a instances::Instance,
    pub accepted_account: Option<&'a str>,
    /// An explicit command-line account still requires ownership in a developer build.
    pub explicit_account: bool,
    pub requested_build: Option<&'a str>,
    pub piped: bool,
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
        native_consent: NativeConsent<'_>,
        progress: impl Fn(Progress),
    ) -> Result<Started, StartError> {
        // A fresh default profile may not have a game directory yet. Do not recreate
        // a named profile that disappeared while its Play awaited this worker.
        if self.instance.kind == instances::Kind::Pack {
            return Err(unsettled().into());
        }
        if self.accepted_account.is_none() && !developer_offline::requested() {
            return Err(StartError::Account(NoOwner::default()));
        }
        if self.instance.kind == instances::Kind::Default {
            std::fs::create_dir_all(&self.instance.home)
                .map_err(|error| super::at(&self.instance.home, error))?;
        }
        let mut profile =
            Profile::open_instance(&backend::game_home(), &backend::data_dir(), self.instance)?;
        let instance = profile.admitted_instance(self.instance);
        let cancellation = native_mods::Cancellation::default();
        let check_build = |tag: &str, variant: Option<&BoundVariant>| {
            if self.accepted_account.is_none() && !developer_offline::enabled(tag) {
                return Err(StartError::Account(NoOwner::default()));
            }
            backend::managed_game_architecture_launch_error(tag, variant)
                .map_or(Ok(()), |error| Err(StartError::Other(error)))
        };
        let (active, registry_notices) = self.ready(
            &mut profile,
            choose,
            check_build,
            (&cancellation, native_consent),
            &progress,
            || {
                use crate::browser::sunrise;
                sunrise::index().ok()?;
                sunrise::fetched_index()
            },
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
                &cancellation,
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
        (cancellation, native_consent): (&native_mods::Cancellation, NativeConsent<'_>),
        progress: impl Fn(Progress),
        load_catalogue: impl FnOnce() -> Option<std::sync::Arc<sunrise_launcher_smod::index::Index>>,
    ) -> Result<(Active, Vec<String>), StartError> {
        let home = profile.home().to_owned();
        let mode = self.instance.native_profile;
        let native = |step| progress(Progress::Native(step));
        if let Some(old) = profile.active()? {
            assert_requested(self.requested_build, old.base(), old.executable())?;
            check_build(old.base(), old.variant.as_deref())?;
            // Legacy writers still use the game lock, not profile admission. Keep
            // them out until refresh has staged the exact set we checked.
            let _manifest = game_lock(&home)?;
            crate::browser::installed::recover_game_install(&home).map_err(StartError::Journal)?;
            let notices = catalogue::inspect(&home, load_catalogue)?;
            let active = profile
                .refresh(mode, cancellation, native, native_consent)
                .map_err(StartError::Native)?
                .ok_or_else(unsettled)?;
            return Ok((active, notices));
        }

        let first = profile.begin_requested(self.requested_build, choose, |fraction| {
            progress(Progress::Download(fraction));
        })?;
        check_build(first.base(), first.variant().as_ref())?;
        progress(Progress::Prepare);
        first.prepare(&|line| progress(Progress::Line(line)))?;
        let (image, notices) = {
            let _manifest = game_lock(&home)?;
            crate::browser::installed::recover_game_install(&home).map_err(StartError::Journal)?;
            first.verify_inputs()?;
            let notices = catalogue::inspect(&home, load_catalogue)?;
            let image = first
                .exact_image(mode, cancellation, native, native_consent)
                .map_err(StartError::Native)?;
            (image, notices)
        };
        // Resource preparation takes a version lock. Don't take it under the game
        // manifest lock; commit rechecks the frozen input epoch before publication.
        Ok((first.commit(&image, cancellation)?, notices))
    }
}

#[cfg(test)]
mod tests;

/// Resolve an ordinary headless first Play lazily. An existing committed or pending
/// build bypasses the catalogue completely; an explicit first selection remains exact.
pub fn headless_first(
    instance: &instances::Instance,
    requested: Option<&str>,
) -> Result<FirstBuild, String> {
    let installed = match instance.resolve_version(requested) {
        Ok(tag) => return FirstBuild::installed(&tag),
        Err(error) => error,
    };
    let releases = backend::releases()?;
    let release = if let Some(tag) = requested.or(instance.version.as_deref()) {
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
