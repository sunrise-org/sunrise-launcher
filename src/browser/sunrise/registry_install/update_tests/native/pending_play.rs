use super::*;
use crate::{
    managed_play::{Client, Progress, StartError},
    native_mods::Cancellation,
};

const BROKEN: &str = "bad_lantern";

fn broken(fixture: &mut Fixture, declarations: &str) {
    fixture.project(BROKEN, "Broken Lantern", V2Kind::NativeMod);
    let tables = format!(
        "{declarations}\n[native]\nsunrise='{}'\nmixins='{}'\ngame='{}'\n",
        "a".repeat(40),
        "c".repeat(40),
        backend::PINNED_GAME,
    );
    fixture.release(
        BROKEN,
        "1.1.0",
        &tables,
        &[
            (
                "Cargo.toml",
                b"[package]\nname='bad_lantern'\nversion='1.1.0'\n",
            ),
            ("src/lib.rs", b"compile_error!(\"broken lantern fixture\");"),
        ],
    );
}

fn corrected_play(mut fixture: Fixture, stop: bool) {
    let executable = backend::version_dir(BUILD).join(backend::CLIENT);
    fs::write(
        &executable,
        r#"#!/bin/sh
if [ "$1" = --launch-api ]; then
  printf '%s\n' '{"options":[{"name":"--developer-single-player"}]}'
else
  touch "$SUNRISE_HOME/started"
fi
"#,
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    configure_tool(&fixture);
    fixture.project(ID, "Native Lantern", V2Kind::NativeMod);
    release(&mut fixture, "1.0.0", 'a', SOURCE);
    broken(&mut fixture, "");
    fixture
        .apply(Action::Install(projects(&[ID, BROKEN])))
        .unwrap();
    let pointer = fixture.home().join(crate::profile_txn::POINTER);
    let started = fixture.home().join("started");
    assert!(!pointer.exists());
    assert!(!fixture.root.join("compiled").exists());

    let play = |cancellation: &Cancellation, stop: bool| {
        Client {
            instance: &fixture.instance,
            accepted_account: None,
            explicit_account: false,
            piped: false,
            registry: || None,
        }
        .start(
            || FirstBuild::installed(BUILD),
            |_| Ok(()),
            |_| Ok(()),
            cancellation,
            |progress| {
                if stop && matches!(progress, Progress::NativeBuild { .. }) {
                    cancellation.stop();
                }
            },
        )
    };
    let Err(error) = play(&Cancellation::default(), stop) else {
        panic!("the first Play must not start the client");
    };
    if stop {
        assert!(
            matches!(error, StartError::Native(native_mods::Failed::Other(ref e), _)
            if e == native_mods::CANCELLED),
            "{error:?}"
        );
    } else {
        assert!(
            matches!(error, StartError::Native(native_mods::Failed::Mods { ref left, compiling: true, .. }, _)
            if left.len() == 1 && left[0].id == BROKEN),
            "{error:?}"
        );
    }
    assert!(!started.exists());
    assert!(!pointer.exists());
    mod_list::set_enabled(fixture.home(), BROKEN, false).unwrap();
    let mut game = play(&Cancellation::default(), false).unwrap();
    assert!(game.child.wait().unwrap().success());
    assert!(started.is_file());
    assert!(mod_list::is_enabled(fixture.home(), ID));
    assert!(!mod_list::is_enabled(fixture.home(), BROKEN));
    assert_eq!(
        Active::read_pointer(fixture.home())
            .unwrap()
            .unwrap()
            .executable(),
        game.executable
    );
    assert_eq!(
        compiled(&fixture),
        if stop { "1.0.0\n" } else { "1.1.0\n1.0.0\n" }
    );
}

#[test]
#[ignore = "daily; build the launcher binary first"]
fn turning_off_a_failed_mod_allows_the_next_play_to_start_with_the_rest() {
    Fixture::run(
        concat!(
            module_path!(),
            "::turning_off_a_failed_mod_allows_the_next_play_to_start_with_the_rest"
        ),
        |fixture| corrected_play(fixture, false),
    );
}

#[test]
#[ignore = "daily; build the launcher binary first"]
fn changing_the_mod_set_after_a_cancelled_first_play_allows_the_next_play() {
    Fixture::run(
        concat!(
            module_path!(),
            "::changing_the_mod_set_after_a_cancelled_first_play_allows_the_next_play"
        ),
        |fixture| corrected_play(fixture, true),
    );
}
