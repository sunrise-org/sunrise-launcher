//! Build the actual admitted client command, then transfer its profile and cache claims.
use super::{first::VerifiedSnapshot, unsettled, Active, Profile, StartError};
use crate::{
    backend::{self, RuntimeGuard},
    native_mods::Cancellation,
    process::{child::Child, leases::ChildLeases},
    run_leases,
};
use cap_std::{ambient_authority, fs::Dir};
use std::{io, path::Path, process::Command};
use sunrise_launcher_smod::{
    holder,
    selection::{Argument, LaunchArguments},
};

struct SpawnHooks<'a, J, B> {
    proof: Option<VerifiedSnapshot>,
    prepare_jars: J,
    hook: Option<(&'a crate::launch::Launch, &'a dyn Fn(String))>,
    before_spawn: B,
}

impl Profile {
    /// The caller applies account, profile and player settings before this boundary. This
    /// method accepts the final, unwrapped game command; it never starts an outer wrapper.
    pub fn spawn_direct(
        &self,
        active: &Active,
        cancellation: &Cancellation,
        command: Command,
        piped: bool,
    ) -> Result<(Child, RuntimeGuard), String> {
        self.spawn_selected(
            active,
            cancellation,
            command,
            &[],
            piped,
            SpawnHooks {
                proof: None,
                prepare_jars: |_: &Command| Ok(()),
                hook: None,
                before_spawn: |_: &mut Command| Ok(()),
            },
        )
        .map_err(String::from)
    }

    /// Apply instance switches and hooks, then start either the exact image or the
    /// one-time relay authenticated by this same launcher admission.
    pub fn spawn_configured(
        &self,
        active: &Active,
        cancellation: &Cancellation,
        command: Command,
        launch: &crate::launch::Launch,
        piped: bool,
    ) -> Result<(Child, RuntimeGuard), StartError> {
        self.spawn_checked_account(
            active,
            cancellation,
            command,
            (launch, piped, &|_| {}, None),
            |_| Ok(()),
            |_| Ok(()),
        )
    }

    pub(super) fn spawn_checked_account(
        &self,
        active: &Active,
        cancellation: &Cancellation,
        mut command: Command,
        (launch, piped, hook_log, proof): (
            &crate::launch::Launch,
            bool,
            &dyn Fn(String),
            Option<VerifiedSnapshot>,
        ),
        prepare_jars: impl FnOnce(&Command) -> Result<(), String>,
        before_spawn: impl FnOnce(&mut Command) -> Result<(), StartError>,
    ) -> Result<(Child, RuntimeGuard), StartError> {
        let wrapper = crate::launch::managed_wrapper(&mut command, launch);
        self.spawn_selected(
            active,
            cancellation,
            command,
            &wrapper,
            piped,
            SpawnHooks {
                proof,
                prepare_jars,
                hook: Some((launch, hook_log)),
                before_spawn,
            },
        )
    }

    fn spawn_selected<J, B>(
        &self,
        active: &Active,
        cancellation: &Cancellation,
        command: Command,
        wrapper: &[std::ffi::OsString],
        piped: bool,
        hooks: SpawnHooks<'_, J, B>,
    ) -> Result<(Child, RuntimeGuard), StartError>
    where
        J: FnOnce(&Command) -> Result<(), String>,
        B: FnOnce(&mut Command) -> Result<(), StartError>,
    {
        let SpawnHooks {
            proof,
            prepare_jars,
            hook,
            before_spawn,
        } = hooks;
        let proof = if let Some((launch, hook_log)) =
            hook.filter(|(launch, _)| !launch.pre_launch.trim().is_empty())
        {
            // Run the synchronous pre-launch hook before acquiring the manifest lock
            // (dropping any seal proof's lock guard first) so any helper or subcommand
            // acquiring `.mod-browser.lock` cannot deadlock, then verify the committed
            // profile, binaries, and prepared resources once before spawn.
            drop(proof);
            crate::launch::managed_pre_launch(&command, launch, piped, hook_log)
                .map_err(StartError::Hook)?;
            None
        } else {
            proof
        };
        let _manifest = self.verify_handoff(active, proof, cancellation)?;
        let selection = validate_selection(self.home(), &active.resource_path(), &command)?;
        prepare_jars(&command)?;
        cancellation.check()?;
        let (mut command, runtime) = prepare(self.home(), active, command)?;
        let data = backend::data_dir();
        let data = data
            .canonicalize()
            .map_err(|error| super::at(&data, error))?;
        let cwd = command
            .get_current_dir()
            .expect("verified client working directory");
        let image = Path::new(command.get_program());
        let mut trees = holder::managed_lease_paths(image, &selection, &data, cwd)
            .map_err(|error| super::at(image, error))?;
        // The runtime copy runs from its own folder; also retain the original install.
        let source = backend::version_dir(active.executable());
        let source = source
            .canonicalize()
            .map_err(|error| super::at(&source, error))?;
        for parent in source
            .ancestors()
            .take_while(|parent| parent.starts_with(&data) && *parent != data)
        {
            if !trees.iter().any(|(path, _)| path == parent) {
                trees.push((parent.to_owned(), parent == source));
            }
        }
        // A capable game still receives the recording/capture protocol of legacy Play.
        let record = if holder::capable(image).map_err(|error| super::at(image, error))? {
            let record = holder::RunRecord::new(data.clone(), trees.clone(), &selection.context)
                .map_err(|error| super::at(&data, error))?;
            let directory = record
                .direct_entry(&mut command)
                .map_err(|error| super::at(&data, error))?;
            trees.push((directory.to_owned(), false));
            Some(record)
        } else {
            None
        };
        let dirs = trees
            .into_iter()
            .map(|(path, _)| Dir::open_ambient_dir(path, ambient_authority()))
            .collect::<io::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        let leases = ChildLeases::acquire(&data, &dirs).map_err(|error| error.to_string())?;
        #[cfg(windows)]
        let leases = {
            let mut leases = leases;
            leases
                .retain(runtime.windows()?.files())
                .map_err(|error| error.to_string())?;
            leases
        };
        #[cfg(unix)]
        if piped {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        // Last, so the game's session file is as young as it can be: the checks above can
        // take longer than the game keeps one.
        before_spawn(&mut command)?;
        let profile = self.handoff();
        let child = if wrapper.is_empty() {
            let program = command.get_program().to_string_lossy().into_owned();
            leases
                .spawn_game(command, profile, piped)
                .map_err(|error| {
                    crate::i18n::trf(
                        "Could not run {program}: {error}",
                        &[("program", &program), ("error", &error.to_string())],
                    )
                })?
        } else {
            crate::process::relay::start(&command, wrapper, &leases, profile, piped)?
        };
        drop(record);
        Ok((child, runtime))
    }
}

fn prepare(
    home: &Path,
    active: &Active,
    mut command: Command,
) -> Result<(Command, RuntimeGuard), String> {
    let source = backend::version_dir(active.executable());
    let program = Path::new(command.get_program());
    let expected = source.join(backend::CLIENT);
    #[cfg(target_os = "macos")]
    let expected = if program
        == source
            .join("Sunrise.app/Contents/MacOS")
            .join(backend::CLIENT)
    {
        crate::game_app::verify_links(&source, program)
            .map_err(|error| super::at(program, error))?;
        program.to_owned()
    } else {
        expected
    };
    if program != expected {
        return Err(unsettled!());
    }
    // The source program was hashed by verified_handoff; a Mac bundle must hard-link it.
    // None of these fields may be selected by inherited environment or by instance Settings.
    for key in [
        run_leases::GAME,
        holder::ENV,
        holder::ENTRY,
        "SUNRISE_RUNTIME_DATA_ROOT",
        "SUNRISE_RUNTIME_LEASE_FDS",
        "SUNRISE_RUNTIME_LEASE_HANDLES",
        "SUNRISE_ACCOUNT_SESSIONS",
    ] {
        command.env_remove(key);
    }
    #[cfg(windows)]
    command.env_remove(crate::windows_runtime::SLOT);
    command.env("SUNRISE_HOME", home);
    run_leases::mark(&mut command, &source);
    if !active.allow_accelerator() {
        command.env("SUNRISE_NO_DATA_FILE", "1");
    }
    let (command, runtime) = backend::prepare_command(command)?;
    #[cfg(windows)]
    active.verify_runtime_copy(runtime.windows()?.game())?;
    Ok((command, runtime))
}

fn validate_selection(
    home: &Path,
    resources: &Path,
    command: &Command,
) -> Result<LaunchArguments, String> {
    let invalid = || {
        crate::i18n::tr("The game arguments change the folders or mods selected for this profile.")
            .to_owned()
    };
    let same =
        |a: &Path, b: &Path| -> io::Result<bool> { Ok(a.canonicalize()? == b.canonicalize()?) };
    let selection = LaunchArguments::for_command(command).map_err(|error| error.to_string())?;
    let cwd = command.get_current_dir().ok_or_else(invalid)?;
    if !same(cwd, home).unwrap_or(false)
        || !same(&selection.context.game_dir, home).unwrap_or(false)
        || !same(&selection.context.home, home).unwrap_or(false)
        || !selection.context.dev_mods.is_empty()
        || selection.is_information_request()
        || selection.installation_side() != sunrise_launcher_smod::manifest::Side::Client
    {
        return Err(invalid());
    }
    let mut found = false;
    for arg in &selection.rest {
        if let Argument::Value(name, value) = arg {
            if name == "--resources" {
                if !same(&cwd.join(value), resources).unwrap_or(false) {
                    return Err(invalid());
                }
                found = true;
            }
        }
    }
    if !found {
        return Err(invalid());
    }
    Ok(selection)
}
