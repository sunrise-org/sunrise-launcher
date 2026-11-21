//! The dedicated CLI must refuse a profile before touching EULA, even on the same build.
use super::{execute, Cli};
use clap::Parser;
use std::{fs, path::Path, process::Command};
use sunrise_launcher::backend;

const DIRECTORY: &str = "SUNRISE_TEST_DEDICATED_DIRECTORY";
const TAG: &str = "nightly-same-build";
const CLIENT: &str = if cfg!(windows) {
    "sunrise-client.exe"
} else {
    "sunrise-client"
};

fn link_dir(target: &Path, alias: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, alias).unwrap();
    #[cfg(windows)]
    {
        // The runner can put forward slashes in its temporary-directory path.
        let native = |path: &Path| path.components().collect::<std::path::PathBuf>();
        sunrise_launcher::sources::junction(&native(target), &native(alias)).unwrap();
    }
}

#[test]
fn dedicated_cli_serves_a_separate_folder_but_refuses_profile_aliases_before_eula() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let game = root.join("game");
    let data = root.join("launcher");
    let own = game.join("instances/friends");
    let pack = game.join("sunrise-packs/adventures/home");
    let external = root.join("external-profile");
    let alias = root.join("alias");
    let accepted = root.join("dedicated/new-server");
    let image = data.join("versions").join(TAG);
    let resources = image
        .join(".local/sunrise-client")
        .join(backend::PINNED_GAME);
    for folder in [&own, &pack, &external, &resources] {
        fs::create_dir_all(folder).unwrap();
    }
    link_dir(&own, &alias);
    link_dir(&external, &game.join("instances/external"));
    fs::write(image.join(".installed"), []).unwrap();
    fs::write(resources.join("sunrise-resources.json"), "{}").unwrap();
    fs::write(
        data.join("instances.json"),
        r#"{"instances":{"old-slug":{}}}"#,
    )
    .unwrap();

    let source = root.join("actor.rs");
    fs::write(&source, r#"
fn main() {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    assert_eq!(args.len(), 4);
    assert_eq!(args[0], "--resources");
    assert!(std::path::Path::new(&args[1]).join("sunrise-resources.json").is_file());
    assert_eq!(args[2], "--dedicated");
    assert_eq!(args[3], "--nogui");
    let expected = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_DEDICATED_DIRECTORY").unwrap()).canonicalize().unwrap();
    assert_eq!(std::env::current_dir().unwrap().canonicalize().unwrap(), expected);
    assert_eq!(std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap()).canonicalize().unwrap(), expected);
    assert!(std::env::var_os("SUNRISE_ACCOUNT_SESSIONS").is_none());
    let data = std::path::PathBuf::from(std::env::var_os("SUNRISE_LAUNCHER_HOME").unwrap());
    let exe = std::env::current_exe().unwrap().canonicalize().unwrap();
    #[cfg(windows)]
    {
        let slot = std::path::PathBuf::from(std::env::var_os("SUNRISE_WINDOWS_RUNTIME_SLOT").unwrap()).canonicalize().unwrap();
        assert_eq!(slot.parent().unwrap(), data.canonicalize().unwrap().join("run"));
        assert_eq!(exe, slot.join("game/sunrise-client.exe"));
    }
    #[cfg(not(windows))]
    assert_eq!(exe, data.canonicalize().unwrap().join("versions/nightly-same-build/sunrise-client"));
    assert!(std::fs::read_to_string("eula.txt").unwrap().contains("eula=true"));
    std::fs::write("server-ran", "yes").unwrap();
}
"#).unwrap();
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    assert!(Command::new(rustc)
        .arg(&source)
        .arg("-o")
        .arg(image.join(CLIENT))
        .status()
        .unwrap()
        .success());

    let cli = |dir: &Path| {
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "cli::server_tests::dedicated_cli_actor",
                "--nocapture",
            ])
            .env(DIRECTORY, dir)
            .envs((dir == accepted).then_some(("SUNRISE_TEST_DEDICATED_CHECK_COMMAND", "1")))
            .env("SUNRISE_HOME", &game)
            .env("SUNRISE_LAUNCHER_HOME", &data)
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("SUNRISE_NO_AUDIO", "1")
            .env("SUNRISE_ACCOUNT_SESSIONS", game.join("sessions"))
            .output()
            .unwrap()
    };
    for folder in [
        &game,
        &own,
        &pack,
        &alias,
        &external,
        &alias.join("missing"),
        root,
    ] {
        let output = cli(folder);
        let err = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{folder:?}: {err}");
        assert!(
            err.contains("separate from a game profile"),
            "{folder:?}: {err}"
        );
        assert!(!folder.join("eula.txt").exists());
        assert!(!folder.join("server-ran").exists());
    }
    #[cfg(windows)]
    {
        let different_case = game.with_file_name("GAME");
        let output = cli(&different_case);
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains("separate from a game profile"));
        assert!(!game.join("eula.txt").exists());
    }
    #[cfg(unix)]
    {
        let file = root.join("not-a-profile.txt");
        fs::write(&file, "not a profile").unwrap();
        std::os::unix::fs::symlink(&file, game.join("instances/file-alias")).unwrap();
        std::os::unix::fs::symlink(&file, game.join("sunrise-packs/file-alias")).unwrap();
        let other_pack = game.join("sunrise-packs/another-pack");
        fs::create_dir_all(&other_pack).unwrap();
        std::os::unix::fs::symlink(&file, other_pack.join("home")).unwrap();
        let dangling = game.join("instances/dangling");
        std::os::unix::fs::symlink(root.join("missing-profile"), &dangling).unwrap();
        let unreconciled = root.join("dedicated/unresolved-profile-check");
        let output = cli(&unreconciled);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!unreconciled.join("eula.txt").exists());
        assert!(!unreconciled.join("server-ran").exists());
        fs::remove_file(dangling).unwrap();
    }
    let output = cli(&accepted);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(accepted.join("server-ran")).unwrap(), b"yes");
    let profile_file = own.join("keep-unchanged.txt");
    fs::write(&profile_file, "profile sentinel").unwrap();
    let hard_link = root.join("dedicated/hard-link-eula");
    fs::create_dir(&hard_link).unwrap();
    fs::hard_link(&profile_file, hard_link.join("eula.txt")).unwrap();
    #[cfg(unix)]
    let sym_link = {
        let folder = root.join("dedicated/symlink-eula");
        fs::create_dir(&folder).unwrap();
        std::os::unix::fs::symlink(&profile_file, folder.join("eula.txt")).unwrap();
        folder
    };
    let assert_replaced = |folder: &Path| {
        let output = cli(folder);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(fs::read(folder.join("server-ran")).unwrap(), b"yes");
        assert!(backend::eula_agreed(folder));
        assert_eq!(fs::read(&profile_file).unwrap(), b"profile sentinel");
    };
    assert_replaced(&hard_link);
    #[cfg(unix)]
    assert_replaced(&sym_link);
    assert_eq!(
        fs::read_to_string(data.join("instances.json")).unwrap(),
        r#"{"instances":{"old-slug":{}}}"#
    );
}

#[test]
fn dedicated_cli_actor() {
    let Some(dir) = std::env::var_os(DIRECTORY) else {
        return;
    };
    if std::env::var_os("SUNRISE_TEST_DEDICATED_CHECK_COMMAND").is_some() {
        let probe = Path::new(&dir).with_file_name("effective-command-test");
        fs::create_dir_all(&probe).unwrap();
        let game = backend::game_home();
        let command = backend::server_command(TAG, &probe).unwrap();
        let (command, _runtime) = backend::prepare_server_command(TAG, &probe, command).unwrap();
        backend::verify_server_command(TAG, &probe, &command).unwrap();
        let clone = |program: &std::ffi::OsStr| {
            let mut copy = Command::new(program);
            copy.args(command.get_args())
                .current_dir(command.get_current_dir().unwrap());
            for (name, value) in command.get_envs() {
                if let Some(value) = value {
                    copy.env(name, value);
                } else {
                    copy.env_remove(name);
                }
            }
            copy
        };
        let forged = clone(std::env::current_exe().unwrap().as_os_str());
        assert!(backend::verify_server_command(TAG, &probe, &forged).is_err());
        let mut forged = clone(command.get_program());
        forged.env("SUNRISE_HOME", &game);
        assert!(backend::verify_server_command(TAG, &probe, &forged).is_err());
        let mut forged = clone(command.get_program());
        forged.current_dir(&game);
        assert!(backend::verify_server_command(TAG, &probe, &forged).is_err());
        let mut forged = clone(command.get_program());
        forged.arg("--home").arg(&game);
        assert!(backend::verify_server_command(TAG, &probe, &forged).is_err());
        #[cfg(windows)]
        {
            let mut forged = clone(command.get_program());
            forged.env("SUNRISE_WINDOWS_RUNTIME_SLOT", &game);
            assert!(backend::verify_server_command(TAG, &probe, &forged).is_err());
        }
    }
    let args = [
        "sunrise-launcher",
        "server",
        "--version",
        TAG,
        "--accept-eula",
        "--dir",
    ];
    let cli =
        Cli::try_parse_from(args.into_iter().map(std::ffi::OsString::from).chain([dir])).unwrap();
    let code = match execute(cli.command.unwrap(), cli.json) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    };
    std::process::exit(code);
}
