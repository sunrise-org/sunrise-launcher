//! Real launcher clients, a local unsigned game fixture, and no production credentials.
#![cfg(unix)]

use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::{OpenOptionsExt, PermissionsExt},
        net::UnixListener,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use sunrise_launcher_smod::{manifest, mod_list};

const BUILD: &str = "developer-local";

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    data: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let home = root.join("game");
        let data = root.join("launcher");
        let build = data.join("versions").join(BUILD);
        let resources = build.join(".local/sunrise-client/26.3");
        fs::create_dir_all(&resources).unwrap();
        fs::create_dir(&home).unwrap();
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
        fs::write(resources.join("sunrise-resources.json"), "{}").unwrap();
        script(
            &build.join("sunrise-client"),
            r#"#!/bin/sh
set -eu
if [ "$1" = --launch-api ]; then
  printf '%s\n' '{"options":[{"name":"--developer-single-player"}]}'
  exit 0
fi
printf '%s\n' "${SUNRISE_SESSION_FILE-unset}" "${SUNRISE_LAUNCHER_EXE-unset}" "${SUNRISE_LAUNCHER_DEVELOPER_OFFLINE-unset}" "${FIXTURE_INSTANCE_ENV-unset}" > "$SUNRISE_HOME/game-env"
{ pwd; printf '%s\n' "$@"; } > "$SUNRISE_HOME/args.tmp"
mv "$SUNRISE_HOME/args.tmp" "$SUNRISE_HOME/args"
n=0
while [ ! -f "$SUNRISE_HOME/finish" ] && [ "$n" -lt 1000 ]; do
  sleep 0.02
  n=$((n+1))
done
exit 7
"#,
        );
        let fixture = Self {
            _temp: temp,
            root,
            home,
            data,
        };
        fixture.configure(json!({}));
        fixture
    }

    fn configure(&self, launch: Value) {
        fs::write(
            self.data.join("instances.json"),
            json!({"instances":{"sunrise":{"version":BUILD,"launch":launch}}}).to_string(),
        )
        .unwrap();
    }

    fn launcher(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_sunrise-launcher"));
        cmd.args(args)
            .env("SUNRISE_HOME", &self.home)
            .env("SUNRISE_LAUNCHER_HOME", &self.data)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_DATA_HOME", self.root.join("xdg"))
            .env("SUNRISE_LAUNCHER_DEVELOPER_OFFLINE", "1")
            .env("SUNRISE_NO_KEYCHAIN", "1")
            .env("SUNRISE_TELEMETRY_URL", "off");
        cmd
    }

    fn game_arguments(&self) -> Vec<String> {
        await_file(&self.home.join("args"));
        fs::read_to_string(self.home.join("args"))
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn pointer(&self) -> Vec<u8> {
        fs::read(self.home.join("mod-browser/active-build.json")).unwrap()
    }
}

fn script(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn await_file(path: &Path) {
    let limit = Instant::now() + Duration::from_secs(25);
    while !path.is_file() {
        assert!(
            Instant::now() < limit,
            "the launcher did not reach {path:?}"
        );
        std::thread::sleep(Duration::from_millis(15));
    }
}

struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn assert_resource_args(fixture: &Fixture, args: &[String]) {
    assert_eq!(PathBuf::from(&args[0]), fixture.home);
    let resource_path = args
        .windows(2)
        .find(|pair| pair[0] == "--resources")
        .unwrap();
    let path = Path::new(&resource_path[1]);
    assert!(
        path.starts_with(
            fixture
                .data
                .join("versions")
                .join(BUILD)
                .join(".local/launcher-prepared")
        ),
        "{path:?}"
    );
    assert_eq!(
        fs::read_to_string(path.join("sunrise-resources.json")).unwrap(),
        "{}"
    );
    assert!(args.contains(&"--developer-single-player".to_owned()));
    assert!(!path.join(".last-used").exists());
}

fn mcp_request(
    to_mcp: &mut impl Write,
    from_mcp: &mut impl BufRead,
    id: u32,
    method: &str,
    params: Value,
) -> Value {
    writeln!(
        to_mcp,
        "{}",
        json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
    )
    .unwrap();
    let mut line = String::new();
    assert_ne!(from_mcp.read_line(&mut line).unwrap(), 0);
    let result: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(result["id"], id);
    result["result"].clone()
}

#[test]
fn cli_first_play_uses_the_relay_and_keeps_the_same_build_for_an_already_open_mcp() {
    let fixture = Fixture::new();
    let wrapper = fixture.root.join("wrapper");
    script(
        &wrapper,
        "#!/bin/sh\nprintf 'wrapped' > \"$SUNRISE_HOME/wrapper-ran\"\n\"$@\"\nexit 43\n",
    );
    fixture.configure(json!({
        "wrapper":format!("\"{}\"",wrapper.display()),
        "post_exit":"printf 'game ended' > post-exit",
    }));
    let mut mcp = Running(
        fixture
            .launcher(&["mcp"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut to_mcp = mcp.0.stdin.take().unwrap();
    let mut from_mcp = BufReader::new(mcp.0.stdout.take().unwrap());
    let mut rpc = |id, method, params| mcp_request(&mut to_mcp, &mut from_mcp, id, method, params);
    rpc(1, "initialize", json!({}));
    let mut cli = Running(
        fixture
            .launcher(&[
                "play",
                "--version",
                BUILD,
                "--world",
                "Lantern",
                "--creative",
            ])
            .spawn()
            .unwrap(),
    );
    let args = fixture.game_arguments();
    assert_resource_args(&fixture, &args);
    assert!(args.windows(2).any(|part| part == ["--world", "Lantern"]));
    assert!(args.contains(&"--creative".into()));
    assert_eq!(
        fs::read(fixture.home.join("wrapper-ran")).unwrap(),
        b"wrapped"
    );
    let committed = fixture.pointer();
    let refused = rpc(2, "tools/call", json!({"name":"play","arguments":{}}));
    assert_eq!(refused["isError"], true, "{refused}");
    assert!(
        refused["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("profile is in use"),
        "{refused}"
    );
    assert!(!fixture
        .launcher(&["remove", BUILD])
        .status()
        .unwrap()
        .success());
    assert!(!fixture.home.join("post-exit").exists());
    fs::write(fixture.home.join("finish"), []).unwrap();
    assert_eq!(cli.0.wait().unwrap().code(), Some(7));
    await_file(&fixture.home.join("post-exit"));

    let wrong = fixture
        .launcher(&["play", "--version", "developer-other"])
        .output()
        .unwrap();
    assert!(!wrong.status.success());
    assert!(String::from_utf8_lossy(&wrong.stderr).contains("not developer-other"));
    assert_eq!(fixture.pointer(), committed);

    // Supply the listening socket only once the actual fixture process has put its
    // selected profile and sealed resources on disk. No game protocol is simulated.
    fs::remove_file(fixture.home.join("finish")).unwrap();
    fs::remove_file(fixture.home.join("args")).unwrap();
    let endpoint = fixture.data.join("game-control.sock");
    let socket = std::thread::scope(|scope| {
        let ready = scope.spawn(|| {
            let args = fixture.game_arguments();
            assert_resource_args(&fixture, &args);
            assert!(args
                .windows(2)
                .any(|part| part[0] == "--control-socket" && Path::new(&part[1]) == endpoint));
            UnixListener::bind(&endpoint).unwrap()
        });
        let reply = rpc(
            3,
            "tools/call",
            json!({"name":"play","arguments":{"world":"MCP world"}}),
        );
        assert_ne!(reply["isError"], true, "{reply}");
        assert!(
            reply["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Started the developer-local client"),
            "{reply}"
        );
        ready.join().unwrap()
    });
    assert_eq!(fixture.pointer(), committed);
    fs::write(fixture.home.join("finish"), []).unwrap();
    drop(socket);
    drop(to_mcp);
    assert!(mcp.0.wait().unwrap().success());
}

#[test]
fn attached_prelaunch_hook_keeps_both_streams_and_its_failure_code() {
    let fixture = Fixture::new();
    let hook = "printf 'hook stdout\\n'; printf 'hook stderr\\n' >&2";
    fixture.configure(json!({"pre_launch":hook}));
    fs::write(fixture.home.join("finish"), []).unwrap();
    let started = fixture.launcher(&["play"]).output().unwrap();
    assert_eq!(
        started.status.code(),
        Some(7),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    assert_eq!(started.stdout, b"hook stdout\n");
    assert!(String::from_utf8_lossy(&started.stderr).contains("hook stderr\n"));
    assert_resource_args(&fixture, &fixture.game_arguments());

    fs::remove_file(fixture.home.join("args")).unwrap();
    fixture.configure(
        json!({"pre_launch":format!("{hook}; exit 37"), "post_exit":"touch bad-post-exit"}),
    );
    let refused = fixture.launcher(&["play"]).output().unwrap();
    assert_eq!(refused.status.code(), Some(37));
    assert_eq!(refused.stdout, b"hook stdout\n");
    let error = String::from_utf8_lossy(&refused.stderr);
    assert!(
        error.contains("hook stderr\n") && error.contains("pre-launch command failed (exit 37)"),
        "{error}"
    );
    assert!(!fixture.home.join("args").exists());
    assert!(!fixture.home.join("bad-post-exit").exists());
}

#[test]
fn mcp_returns_a_bounded_tail_when_the_prelaunch_hook_fails() {
    let fixture = Fixture::new();
    fixture.configure(json!({"pre_launch": r#"i=1; while [ "$i" -le 45 ]; do printf 'hook-line-%02d\n' "$i"; i=$((i+1)); done; exit 37"#}));
    let mut mcp = Running(
        fixture
            .launcher(&["mcp"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut input = mcp.0.stdin.take().unwrap();
    let mut output = BufReader::new(mcp.0.stdout.take().unwrap());
    mcp_request(&mut input, &mut output, 1, "initialize", json!({}));
    let reply = mcp_request(
        &mut input,
        &mut output,
        2,
        "tools/call",
        json!({"name":"play","arguments":{}}),
    );
    assert_eq!(reply["isError"], true, "{reply}");
    let detail = reply["content"][0]["text"].as_str().unwrap();
    assert!(
        detail.contains("pre-launch command failed (exit 37)"),
        "{detail}"
    );
    assert_eq!(
        detail
            .lines()
            .filter(|line| line.starts_with("hook-line-"))
            .count(),
        20,
        "{detail}"
    );
    assert!(
        detail.contains("hook-line-26") && detail.ends_with("hook-line-45"),
        "{detail}"
    );
    assert!(!detail.contains("hook-line-25"), "{detail}");
    assert!(!fixture.home.join("args").exists());
    drop(input);
    assert!(mcp.0.wait().unwrap().success());
}

#[test]
fn offline_play_keeps_protected_environment_filtered_after_jar_preparation() {
    let fixture = Fixture::new();
    fixture.configure(json!({
        "env": {
            "SUNRISE_SESSION_FILE":"not-the-selected-session",
            "SUNRISE_LAUNCHER_EXE":"not-the-launcher",
            "SUNRISE_LAUNCHER_DEVELOPER_OFFLINE":"0",
            "FIXTURE_INSTANCE_ENV":"kept",
        },
        "pre_launch": "printf '%s\\n' \"${SUNRISE_SESSION_FILE-unset}\" \"${SUNRISE_LAUNCHER_EXE-unset}\" \"${SUNRISE_LAUNCHER_DEVELOPER_OFFLINE-unset}\" \"${FIXTURE_INSTANCE_ENV-unset}\" > hook-env",
    }));
    fs::write(fixture.home.join("finish"), []).unwrap();
    let played = fixture
        .launcher(&["play"])
        .env("SUNRISE_SESSION_FILE", "inherited-stale-session")
        .output()
        .unwrap();
    assert_eq!(
        played.status.code(),
        Some(7),
        "{}",
        String::from_utf8_lossy(&played.stderr)
    );
    for path in ["hook-env", "game-env"] {
        assert_eq!(
            fs::read_to_string(fixture.home.join(path)).unwrap(),
            "unset\nunset\n1\nkept\n",
            "{path}"
        );
    }
    assert_resource_args(&fixture, &fixture.game_arguments());
}

#[test]
fn cli_persists_its_first_build_before_the_registry_replies() {
    let fixture = Fixture::new();
    let pipe = fixture.root.join("registry");
    assert!(Command::new("mkfifo")
        .arg(&pipe)
        .status()
        .unwrap()
        .success());
    let file = format!("{}/lantern.smod", mod_list::DIR);
    let package = fixture.home.join(&file);
    fs::create_dir_all(package.parent().unwrap()).unwrap();
    let body = format!(
        "id='lantern'\nname='Lantern'\nversion='1.0.0'\napi='{}'\ncapabilities=['registries']\n",
        manifest::API
    );
    fs::write(
        &package,
        manifest::Package::write_files(&body, &[]).unwrap(),
    )
    .unwrap();
    let sha = sunrise_launcher_smod::index::sha256_file(&package).unwrap();
    let row = fixture
        .home
        .join(sunrise_launcher_smod::index::browser_state::MANIFEST);
    fs::create_dir_all(row.parent().unwrap()).unwrap();
    fs::write(
        row,
        json!({"entries":[{
            "source":"sunrise", "project":"lantern", "slug":"lantern", "title":"Lantern",
            "kind":"mod", "version":"1.0.0", "number":"1.0.0", "file":file,
            "checksum":format!("SHA-256:{sha}"), "page":"", "dependency":false,
        }]})
        .to_string(),
    )
    .unwrap();
    fs::write(fixture.home.join(mod_list::FILE), b"mod.lantern:true\n").unwrap();

    let mut interrupted = Running(
        fixture
            .launcher(&["play", "--version", BUILD])
            .env("SUNRISE_MOD_INDEX", &pipe)
            .spawn()
            .unwrap(),
    );
    // Successfully opening the write end proves the real index read is waiting;
    // keep it open without sending a reply.
    let limit = Instant::now() + Duration::from_secs(25);
    let writer = loop {
        match fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&pipe)
        {
            Ok(file) => break file,
            Err(error) if error.raw_os_error() == Some(libc::ENXIO) => {
                assert!(
                    Instant::now() < limit,
                    "the launcher did not reach the registry read"
                );
                assert!(interrupted.0.try_wait().unwrap().is_none());
                std::thread::sleep(Duration::from_millis(15));
            }
            Err(error) => panic!("registry fixture: {error}"),
        }
    };
    let pending = sunrise_launcher::profile_txn::references(&fixture.home).unwrap();
    assert!(pending.targets.contains(BUILD));
    assert!(pending.pointer.is_none());
    assert!(!fixture.home.join("args").exists());
    interrupted.0.kill().unwrap();
    assert!(!interrupted.0.wait().unwrap().success());
    drop(writer);
    fs::remove_file(&pipe).unwrap();
    fs::write(&pipe, r#"{"schema":1,"projects":[]}"#).unwrap();
    fs::write(
        fixture.data.join("instances.json"),
        r#"{"instances":{"sunrise":{"version":"developer-other"}}}"#,
    )
    .unwrap();
    fs::write(fixture.home.join("finish"), []).unwrap();
    let retried = fixture
        .launcher(&["play"])
        .env("SUNRISE_MOD_INDEX", &pipe)
        .output()
        .unwrap();
    assert_eq!(
        retried.status.code(),
        Some(7),
        "{}",
        String::from_utf8_lossy(&retried.stderr)
    );
    assert_resource_args(&fixture, &fixture.game_arguments());
    let committed: Value = serde_json::from_slice(&fixture.pointer()).unwrap();
    assert_eq!(committed["base"]["tag"], BUILD);
}

#[test]
fn cli_does_not_spawn_after_jar_refusal_or_when_prelaunch_changes_the_committed_inputs() {
    let fixture = Fixture::new();
    let mods = fixture.home.join(mod_list::DIR);
    fs::create_dir(&mods).unwrap();
    let package = mods.join("lantern.smod");
    let manifest = format!("id='lantern'\nname='Lantern'\nversion='1.0.0'\napi='{}'\ncapabilities=['registries']\n[[jar]]\nid='fixture'\nversion='1.0.0'\nsha512='{}'\n", manifest::API, "a".repeat(128));
    fs::write(
        &package,
        manifest::Package::write_files(&manifest, &[]).unwrap(),
    )
    .unwrap();
    fs::write(fixture.home.join(mod_list::FILE), b"mod.lantern:true\n").unwrap();
    let refused = fixture.launcher(&["play"]).output().unwrap();
    assert!(!refused.status.success());
    let error = String::from_utf8_lossy(&refused.stderr);
    assert!(error.contains("Jar downloads were not allowed"), "{error}");
    assert!(!fixture.home.join("args").exists());
    assert!(!fixture.home.join("sessions").exists());

    fs::remove_file(package).unwrap();
    fixture.configure(json!({"pre_launch":"printf 'mod.lantern:false\\n' > sunrise-mods.txt"}));
    let refused = fixture.launcher(&["play"]).output().unwrap();
    assert!(!refused.status.success());
    let error = String::from_utf8_lossy(&refused.stderr);
    assert!(
        error.contains("couldn't verify the profile's saved game build"),
        "{error}"
    );
    assert!(!fixture.home.join("args").exists());
    assert!(fixture.home.join("sunrise-mods.txt").is_file());
}
