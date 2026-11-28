//! The Servers page: create a server in one dialog, then start, stop and watch it, give friends its
//! address, manage who may join, change its settings, back it up and update it, all with buttons. Every
//! action runs `sunrise-server` or calls the running server's control API (`sunrise_launcher::servers`).
use super::line_edit::{self, Key, LineEdit};
use super::*;
use crate::frame::Reach as _;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use sunrise_launcher::backend::lock::recover;
use sunrise_launcher::heads;
use sunrise_launcher::servers::{
    self, Applied, Backup, Entry, Fix, NewServer, Problem, Status, World,
};
use sunrise_launcher_theme::glass::radius;

mod actions;
mod dialog;

mod form;
mod format;
mod hosting;
mod jobs;
mod state;
use format::{backup_label, bytes, uptime};

pub(super) use state::Servers;

/// One consent boundary for every launcher-managed start/restart route.
fn run_managed_server(
    tx: &std::sync::mpsc::Sender<backend::Msg>,
    dir: &Path,
    args: &[&str],
    log: &dyn Fn(String),
) -> Result<(), Problem> {
    crate::jar_launch::with_consent(tx, |permission| {
        servers::run_managed(dir, args, log, permission)
    })
}

/// The servers this launcher started in this session, stopped when it closes unless they run as a
/// service (`stop_on_quit`).
static STARTED_HERE: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// How often the open server's status and log are read, and the list's.
const POLL_OPEN: Duration = Duration::from_millis(1000);
const POLL_LIST: Duration = Duration::from_secs(3);

/// Lines of the server's log kept on the page.
const CONSOLE_KEEP: usize = 1000;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) enum Field {
    Name,
    Seed,
    Players,
    Command,
    AddPlayer,
    Motd,
    Port,
    Token,
    Email,
    /// The instance dialog's (`instances_page.rs`).
    InstanceName,
    InstanceGroup,
    /// The new-profile dialog's "New group" popup.
    NewGroup,
    /// The new-profile dialog's search over the profiles of other launchers.
    ImportSearch,
    /// The Launch panel's (`launch_page.rs`).
    PreLaunch,
    Wrapper,
    LaunchEnv,
    PostExit,
    DeleteServer,
    /// Settings' accent as hex (`themes_page.rs`).
    AccentHex,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Tab {
    Overview,
    Players,
    Settings,
    Backups,
}

const TABS: [(Tab, &str); 4] = [
    (Tab::Overview, "Overview"),
    (Tab::Players, "Players"),
    (Tab::Settings, "Settings"),
    (Tab::Backups, "Backups"),
];

const MODES: [(&str, &str); 3] = [
    ("survival", "Survival"),
    ("creative", "Creative"),
    ("adventure", "Adventure"),
];

const DIFFICULTIES: [(&str, &str); 4] = [
    ("peaceful", "Peaceful"),
    ("easy", "Easy"),
    ("normal", "Normal"),
    ("hard", "Hard"),
];

/// What the background work reports.
pub(super) enum Event {
    PublicAddress {
        request: u64,
        result: Result<std::net::Ipv4Addr, String>,
    },
    Polled {
        dir: PathBuf,
        status: Box<Status>,
        logs: Option<(u64, Vec<String>)>,
    },
    Statuses(Vec<(PathBuf, Status)>),
    Line(String),
    Done(Result<String, Problem>),
    Created(Result<Entry, Problem>),
    Deleted {
        dir: PathBuf,
        result: Result<(), String>,
    },
    Details {
        dir: PathBuf,
        request: u64,
        config: Option<Value>,
        backups: Vec<Backup>,
        service: bool,
    },
    Applied {
        dir: PathBuf,
        result: Result<Applied, Problem>,
    },
    Pvp(Option<bool>),
    UpdateCheck(Result<Value, Problem>),
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PublicLookup {
    Idle,
    Loading(u64),
    Detected,
    Failed,
}

/// The Create dialog's answers.
pub(super) struct Create {
    world: WorldChoice,
    /// The player's worlds in every profile, the last played first.
    worlds: Vec<CreateWorld>,
    /// The dropdown that is open.
    menu: Option<CreateMenu>,
    mode: &'static str,
    difficulty: &'static str,
    max_players: u32,
    friends_only: bool,
    check_accounts: bool,
    eula: bool,
    /// Create was pressed without the EULA ticked.
    eula_missing: bool,
    /// The glass dialog's friends let in, the first an operator (the light one types them in
    /// `Field::Players`).
    friends: Vec<String>,
}

/// What the new server's world is made from.
#[derive(Clone, PartialEq)]
enum WorldChoice {
    New,
    /// A copy of one of the player's worlds (its folder).
    Copy(PathBuf),
    /// A world file or folder the player picked.
    Import(PathBuf),
}

/// One of the player's worlds, as the dialog's World dropdown lists it.
struct CreateWorld {
    /// Its name in the game's list.
    name: String,
    /// The profile it is in.
    profile: String,
    path: PathBuf,
    /// Its mode and when it was last played.
    hint: String,
}

/// The dropdowns of the Create dialog.
#[derive(Clone, Copy, PartialEq)]
enum CreateMenu {
    World,
    Mode,
    Difficulty,
    Who,
}

pub(super) fn stop_on_quit() {
    for dir in recover(&STARTED_HERE).drain(..) {
        if servers::service_installed(&dir) {
            continue;
        }
        if let Some(api) = servers::Api::open(&dir) {
            let _ = api.call("POST", "/api/v1/stop", None);
        }
    }
}

/// Update closes the window to open the new launcher at once: a server is its own process and the
/// new window finds it running, so an update doesn't end anyone's game. It stays up when that new
/// window closes too, until it is stopped on the Servers page.
pub(super) fn keep_running_through_update() {
    recover(&STARTED_HERE).clear();
}

/// The player's own name, to let them in on a friends-only server: the Microsoft account that plays.
fn own_name(accounts: &Accounts) -> Option<String> {
    accounts.selected().map(|a| a.name.clone())
}

/// The state, in a few words.
fn phase_label(status: &Status) -> String {
    if let Some(busy) = &status.update_busy {
        return trf("Updating: {step}", &[("step", busy)]);
    }
    match status.phase.as_str() {
        "running" if status.crashes > 0 => tr("Running · restarted after a crash").into(),
        "running" if status.paused => tr("Running · paused while nobody plays").into(),
        "running" => tr("Running").into(),
        "starting" => tr("Starting").into(),
        "restarting" => tr("Restarting").into(),
        "stopping" => tr("Stopping").into(),
        "unknown" => tr("Server status unavailable").into(),
        _ => tr("Stopped").into(),
    }
}

/// A setting's name for people.
fn setting_name(key: &str) -> &str {
    match key {
        "server.motd" => "the message in the server list",
        "server.port" => "the port",
        "server.max_players" => "most players",
        "server.view_distance" => "view distance",
        "server.simulation_distance" => "simulation distance",
        "server.online_mode" => "account checks",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sharing_uses_the_live_bind_without_saved_settings_and_clears_an_unknown_status() {
        let mut servers = Servers::new();
        let dir = PathBuf::from("server");
        servers.open = Some(dir.clone());
        servers.config = Some(serde_json::json!({"server": {"port": 25567}}));
        let old = "127.0.0.1:25566".parse().unwrap();
        assert_eq!(
            servers.share_address(&servers.status()),
            (None, Some(25567))
        );
        servers.note_status(
            dir.clone(),
            Status {
                phase: "running".into(),
                listener: Some(old),
                ..Status::default()
            },
        );
        assert_eq!(
            servers.share_address(&servers.status()),
            (Some(old), Some(25567))
        );
        servers.config = None;
        assert_eq!(servers.share_address(&servers.status()), (Some(old), None));
        servers.config = Some(serde_json::json!({"server": {"port": 25567}}));
        servers.note_status(dir.clone(), Status::unknown());
        assert_eq!(
            servers.share_address(&servers.status()),
            (None, Some(25567))
        );
        let restarted = "0.0.0.0:25567".parse().unwrap();
        servers.note_status(
            dir.clone(),
            Status {
                phase: "running".into(),
                listener: Some(restarted),
                ..Status::default()
            },
        );
        assert_eq!(
            servers.share_address(&servers.status()),
            (Some(restarted), Some(25567))
        );
        servers.note_status(dir.clone(), Status::stopped());
        assert_eq!(
            servers.share_address(&servers.status()),
            (Some(restarted), Some(25567))
        );
        servers.queue_setting(dir, "server.port", "25568".into());
        assert_eq!(
            servers.share_address(&servers.status()),
            (None, Some(25567))
        );
    }

    #[test]
    fn a_dismissed_lookup_remains_in_flight_and_cannot_enable_copy() {
        let mut servers = Servers::new();
        servers.internet_address = true;
        let first = servers.begin_public_lookup().unwrap();
        servers.reset_public_lookup();
        assert!(servers.begin_public_lookup().is_none());
        let ip = "192.0.2.42".parse().unwrap();
        assert!(servers.finish_public_lookup(first, Ok(ip)));
        assert_eq!(servers.public_lookup, PublicLookup::Idle);
        assert!(servers.last_public_address.is_none());
        let second = servers.begin_public_lookup().unwrap();
        assert!(servers.finish_public_lookup(second, Ok(ip)));
        assert_eq!(servers.public_lookup, PublicLookup::Detected);
        assert_eq!(servers.last_public_address, Some(ip));
    }

    #[test]
    fn a_running_server_is_not_a_launcher_write_but_its_hosting_workers_are() {
        let mut servers = Servers::new();
        servers.statuses.insert(
            PathBuf::from("server"),
            Status {
                phase: "running".into(),
                ..Status::default()
            },
        );
        assert!(servers.location_busy());
        assert!(!servers.writing());
        servers.creating = true;
        assert!(servers.writing());
        servers.creating = false;
        servers.busy = Some("Restoring a backup".into());
        assert!(servers.writing());
        servers.busy = None;
        let server = PathBuf::from("server");
        servers.queue_setting(server.clone(), "server.port", "25567".into());
        assert!(servers.next_settings_batch().is_some());
        assert!(servers.writing());
        servers.finish_settings(&server, Err(Problem::plain("save failed")));
        assert!(!servers.writing());
    }

    #[test]
    fn an_update_waits_for_a_hosting_restore_until_the_worker_reports_completion() {
        use crate::updates::Wait;
        let mut servers = Servers::new();
        servers.statuses.insert(
            PathBuf::from("server"),
            Status {
                phase: "running".into(),
                ..Status::default()
            },
        );
        let (finish, wait_for_finish) = std::sync::mpsc::channel();
        servers.start_job("Restore a backup", move |_| {
            wait_for_finish.recv().unwrap();
            Ok(String::new())
        });
        let ready = SelfUpdate::Ready(Release {
            tag: "v1.3.0".into(),
            name: String::new(),
            body: String::new(),
            date: String::new(),
            time: String::new(),
            published_at: String::new(),
            asset: None,
            prerelease: false,
            nightly: None,
        });
        let restart = |hosting: &Servers| {
            ready
                .ready_to_restart(true, true, || {
                    Wait::for_activity(false, false, hosting, false)
                })
                .is_some()
        };
        assert!(!restart(&servers));
        finish.send(()).unwrap();
        let Event::Done(result) = servers.rx.recv_timeout(Duration::from_secs(10)).unwrap() else {
            panic!("expected the Hosting worker's completion event");
        };
        result.as_ref().unwrap();
        servers.finish_job(result);
        assert!(restart(&servers), "a running server must not block restart");
    }

    #[test]
    fn queued_settings_drain_to_their_own_servers_without_an_open_page() {
        let (one, two) = (PathBuf::from("one"), PathBuf::from("two"));
        let mut servers = Servers::new();
        servers.open = Some(one.clone());
        servers.queue_setting(one.clone(), "server.motd", "saving".into());
        let first = servers.next_settings_batch().unwrap();
        assert_eq!(first.dir, one);
        servers.queue_setting(one.clone(), "server.motd", "old edit".into());
        servers.queue_setting(one.clone(), "server.motd", "latest edit".into());
        servers.open = Some(two.clone());
        assert_eq!(servers.shown("server", "motd"), "");
        servers.queue_setting(two.clone(), "server.motd", "other server".into());
        assert_eq!(servers.shown("server", "motd"), "other server");
        servers.open = None;
        assert!(servers.next_settings_batch().is_none());
        let done = || {
            Ok(Applied {
                applied: vec![],
                restart: vec!["server.motd".into()],
                running: true,
            })
        };
        assert!(!servers.finish_settings(&first.dir, done()));
        assert!(servers.writing());
        for (dir, motd) in [(one, "latest edit"), (two, "other server")] {
            let batch = servers.next_settings_batch().unwrap();
            assert_eq!(batch.dir, dir);
            assert_eq!(
                batch.changes,
                [(String::from("server.motd"), String::from(motd))]
            );
            assert!(servers.writing());
            assert!(!servers.finish_settings(&batch.dir, done()));
        }
        assert!(servers.next_settings_batch().is_none());
        assert!(!servers.writing());
    }

    #[test]
    fn running_settings_remain_visible_on_reopen_without_mutating_the_snapshot() {
        let one = PathBuf::from("one");
        let two = PathBuf::from("two");
        let mut servers = Servers::new();
        let snapshot = serde_json::json!({"world": "not an object"});
        servers.config = Some(snapshot.clone());
        servers.open = Some(one.clone());
        servers.queue_setting(one.clone(), "world.view_distance", "9".into());
        assert!(servers.next_settings_batch().is_some());
        assert_eq!(servers.config.as_ref(), Some(&snapshot));
        servers.open = Some(two);
        assert_eq!(servers.shown("world", "view_distance"), "");
        servers.open = None;
        servers.open = Some(one.clone());
        servers.config = Some(serde_json::json!({"world": {"view_distance": 8}}));
        assert_eq!(servers.shown("world", "view_distance"), "9");
        servers.queue_setting(one, "world.view_distance", "10".into());
        assert_eq!(servers.shown("world", "view_distance"), "10");
    }

    #[test]
    fn settings_completion_keeps_notices_and_mode_invalidation_on_the_written_server() {
        let one = PathBuf::from("one");
        let two = PathBuf::from("two");
        let mut servers = Servers::new();
        let apply = |servers: &mut Servers, result| {
            servers.open = Some(one.clone());
            servers.queue_setting(one.clone(), "server.port", "25577".into());
            let batch = servers.next_settings_batch().unwrap();
            servers.open = Some(two.clone());
            assert!(!servers.finish_settings(&batch.dir, result));
            assert!(!servers.settings_outcomes.contains_key(&two));
            assert!(servers.problem.is_none());
        };
        servers.modes.insert(one.clone(), Some("survival".into()));
        servers.modes.insert(two.clone(), Some("creative".into()));
        apply(
            &mut servers,
            Ok(Applied {
                applied: vec![],
                restart: vec!["server.port".into()],
                running: true,
            }),
        );
        assert!(!servers.modes.contains_key(&one));
        assert!(servers.modes.contains_key(&two));
        apply(
            &mut servers,
            Err(Problem {
                message: "port is unavailable".into(),
                fix: Some(Fix::UsePort(25578)),
            }),
        );
        servers.open = Some(one.clone());
        let outcome = &servers.settings_outcomes[&one];
        assert!(outcome.restart.contains("server.port"));
        let problem = outcome.problem.as_ref().unwrap();
        assert_eq!(problem.message, "port is unavailable");
        assert!(matches!(problem.fix.as_ref(), Some(Fix::UsePort(25578))));
        servers.dismiss_problem(Some(&one));
        assert!(servers.settings_outcomes[&one].problem.is_none());
    }

    #[test]
    fn saved_value_stays_visible_while_another_batch_is_queued_without_mutating_snapshot() {
        let dir = PathBuf::from("one");
        let mut servers = Servers::new();
        let snapshot = serde_json::json!({"world": {"view_distance": 8, "difficulty": "normal"}});
        servers.open = Some(dir.clone());
        servers.config = Some(snapshot.clone());

        // Start saving view_distance 8 -> 9, then queue an unrelated difficulty edit.
        servers.queue_setting(dir.clone(), "world.view_distance", "9".into());
        let first = servers.next_settings_batch().unwrap();
        servers.queue_setting(dir.clone(), "world.difficulty", "hard".into());

        // First batch finishes while the second is still queued: snapshot stays untouched,
        // yet view_distance still shows 9 so a follow-up `+` submits 10 rather than 9.
        assert!(!servers.finish_settings(
            &first.dir,
            Ok(Applied {
                applied: vec!["world.view_distance".into()],
                restart: vec![],
                running: false,
            })
        ));
        assert_eq!(servers.config.as_ref(), Some(&snapshot));
        assert_eq!(servers.shown("world", "view_distance"), "9");
        assert_eq!(servers.shown("world", "difficulty"), "hard");

        // A newer queued edit to view_distance sits on top of the saved value.
        servers.queue_setting(dir.clone(), "world.view_distance", "10".into());
        assert_eq!(servers.shown("world", "view_distance"), "10");
    }

    #[test]
    fn unrelated_successful_save_keeps_earlier_failed_save_error_until_fixed_or_dismissed() {
        let one = PathBuf::from("one");
        let two = PathBuf::from("two");
        let mut servers = Servers::new();
        servers.open = Some(one.clone());

        // Port save fails while difficulty save is queued on the same server.
        servers.queue_setting(one.clone(), "server.port", "25577".into());
        let port_batch = servers.next_settings_batch().unwrap();
        servers.queue_setting(one.clone(), "world.difficulty", "hard".into());
        servers.open = Some(two);
        assert!(
            !servers.finish_settings(&port_batch.dir, Err(Problem::plain("port is unavailable")))
        );

        // Unrelated difficulty save succeeds: the port error stays visible.
        let diff_batch = servers.next_settings_batch().unwrap();
        assert!(!servers.finish_settings(
            &diff_batch.dir,
            Ok(Applied {
                applied: vec!["world.difficulty".into()],
                restart: vec![],
                running: false,
            })
        ));
        servers.open = Some(one.clone());
        assert_eq!(
            servers.settings_outcomes[&one]
                .problem
                .as_ref()
                .map(|p| p.message.as_str()),
            Some("port is unavailable")
        );

        // Saving server.port successfully clears the port error.
        servers.queue_setting(one.clone(), "server.port", "25578".into());
        let retry_port = servers.next_settings_batch().unwrap();
        assert!(servers.finish_settings(
            &retry_port.dir,
            Ok(Applied {
                applied: vec!["server.port".into()],
                restart: vec![],
                running: false,
            })
        ));
        assert!(servers.settings_outcomes[&one].problem.is_none());
    }

    #[test]
    fn late_running_poll_after_stale_stopped_status_keeps_restart_notice_until_real_restart() {
        let dir = PathBuf::from("one");
        let mut servers = Servers::new();
        servers.open = Some(dir.clone());
        let status_with = |phase: &str| Status {
            phase: phase.into(),
            ..Status::unknown()
        };

        // Cached status is still stale "stopped" when a port save reports running=true.
        servers.note_status(dir.clone(), status_with("stopped"));
        servers.queue_setting(dir.clone(), "server.port", "25577".into());
        let batch = servers.next_settings_batch().unwrap();
        assert!(servers.finish_settings(
            &batch.dir,
            Ok(Applied {
                applied: vec![],
                restart: vec!["server.port".into()],
                running: true,
            })
        ));
        assert!(servers.settings_outcomes[&dir]
            .restart
            .contains("server.port"));

        // The next poll catches up to "running" for the same game: restart notice must stay.
        assert!(servers.note_status(dir.clone(), status_with("running")));
        assert!(
            servers.settings_outcomes[&dir]
                .restart
                .contains("server.port"),
            "catch-up running poll must not clear restart notice"
        );

        // Once the server actually stops and starts again, the notice clears.
        assert!(servers.note_status(dir.clone(), status_with("stopped")));
        assert!(servers.settings_outcomes[&dir]
            .restart
            .contains("server.port"));
        assert!(servers.note_status(dir.clone(), status_with("running")));
        assert!(servers.settings_outcomes[&dir].restart.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn managed_start_and_restart_require_consent_before_running_the_supervisor() {
        const CHILD: &str = "SUNRISE_MANAGED_CONSENT_TEST";
        if std::env::var_os(CHILD).is_none() {
            let root = tempfile::tempdir().unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "servers_page::tests::managed_start_and_restart_require_consent_before_running_the_supervisor", "--nocapture"])
                .env(CHILD, "1")
                .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
                .env("SUNRISE_HOME", root.path().join("cache"))
                .status().unwrap();
            assert!(status.success());
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let bin = servers::runtime_root().join("current");
        std::fs::create_dir_all(&bin).unwrap();
        let supervisor = bin.join("sunrise-server");
        let engine = bin.join("sunrise-engine");
        // Answers `engine-command --json` as the real supervisor does; anything else would start the server.
        let reply = serde_json::json!({
            "format": 1,
            "program": engine,
            "args": ["--resources", root.path(), "--control-socket", root.path().join(".sunrise/engine.json"), "--dedicated", "--nogui"],
            "cwd": root.path(),
            "env": {"SUNRISE_NO_AUDIO": "1"},
            "resources": root.path(),
        });
        std::fs::write(
            &supervisor,
            format!("#!/bin/sh\nif [ \"$3\" = engine-command ]; then printf '%s' '{reply}'; exit 0; fi\nprintf ran > \"$2/ran\"\n"),
        )
        .unwrap();
        std::fs::set_permissions(&supervisor, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(&engine, "never executed").unwrap();
        std::fs::write(root.path().join("sunrise-resources.json"), "{}").unwrap();
        let config = format!(
            "[engine]\nresources = {:?}\n",
            root.path().to_str().unwrap()
        );
        std::fs::write(root.path().join("server.toml"), config).unwrap();
        let package = root
            .path()
            .join(sunrise_launcher_smod::mod_list::DIR)
            .join("needs_jar");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(package.join("sunrise-mod.toml"), format!(
            "id='needs_jar'\nname='Needs jar'\nversion='1'\napi='0.2'\ncapabilities=['registries']\n[[jar]]\nid='fixture'\nversion='1'\nsha512='{}'\n", "a".repeat(128)
        )).unwrap();
        std::fs::write(
            root.path().join(sunrise_launcher_smod::mod_list::FILE),
            "mod.needs_jar:true\n",
        )
        .unwrap();
        for action in ["start", "restart"] {
            let (tx, rx) = std::sync::mpsc::channel();
            let dir = root.path().to_path_buf();
            let job = std::thread::spawn(move || run_managed_server(&tx, &dir, &[action], &|_| {}));
            let message = rx.recv_timeout(Duration::from_secs(10));
            let backend::Msg::JarConsent { jars, reply } = message.unwrap() else {
                panic!("expected jar consent")
            };
            assert_eq!(jars.len(), 1);
            assert_eq!(jars[0].id(), "fixture");
            reply.send(false).unwrap();
            assert!(job.join().unwrap().is_err());
            assert!(!root.path().join("ran").exists());
        }
    }

    #[test]
    fn backups_read_for_people() {
        let b = |name: &str| Backup {
            name: name.into(),
            bytes: 1,
            modified: None,
        };
        assert_eq!(
            backup_label(&b("2026-10-19_11-28-57-manual.zip")),
            ("2026-10-19 11:28:57 UTC".into(), "Made by hand".into())
        );
        assert_eq!(
            backup_label(&b("2026-10-19_11-28-57-before-v2026.10.20.zip")).1,
            "Before updating to v2026.10.20"
        );
        assert_eq!(
            backup_label(&b("2026-10-19_13-35-36-before-v2026-10-19-10.zip")).1,
            "Before updating to v2026.10.19.10"
        );
        assert_eq!(
            backup_label(&b("2026-10-14_17-46-40-before-update.zip")).1,
            "Before an update"
        );
        assert_eq!(
            backup_label(&b("2026-10-19_11-28-57-auto.zip")).1,
            "Automatic"
        );
        assert_eq!(backup_label(&b("odd.zip")), ("odd".into(), String::new()));
        assert_eq!(bytes(5_292_542), "5.3 MB");
        assert_eq!(uptime(3725), "1 h 02 min");
    }
}
