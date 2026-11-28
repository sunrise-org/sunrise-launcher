use super::*;

/// Signed in, a build installed, nothing under way: Play.
fn idle() -> Facts {
    Facts {
        signed_in: true,
        installed: true,
        located: true,
        ..Facts::default()
    }
}

/// Takes the winning state away, so the next one shows.
type Next = fn(&mut Facts);

const LONG: &str = "The build failed. The compiler said a great deal more than fits here.";

#[test]
fn play_says_each_state_in_the_order_they_win() {
    let all = Facts {
        installing: Some(0.42),
        preparing: true,
        building: true,
        starting: true,
        mutating: true,
        running: true,
        ..Facts::default()
    };
    // Each step takes the winning state away: the next one shows.
    let steps: [(Main, Next); 9] = [
        (Main::Installing(0.42), |f| f.installing = None),
        (Main::Preparing, |f| f.preparing = false),
        (Main::Building, |f| f.building = false),
        (Main::Starting, |f| f.starting = false),
        (Main::ChangingMods, |f| f.mutating = false),
        (Main::Stop, |f| f.running = false),
        (Main::SignIn, |f| f.signed_in = true),
        (Main::Install, |f| f.installed = true),
        (Main::Play, |_| {}),
    ];
    let mut facts = all;
    for (main, next) in steps {
        assert_eq!(facts.view().main, main);
        next(&mut facts);
    }
    assert_eq!(Main::Play.label(), "Play");
    // Signed out, Play is still "Play": Home has no second "Sign in with Microsoft".
    assert_eq!(Main::SignIn.label(), "Sign in to play");
    assert_eq!(Main::Installing(0.42).label(), "Installing · 42%");
}

#[test]
fn play_names_the_step_it_is_at_while_the_game_starts() {
    let starting = |step| Facts {
        starting: true,
        step,
        ..idle()
    };
    for (step, label) in [
        (None, "Starting…"),
        (Some(backend::PlayStep::Assets), "Checking game files…"),
        (Some(backend::PlayStep::Mods), "Updating mods…"),
        (Some(backend::PlayStep::SignIn), "Signing in…"),
    ] {
        let view = starting(step).view();
        assert_eq!(view.main.label(), label);
        assert!(!view.enabled, "{label}");
    }
    // A step left over once the start is over says nothing.
    let done = Facts {
        step: Some(backend::PlayStep::Mods),
        ..idle()
    };
    assert_eq!(done.view().main, Main::Play);
}

#[test]
fn pending_native_mods_change_the_idle_play_label_and_leave_the_gates_in_place() {
    let mut facts = Facts {
        pending_mods: true,
        ..idle()
    };
    let ready = facts.view();
    assert_eq!(ready.main.label(), "Build mods and play");
    assert!(ready.enabled);
    facts.building = true;
    assert_eq!(facts.view().main, Main::Building);
    facts.building = false;
    facts.signed_in = false;
    assert_eq!(facts.view().main, Main::SignIn);
}

#[test]
fn a_stopped_managed_build_opens_its_result_from_home_and_the_strip() {
    let view = Facts {
        build_result: Some("A mod couldn't be built.".into()),
        native_error: Some(LONG.into()),
        pending_mods: true,
        ..idle()
    }
    .view();
    assert_eq!(view.main, Main::BuildAndPlay);
    assert!(view.enabled);
    assert_eq!(view.act, Some(Act::BuildResult));
    assert_eq!(
        view.strip(),
        Some((
            "A mod couldn't be built.".into(),
            StripAct::Act(Act::BuildResult)
        ))
    );
}

#[test]
fn a_chosen_build_that_isnt_the_one_played_turns_play_into_its_install_action() {
    let download = Facts {
        switch_build: Some(true),
        ..idle()
    }
    .view();
    assert_eq!(download.main, Main::InstallBuild);
    assert_eq!(download.main.label(), "Install build");
    assert!(download.enabled);
    let on_disk = Facts {
        switch_build: Some(false),
        ..idle()
    }
    .view();
    assert_eq!(on_disk.main, Main::SwitchBuild);
    assert_eq!(on_disk.main.label(), "Switch build");
    // It needs no account, and it is not Play's own work: nothing is under way.
    let signed_out = Facts {
        signed_in: false,
        switch_build: Some(true),
        ..idle()
    }
    .view();
    assert_eq!(signed_out.main, Main::InstallBuild);
    assert!(!signed_out.main.under_way());
    // A running game, or work under way, still wins and disables it.
    let running = Facts {
        running: true,
        switch_build: Some(true),
        ..idle()
    }
    .view();
    assert_eq!(running.main, Main::Stop);
    let busy = Facts {
        mutating: true,
        switch_build: Some(true),
        ..idle()
    }
    .view();
    assert!(!busy.enabled);
    // No chosen build left to install: Play is back.
    assert_eq!(idle().view().main, Main::Play);
}

#[test]
fn signed_out_play_says_sign_in_and_signs_in_first() {
    let out = Facts {
        signed_in: false,
        ..idle()
    }
    .view();
    assert_eq!(out.main, Main::SignIn);
    assert!(out.enabled, "pressing it signs in, then plays");
    assert_eq!(out.main.label(), "Sign in to play");
    assert_eq!(out.main.tooltip(), None, "the label says it, not a tooltip");
    // Signed in: the normal Play, no tooltip.
    let signed_in = idle().view();
    assert_eq!(signed_in.main, Main::Play);
    assert!(signed_in.enabled);
    assert_eq!(signed_in.main.tooltip(), None);
    for main in [Main::Play, Main::Install, Main::Stop, Main::Preparing] {
        assert_eq!(main.tooltip(), None);
    }
}

#[test]
fn play_can_be_pressed_unless_something_is_under_way_or_the_data_is_lost() {
    assert!(idle().view().enabled);
    // Stop and the first install are presses too.
    for facts in [
        Facts {
            running: true,
            ..idle()
        },
        Facts {
            installed: false,
            ..idle()
        },
    ] {
        assert!(facts.view().enabled, "{facts:?}");
    }
    for facts in [
        Facts {
            installing: Some(0.),
            ..idle()
        },
        Facts {
            preparing: true,
            ..idle()
        },
        Facts {
            building: true,
            ..idle()
        },
        Facts {
            starting: true,
            ..idle()
        },
        Facts {
            mutating: true,
            ..idle()
        },
        Facts {
            located: false,
            ..idle()
        },
    ] {
        assert!(!facts.view().enabled, "{facts:?}");
    }
}

#[test]
fn a_build_under_way_has_stop_and_keeps_its_old_error_quiet() {
    let view = Facts {
        building: true,
        native_error: Some(LONG.into()),
        held: Some("Nov 17".into()),
        update: Some("Nov 17".into()),
        mods_off: 2,
        ..idle()
    }
    .view();
    assert_eq!(view.act, Some(Act::StopBuild));
    assert_eq!(view.act.as_ref().map(Act::label), Some("Stop"));
    assert!(!view.failed);
    assert_eq!(view.mods_off, None);
    assert_eq!(
        view.strip(),
        Some(("Building mods…".into(), StripAct::Act(Act::StopBuild)))
    );
}

#[test]
fn a_problem_is_one_sentence_with_details_when_there_is_more() {
    let view = Facts {
        native_error: Some(LONG.into()),
        ..idle()
    }
    .view();
    assert_eq!(view.said.as_deref(), Some("The build failed."));
    assert!(view.failed);
    let details = Act::Notice {
        text: LONG.into(),
        way_out: None,
    };
    assert_eq!(view.act.as_ref().map(Act::label), Some("Details…"));
    assert_eq!(view.act, Some(details.clone()));
    assert_eq!(
        view.strip(),
        Some(("The build failed.".into(), StripAct::Act(details)))
    );
}

#[test]
fn a_short_problem_has_no_action_and_the_strip_leads_home() {
    let view = Facts {
        client_error: Some("The game is already running.".into()),
        ..idle()
    }
    .view();
    assert_eq!(view.said.as_deref(), Some("The game is already running."));
    assert_eq!(view.act, None);
    assert_eq!(
        view.strip(),
        Some(("The game is already running.".into(), StripAct::Home))
    );
    assert_eq!(StripAct::Home.label(), "Go to Home");
}

#[test]
fn missing_game_files_are_fixed_by_choosing_the_folder() {
    let view = Facts {
        prepare_error: Some("Minecraft was not found here.".into()),
        // The channel's way out is the build's, never this problem's.
        build_gone: Some(instances::Channel::Stable),
        ..idle()
    }
    .view();
    let fix = Act::Notice {
        text: "Minecraft was not found here.".into(),
        way_out: Some(WayOut::MinecraftFolder),
    };
    assert_eq!(fix.label(), "Fix…");
    assert_eq!(view.act, Some(fix.clone()));
    assert_eq!(
        view.strip(),
        Some(("Minecraft was not found here.".into(), StripAct::Act(fix)))
    );
}

#[test]
fn a_build_that_is_gone_is_fixed_by_the_newest_of_its_channel_once_idle() {
    let gone = Facts {
        native_error: Some("No mod tools for it.".into()),
        build_gone: Some(instances::Channel::Nightly),
        ..idle()
    };
    assert_eq!(
        gone.view().act,
        Some(Act::Notice {
            text: "No mod tools for it.".into(),
            way_out: Some(WayOut::Newest(instances::Channel::Nightly)),
        })
    );
    // While something else runs the channel must not change under it.
    let busy = Facts {
        mutating: true,
        ..gone
    };
    assert_eq!(busy.view().act, None);
}

#[test]
fn problems_win_in_the_order_the_launch_meets_them() {
    let mut facts = Facts {
        client_error: Some("The client step failed.".into()),
        prepare_error: Some("The prepare step failed.".into()),
        native_error: Some("The native step failed.".into()),
        install_error: Some("The install step failed.".into()),
        crash: Some("The crash step failed.".into()),
        ..idle()
    };
    let steps: [(&str, Next); 5] = [
        ("The client step failed.", |f| f.client_error = None),
        ("The prepare step failed.", |f| f.prepare_error = None),
        ("The native step failed.", |f| f.native_error = None),
        ("The install step failed.", |f| f.install_error = None),
        ("The crash step failed.", |f| f.crash = None),
    ];
    for (said, next) in steps {
        assert_eq!(facts.view().said.as_deref(), Some(said));
        next(&mut facts);
    }
    assert_eq!(facts.view().said, None);
}

#[test]
fn a_held_update_speaks_for_the_build_and_is_fixed_in_its_sheet() {
    let held = Facts {
        native_error: Some(LONG.into()),
        held: Some("Nov 17".into()),
        update: Some("Nov 17".into()),
        ..idle()
    };
    let view = held.view();
    assert_eq!(
        view.said.as_deref(),
        Some("The update to Nov 17 is on hold")
    );
    assert!(!view.failed);
    assert_eq!(view.act, Some(Act::Resolve));
    assert_eq!(Act::Resolve.label(), "Fix…");
    assert_eq!(
        view.strip(),
        Some((
            "The update to Nov 17 is on hold".into(),
            StripAct::Act(Act::Resolve)
        ))
    );
    // Nothing is resolved while something runs; another problem is still said first.
    let busy = Facts {
        mutating: true,
        ..held.clone()
    };
    assert_eq!(busy.view().act, None);
    let other = Facts {
        crash: Some("The game stopped early.".into()),
        ..held
    };
    assert_eq!(
        other.view().said.as_deref(),
        Some("The game stopped early.")
    );
    assert_eq!(other.view().act, Some(Act::Resolve));
}

#[test]
fn a_newer_build_is_offered_only_to_an_idle_installed_game_and_only_on_home() {
    let newer = Facts {
        update: Some("Nov 18".into()),
        ..idle()
    };
    let view = newer.view();
    assert_eq!(view.said.as_deref(), Some("Nov 18 is available"));
    assert_eq!(view.act, Some(Act::Update));
    assert_eq!(Act::Update.label(), "Update game");
    assert_eq!(view.strip(), None);
    for facts in [
        Facts {
            running: true,
            ..newer.clone()
        },
        Facts {
            installed: false,
            ..newer.clone()
        },
        Facts {
            preparing: true,
            ..newer
        },
    ] {
        let view = facts.view();
        assert_eq!((view.said, view.act), (None, None), "{facts:?}");
    }
}

#[test]
fn mods_left_out_are_one_quiet_line_when_nothing_else_is_said() {
    let off = Facts {
        mods_off: 2,
        ..idle()
    };
    assert_eq!(off.view().mods_off, Some(2));
    assert_eq!(off.view().strip(), None);
    let said = Facts {
        crash: Some("The game stopped early.".into()),
        ..off
    };
    assert_eq!(said.view().mods_off, None);
}

#[test]
fn the_strip_shows_work_under_way_then_a_running_game_and_nothing_when_idle() {
    assert_eq!(idle().view().strip(), None);
    assert_eq!(
        Facts {
            signed_in: false,
            installed: false,
            ..idle()
        }
        .view()
        .strip(),
        None
    );
    let home = |facts: Facts, says: &str| {
        assert_eq!(
            facts.view().strip(),
            Some((says.to_string().into(), StripAct::Home)),
            "{facts:?}"
        );
    };
    home(
        Facts {
            installing: Some(0.5),
            ..idle()
        },
        "Installing · 50%",
    );
    home(
        Facts {
            preparing: true,
            ..idle()
        },
        "Preparing…",
    );
    home(
        Facts {
            starting: true,
            ..idle()
        },
        "Starting…",
    );
    home(
        Facts {
            mutating: true,
            ..idle()
        },
        "Changing mods…",
    );
    let running = Facts {
        running: true,
        ..idle()
    };
    assert_eq!(
        running.view().strip(),
        Some(("The game is running".into(), StripAct::StopGame))
    );
    assert_eq!(StripAct::StopGame.label(), "Stop");
    // What failed is said before the game that still runs.
    let failed = Facts {
        client_error: Some(LONG.into()),
        ..running
    };
    assert!(matches!(
        failed.view().strip(),
        Some((_, StripAct::Act(Act::Notice { .. })))
    ));
}

#[test]
fn the_first_sentence_is_what_is_said() {
    assert_eq!(first_sentence("  One. Two.\nThree"), "One.");
    assert_eq!(first_sentence("One line\nand another"), "One line");
    assert_eq!(first_sentence(""), "");
}

/// Puts a raw message where one step of Play leaves its error.
type Raise = fn(&mut Facts, String);

/// Each step's error, and what it says when nothing plain can be said of it.
const STEPS: [(Raise, &str); 4] = [
    (|f, e| f.client_error = Some(e), "The game couldn't start."),
    (
        |f, e| f.prepare_error = Some(e),
        "The game's files couldn't be prepared.",
    ),
    (
        |f, e| f.native_error = Some(e),
        "The mods couldn't be built.",
    ),
    (
        |f, e| f.install_error = Some(e),
        "The game couldn't be installed.",
    ),
];

const OFFLINE: &str = "No connection: https://github.com/sunrise-org/sunrise/releases/download/nightly/SHA256SUMS: Connection Failed: Connect error: Connection refused (os error 111)";

fn raised(raise: Raise, raw: &str) -> View {
    let mut facts = idle();
    raise(&mut facts, raw.to_string());
    facts.view()
}

/// The whole message, behind the one action.
fn details(view: &View) -> &str {
    match &view.act {
        Some(Act::Notice { text, .. }) => text,
        other => panic!("{:?}: {other:?}", view.said),
    }
}

#[test]
fn a_dead_connection_is_one_plain_line_with_the_address_behind_details() {
    let says = [
        "No connection: Sunrise couldn't check what it needs to play.",
        "No connection: the game's files couldn't be downloaded.",
        "No connection: the mod tools couldn't be downloaded.",
        "No connection: the game couldn't be downloaded.",
    ];
    for ((raise, _), says) in STEPS.into_iter().zip(says) {
        let view = raised(raise, OFFLINE);
        assert_eq!(view.said.as_deref(), Some(says));
        assert_eq!(details(&view), OFFLINE, "every word is still there to read");
        // The strip says the same line and opens the same sheet.
        let (line, act) = view.strip().unwrap();
        assert_eq!(line, says);
        assert_eq!(act, StripAct::Act(view.act.clone().unwrap()));
    }
}

#[test]
fn unconfirmed_spawn_does_not_say_the_game_could_not_start() {
    let message = "Sunrise may have started. Close the game if it opened before trying again.\nWrapper: /some/private/path\nbroken pipe (os error 32)";
    let view = raised(STEPS[0].0, message);
    assert_eq!(view.said.as_deref(), Some("Sunrise may have started."));
    assert_eq!(details(&view), message);
}

#[test]
fn what_a_program_or_the_system_said_is_never_put_beside_play() {
    let raw = [
        // A path with the system's error, on either system, and a bare path.
        "/home/ana/.local/share/sunrise/launcher/versions/nightly-2026-11-17/sunrise: Permission denied (os error 13)",
        "C:\\Users\\ana\\AppData\\Roaming\\sunrise\\launcher\\settings.json: Access is denied. (os error 5)",
        "/home/ana/.local/share/sunrise/launcher/versions/nightly-2026-11-17/sunrise",
        "C:\\Users\\ana\\AppData\\Roaming\\sunrise\\launcher\\versions",
        "\\\\server\\share\\sunrise\\settings.json",
        // An address that answered: a status, a checksum. Reached, so not "no connection".
        "HTTP 404 from https://github.com/sunrise-org/sunrise/releases/download/nightly/SHA256SUMS",
        "https://github.com/sunrise-org/sunrise/releases/download/nightly/sunrise-linux-x86_64.tar.gz: SHA-256 is 9f2c… but SHA256SUMS says 77ab…",
        "sunrise-linux-x86_64.tar.gz failed its signature check",
        // One-line diagnostics nobody wrote for a player.
        "invalid type: map, expected a string at line 3 column 5",
        "Permission denied",
        "Connection refused",
        "expected value at line 1 column 1",
        "Error { kind: InvalidData, message: \"stream did not contain valid UTF-8\" }",
        "serde_json::Error(\"expected value\", line: 1, column: 1)",
        "called `Result::unwrap()` on an `Err` value: Os { code: 2, kind: NotFound }",
        "ENOENT",
        "exit status: 101",
        "sunrise-prepare ended with exit status: 101\nthread 'main' panicked at src/main.rs:42",
        "error[E0432]: unresolved import `sunrise_mod_api::hooks`",
        "The requested operation requires elevation. (os error 740)",
        "Failed to read manifest: missing field `name` at line 1 column 2.",
        "",
    ];
    for (raise, stopped) in STEPS {
        for raw in raw {
            let view = raised(raise, raw);
            assert_eq!(view.said.as_deref(), Some(stopped), "{raw:?}");
            if raw.is_empty() {
                continue;
            }
            assert_eq!(
                details(&view),
                raw,
                "the whole message is behind the action"
            );
            assert_eq!(view.strip().map(|(line, _)| line), Some(stopped.into()));
        }
    }
    // Only a connection that was not made is called one.
    for reached in [
        "HTTP 503 from https://api.github.com/repos/sunrise-org/sunrise/releases",
        "The download stopped: No connection",
    ] {
        assert_eq!(
            plain(Step::Install, reached),
            "The game couldn't be installed."
        );
    }
}

#[test]
fn the_launchers_own_sentences_are_said_as_written_and_a_long_one_is_cut_at_a_word() {
    for own in [
        "The game is already running.",
        "Install a Sunrise game build first.",
        "Checking available builds. Press Play when the list is ready.",
        "No downloadable build matches this profile. Choose a game build.",
        "Wait for the preparation to finish, then press Play.",
        "Sunrise nightly-2026-11-17 is too old for this launcher: install a newer version.",
        "Nov 17 isn't prepared yet: choose Prepare in Play's menu.",
    ] {
        assert_eq!(plain(Step::Start, own), first_sentence(own), "{own}");
    }
    let long = "The game stopped before it opened a window because the graphics driver on this computer answered with something Sunrise has never seen from any driver before.";
    let said = plain(Step::Start, long);
    assert!(
        said.ends_with('…') && said.chars().count() <= BRIEF + 1,
        "{said}"
    );
    assert!(long.starts_with(said.trim_end_matches('…')));
    assert!(!said.trim_end_matches('…').ends_with(' '));
    let view = raised(STEPS[0].0, long);
    assert_eq!(details(&view), long);
    assert_eq!(view.act.as_ref().map(Act::label), Some("Details…"));
    // A plain first sentence is said; the lines after it wait behind Details.
    let two = "The build stopped for one mod.\nerror[E0432]: unresolved import in fixture_minimap/src/lib.rs";
    let view = raised(STEPS[2].0, two);
    assert_eq!(view.said.as_deref(), Some("The build stopped for one mod."));
    assert_eq!(details(&view), two);
    // A short problem with nothing more to read has no Details….
    assert_eq!(raised(STEPS[0].0, "The game is already running.").act, None);
    // A crash's summary is the launcher's own; one that isn't plain says only that it stopped.
    let crashed = |summary: &str| {
        Facts {
            crash: Some(summary.into()),
            ..idle()
        }
        .view()
        .said
    };
    assert_eq!(
        crashed("The game crashed while drawing. Its report is in the crash folder.").as_deref(),
        Some("The game crashed while drawing.")
    );
    assert_eq!(
        crashed("thread 'main' panicked at src/render.rs:9").as_deref(),
        Some("The game stopped.")
    );
}

#[test]
fn the_way_out_is_the_said_problems_own() {
    let missing =
        "sunrise-prepare: /home/ana/.minecraft/versions: No such file or directory (os error 2)";
    // The game's files not found: Fix… to the folder chooser, with the raw text whole.
    let view = raised(STEPS[1].0, missing);
    let fix = Act::Notice {
        text: missing.into(),
        way_out: Some(WayOut::MinecraftFolder),
    };
    assert_eq!(fix.label(), "Fix…");
    assert_eq!(view.act, Some(fix.clone()));
    assert_eq!(view.strip().map(|(_, act)| act), Some(StripAct::Act(fix)));
    // A start that failed is said first. Its action is its own Details…: the folder chooser
    // is the other problem's way out, and would fix nothing here.
    let both = Facts {
        client_error: Some(OFFLINE.into()),
        prepare_error: Some(missing.into()),
        ..idle()
    }
    .view();
    assert_eq!(
        both.said.as_deref(),
        Some("No connection: Sunrise couldn't check what it needs to play.")
    );
    assert_eq!(
        both.act,
        Some(Act::Notice {
            text: OFFLINE.into(),
            way_out: None,
        })
    );
    // The same for a build that is gone: its way out shows only when its problem is the one
    // said.
    let gone = Facts {
        native_error: Some(OFFLINE.into()),
        build_gone: Some(instances::Channel::Stable),
        ..idle()
    };
    assert_eq!(
        gone.view().act,
        Some(Act::Notice {
            text: OFFLINE.into(),
            way_out: Some(WayOut::Newest(instances::Channel::Stable)),
        })
    );
    let behind = Facts {
        client_error: Some("exit status: 101".into()),
        ..gone.clone()
    }
    .view();
    assert_eq!(behind.said.as_deref(), Some("The game couldn't start."));
    assert_eq!(
        behind.act,
        Some(Act::Notice {
            text: "exit status: 101".into(),
            way_out: None,
        })
    );
    // A short plain start problem has nothing to open, whatever else is left over.
    let quiet = Facts {
        client_error: Some("The game is already running.".into()),
        ..gone
    }
    .view();
    assert_eq!(quiet.act, None);
}

#[test]
fn an_interrupted_game_install_has_a_plain_message_and_exact_profile_folder() {
    let folder = std::path::PathBuf::from("profile-with-journal");
    let detail = "A mod install the game started was interrupted and couldn't be finished.\n.mod-browser.pending.json: duplicate project".to_owned();
    let facts = Facts {
        install_recovery: Some((folder.clone(), detail.clone())),
        ..idle()
    };
    let view = facts.view();
    assert_eq!(
        view.said.as_deref(),
        Some("A mod install couldn't be finished.")
    );
    assert!(
        view.failed && view.enabled,
        "Play can retry admission after repair"
    );
    assert_eq!(
        view.act,
        Some(Act::Notice {
            text: detail,
            way_out: Some(WayOut::ProfileFolder(folder)),
        })
    );
    assert!(
        idle().view().act.is_none(),
        "another profile has no repair action"
    );
}

#[test]
fn registry_install_shows_build_percent_and_offers_stop_until_publishing() {
    use sunrise_launcher::browser::sunrise::InstallProgress;
    let building = Facts {
        registry: Some(InstallProgress::Building(Some(0.42))),
        ..idle()
    }
    .view();
    assert_eq!(building.main.label(), "Building 42%");
    assert_eq!(
        building.strip(),
        Some(("Building 42%".into(), StripAct::StopInstall))
    );
    assert_eq!(StripAct::StopInstall.label(), "Stop");

    let tools = Facts {
        registry: Some(InstallProgress::Rust(0.3)),
        ..idle()
    }
    .view();
    assert_eq!(
        tools.strip(),
        Some(("Downloading build tools 30%".into(), StripAct::StopInstall))
    );

    let publishing = Facts {
        registry: Some(InstallProgress::Publishing),
        ..idle()
    }
    .view();
    assert_eq!(
        publishing.strip(),
        Some(("Finishing…".into(), StripAct::Home))
    );
}

#[test]
fn play_build_account_error_shows_the_account_refusal_instead_of_publish_fallback() {
    use crate::mods::build_run::{End, Run};
    use sunrise_launcher::account::NoOwner;

    for refused in [
        NoOwner::default(),
        NoOwner {
            keyring: Some(
                "Couldn't sign in to play online: Your Microsoft sign-in expired. Sign in again. Press Play to try again."
                    .into(),
            ),
        },
    ] {
        let mut run = Run::start(
            "inst".into(),
            "Default".into(),
            "Nightly".into(),
            Vec::new(),
            true,
        );
        let expected = refused.in_window();
        let captured = settle_play_build_error(&mut run, &StartError::Account(refused), false);
        assert!(
            !captured,
            "account refusal still runs the Play button's account-refusal path"
        );
        let Some(End::Stopped(stop)) = &run.end else {
            panic!("expected stopped build run");
        };
        assert_eq!(stop.summary, expected);
        assert!(stop.error.is_empty());
        assert_eq!(stop.action, None);
    }
}

#[test]
fn work_that_can_stop_gets_a_stop_in_the_strip_and_on_play() {
    let downloading = Facts {
        step: Some(backend::PlayStep::Download),
        starting: true,
        start_stoppable: true,
        ..idle()
    }
    .view();
    assert_eq!(downloading.halt, Some(Halt::Start));
    assert_eq!(
        downloading.strip(),
        Some(("Downloading the game…".into(), StripAct::Stop(Halt::Start)))
    );
    assert_eq!(StripAct::Stop(Halt::Start).label(), "Stop");
    assert!(!downloading.stopping);

    let winding_down = Facts {
        stopping: true,
        ..Facts {
            installing: Some(0.3),
            install_stoppable: true,
            ..idle()
        }
    }
    .view();
    assert_eq!(winding_down.halt, Some(Halt::Install));
    assert!(winding_down.stopping);

    let preparing = Facts {
        preparing: true,
        prepare_stoppable: true,
        ..idle()
    }
    .view();
    assert_eq!(
        preparing.strip(),
        Some(("Preparing…".into(), StripAct::Stop(Halt::Prepare)))
    );
}

#[test]
fn work_without_a_stop_has_none_and_a_stop_alone_changes_nothing() {
    // A pack's legacy start holds no stop: the strip keeps pointing Home.
    let legacy = Facts {
        starting: true,
        ..idle()
    }
    .view();
    assert_eq!(legacy.halt, None);
    assert_eq!(legacy.strip().map(|(_, act)| act), Some(StripAct::Home));
    // A stop with no work under way is nothing to show.
    let idle_with_stop = Facts {
        install_stoppable: true,
        prepare_stoppable: true,
        start_stoppable: true,
        stopping: true,
        ..idle()
    }
    .view();
    assert_eq!(idle_with_stop.halt, None);
    assert!(!idle_with_stop.stopping);
    assert_eq!(idle_with_stop.strip(), None);
}
