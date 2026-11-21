use super::*;
use serde_json::json;
use std::{fs, os::unix::fs::PermissionsExt, process::Command};
use sunrise_launcher_nbt::{self as nbt, Limits, NamedTag, Tag};
use sunrise_launcher_smod::{index, Package};

#[test]
fn github_update_cli_checks_origin_and_world_approval_before_publishing() {
    const ACTOR: &str = "SUNRISE_TEST_GITHUB_UPDATE_CLI";
    let root = match std::env::var_os(ACTOR) {
        Some(root) => std::path::PathBuf::from(root),
        None => {
            let root = tempfile::tempdir().unwrap();
            let test_module = module_path!().split_once("::").unwrap().1;
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    &format!("{test_module}::github_update_cli_checks_origin_and_world_approval_before_publishing"),
                ])
                .env(ACTOR, root.path())
                .env("SUNRISE_HOME", root.path().join("game"))
                .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
                .env("SUNRISE_LAUNCHER_DEVELOPER_OFFLINE", "1")
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            return;
        }
    };
    const ID: &str = "github_lantern";
    const TAG: &str = "developer-local";
    let archive = |version: &str| {
        let manifest = format!(
            "id='{ID}'\nname='GitHub Lantern'\nversion='{version}'\napi='0.2'\nside='both'\ncapabilities=['registries']\n[native]\nsunrise='{}'\nmixins='{}'\ngame='26.3'\n",
            "a".repeat(40),
            "c".repeat(40),
        );
        let cargo = format!("[package]\nname='{ID}'\nversion='{version}'\n");
        Package::write_files(
            &manifest,
            &[("Cargo.toml", cargo.as_bytes()), ("src/lib.rs", b"")],
        )
        .unwrap()
    };
    let old = archive("1.0.0");
    let new = archive("1.1.0");
    let home = root.join("game");
    let data = root.join("launcher");
    let build = data.join("versions").join(TAG);
    let resources = build.join(".local/sunrise-client/26.3");
    fs::create_dir_all(&resources).unwrap();
    fs::write(resources.join("sunrise-resources.json"), "{}").unwrap();
    fs::write(build.join(".installed"), []).unwrap();
    fs::write(
        build.join("developer.json"),
        json!({"revision":"a".repeat(40)}).to_string(),
    )
    .unwrap();
    fs::write(
        build.join("research-lock.json"),
        r#"{"minecraft":{"version":"26.3"}}"#,
    )
    .unwrap();
    let client = build.join("sunrise-client");
    fs::write(
        &client,
        "#!/bin/sh\nprintf '%s\\n' '{\"options\":[{\"name\":\"--developer-single-player\"}]}'\n",
    )
    .unwrap();
    fs::set_permissions(&client, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(
        data.join("instances.json"),
        json!({"instances":{"sunrise":{"version":TAG}}}).to_string(),
    )
    .unwrap();
    fs::write(
        data.join("settings.json"),
        r#"{"native_mods_allowed_at":123,"native_consent_revision":1}"#,
    )
    .unwrap();

    // An explicitly off, unheld archive still warns for saved worlds. This fixture proves
    // the real transaction without asking the libtest executable to run the compiler worker.
    let dir = home.join(native_mods::INSTALLED);
    fs::create_dir_all(&dir).unwrap();
    let target = dir.join(format!("{ID}.smod"));
    fs::write(&target, &old).unwrap();
    let sidecar = target.with_extension("origin.json");
    let mut origin = json!({"id":ID,"version":"1.0.0","owner":"fixture","repo":"lantern",
        "tag":"v1.0.0","commit":"a".repeat(40),"asset":"lantern-1.0.0.smod","sha256":"0".repeat(64)});
    fs::write(&sidecar, origin.to_string()).unwrap();
    let world = home.join("saves/Previous world");
    fs::create_dir_all(&world).unwrap();
    let level = NamedTag {
        name: "".into(),
        tag: Tag::compound([(
            "Data",
            Tag::compound([("LevelName", Tag::String("Previous world".into()))]),
        )]),
    };
    fs::write(
        world.join("level.dat"),
        nbt::write_named(&level, Limits::default()).unwrap(),
    )
    .unwrap();

    const URL: &str =
        "https://github.com/fixture/lantern/releases/download/v1.1.0/lantern-1.1.0.smod";
    let found = github_mods::resolve(&github_mods::parse("https://github.com/fixture/lantern").unwrap(), &|path| {
        Ok(Some(match path {
            "repos/fixture/lantern/releases/latest" => json!({"tag_name":"v1.1.0","assets":[{
                "name":"lantern-1.1.0.smod","browser_download_url":URL,"digest":format!("sha256:{}", index::sha256(&new))
            }]}),
            "repos/fixture/lantern/commits/v1.1.0" => json!({"sha":"b".repeat(40)}),
            _ => panic!("unexpected GitHub API request: {path}"),
        }))
    }).unwrap();
    let instance = instances::selection().unwrap();
    let invoke = |extra: &[&str]| {
        let args = [
            &["sunrise-launcher", "mods", "update-github", ID][..],
            extra,
        ]
        .concat();
        let cli = Cli::try_parse_from(args).unwrap();
        let Some(Cmd::Mods {
            action:
                Some(ModsCmd::UpdateGithub {
                    id,
                    trust_native_code,
                    confirm_world_loss,
                    ..
                }),
        }) = cli.command
        else {
            panic!("expected the Update GitHub mod command");
        };
        github_native_from(
            GithubInstall {
                instance: &instance,
                found: &found,
                only: id.as_deref(),
                all: false,
                update: true,
                trust: trust_native_code,
                confirm_world_loss,
            },
            |found| {
                github_mods::fetch_with(found, |url| {
                    assert_eq!(url, URL);
                    Ok(new.clone())
                })
            },
        )
    };
    let pointer = home.join("mod-browser/active-build.json");
    let denied = invoke(&["--confirm-world-loss"]).unwrap_err();
    assert!(
        denied.contains("GitHub") && !denied.contains("Nothing was updated."),
        "{denied}"
    );
    assert_eq!(fs::read(&target).unwrap(), old);
    assert_eq!(fs::read_to_string(&sidecar).unwrap(), origin.to_string());
    assert!(!pointer.exists());

    origin["sha256"] = json!(index::sha256(&old));
    fs::write(&sidecar, origin.to_string()).unwrap();
    let denied = invoke(&[]).unwrap_err();
    assert_eq!(denied, "Nothing was updated.");
    assert_eq!(fs::read(&target).unwrap(), old);
    assert_eq!(fs::read_to_string(&sidecar).unwrap(), origin.to_string());
    assert!(!pointer.exists());

    invoke(&["--confirm-world-loss"]).unwrap();
    assert_eq!(fs::read(&target).unwrap(), new);
    let published = github_mods::origin(&home, ID).unwrap();
    assert_eq!(published.asset.as_deref(), Some("lantern-1.1.0.smod"));
    assert_eq!(published.sha256, index::sha256(&new));
    assert!(pointer.is_file());
    assert!(!sunrise_launcher::browser::mod_list::is_enabled(&home, ID));
}
