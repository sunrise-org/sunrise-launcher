use super::*;
use sunrise_launcher::account::capes::Cape;

const SKIN: &str = "http://textures.minecraft.net/texture/worn";
const ALEX: &str = "http://textures.minecraft.net/texture/alex";
const KEPT: &str = "0123456789abcdef";

fn cape(id: &str, active: bool) -> Cape {
    Cape {
        id: id.into(),
        alias: id.into(),
        url: format!("http://textures.minecraft.net/texture/{id}"),
        active,
    }
}

fn profile(skin: &str, slim: bool, worn_cape: Option<&str>) -> Profile {
    Profile {
        id: "069a79f444e94726a5befca90e38aaf5".into(),
        skin: Some((skin.into(), if slim { "slim" } else { "classic" }.into())),
        capes: ["c1", "c2"]
            .into_iter()
            .map(|id| cape(id, worn_cape == Some(id)))
            .collect(),
    }
}

fn worn(skins: &Skins) -> Worn {
    skins.worn(None, false)
}

/// Chooses on the page the way a tile does: the edit, then forgetting what is worn already.
fn choose(skins: &mut Skins, change: impl FnOnce(&mut Edit)) {
    let worn = worn(skins);
    skins.choose(change, &worn);
}

fn pick(skins: &mut Skins, skin: SkinPick) {
    choose(skins, |edit| edit.skin = Some(skin));
}

/// A Save answered and settled, as `Launcher::look_saved` does it.
fn answer_save(skins: &mut Skins, id: u64, outcome: &look::Outcome) -> bool {
    let answered = skins.save_answered(id, outcome);
    let worn = worn(skins);
    skins.settle(&worn);
    answered
}

fn outcome(skin: Option<Change>, cape: Option<Change>, profile: Option<Profile>) -> look::Outcome {
    look::Outcome {
        kept: Kept::Nothing,
        skin,
        cape,
        profile,
        arms: None,
        recorded: None,
        uploaded: None,
    }
}

fn kept(url: &str) -> Kept {
    Kept::Kept {
        url: url.into(),
        saved: Saved {
            id: KEPT.into(),
            name: String::new(),
            slim: false,
        },
    }
}

fn read(skins: &mut Skins, profile: Profile) {
    let id = skins.read_asked();
    assert!(skins.read_answered("a", id, Ok(profile), false));
}

/// The launcher itself, in a child of the test binary with scratch homes, no keychain and an
/// unreachable proxy, so it reads and writes no player's data and fetches nothing.
fn in_a_scratch_launcher(name: &str, test: impl FnOnce(&mut Launcher, &mut Context<Launcher>)) {
    const CHILD: &str = "SUNRISE_SKINS_LAUNCHER_TEST";
    if std::env::var_os(CHILD).is_none() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        let nowhere = "http://127.0.0.1:9";
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture"])
            .env(CHILD, root)
            .env("SUNRISE_LAUNCHER_HOME", root.join("launcher"))
            .env("SUNRISE_HOME", root.join("game"))
            .env("SUNRISE_LAUNCHER_LOCATION_FILE", root.join("location.json"))
            .env("SUNRISE_NO_KEYCHAIN", "1")
            .env("SUNRISE_TELEMETRY_URL", "off")
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("XDG_DATA_HOME", root.join("data"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("HTTPS_PROXY", nowhere)
            .env("HTTP_PROXY", nowhere)
            .env("ALL_PROXY", nowhere)
            .env_remove("NO_PROXY")
            .env_remove("SUNRISE_GITHUB_MIRROR")
            .env_remove("SUNRISE_LAUNCHER_BENCH")
            .env_remove("SUNRISE_LAUNCHER_DEVELOPER_OFFLINE")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let mut app = gpui::TestAppContext::single();
    let (shell, cx) = app.add_window_view(crate::window_root::Shell::new_for_test);
    let launcher = shell.read_with(cx, |shell, _| shell.launcher());
    launcher.update(cx, |l, cx| {
        l.accounts.accounts.push(account::Account {
            uuid: UUID.into(),
            name: "Player".into(),
            skin: None,
            cape: None,
        });
        test(l, cx)
    });
}

const UUID: &str = "069a79f444e94726a5befca90e38aaf5";

/// The launcher's next message that `wanted` picks, the others (release lists, heads) let go.
fn next(l: &mut Launcher, wanted: impl Fn(&Msg) -> bool) -> Msg {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let msg = l.rx.recv_timeout(left).expect("the message came");
        if wanted(&msg) {
            return msg;
        }
    }
}

/// A 64 by 64 skin, one texel of it `n`.
fn skin_png(n: u8) -> Vec<u8> {
    let image = image::RgbaImage::from_fn(64, 64, |x, y| {
        image::Rgba(if (x, y) == (0, 0) {
            [n, 1, 1, 255]
        } else {
            [x as u8, y as u8, 7, 255]
        })
    });
    let mut out = std::io::Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

const NEW: &str = "http://textures.minecraft.net/texture/new";

/// A Save the services took, whose own profile was the one from before it.
fn unconfirmed_save(skins: &mut Skins) {
    pick(skins, SkinPick::Default("alex"));
    choose(skins, |edit| {
        edit.cape = Some(CapePick::Minecraft("c1".into()))
    });
    let submitted = skins.submit(false).unwrap();
    let mut stale = outcome(
        Some(Change::Unconfirmed("old".into())),
        Some(Change::Unconfirmed("old".into())),
        Some(profile(SKIN, false, Some("c2"))),
    );
    stale.uploaded = Some((skin_library::id_of_png(&skin_png(5)).unwrap(), true));
    answer_save(skins, submitted.id, &stale);
    assert_eq!(
        skins.confirmed,
        look::Record::default(),
        "nothing shown yet"
    );
}

mod jobs;
mod library;
mod pages;
mod recording;
mod saves;
mod worn_look;
