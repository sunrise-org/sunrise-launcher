use super::*;

fn account(uuid: &str, name: &str) -> account::Account {
    account::Account {
        uuid: uuid.into(),
        name: name.into(),
        skin: None,
        cape: None,
    }
}

fn three(selected: Option<&str>) -> Accounts {
    Accounts {
        accounts: vec![
            account("a1", "Juniper"),
            account("b2", "Mossbyte"),
            account("c3", "Cinder"),
        ],
        selected: selected.map(Into::into),
    }
}

#[test]
fn every_account_is_listed_and_only_the_one_that_plays_is_ticked() {
    let none = std::collections::HashMap::new();
    let listed = rows(&three(Some("b2")), &none);
    let names: Vec<_> = listed.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["Juniper", "Mossbyte", "Cinder"]);
    let ticked: Vec<_> = listed.iter().map(|r| r.plays).collect();
    assert_eq!(ticked, [false, true, false]);
    assert!(listed.iter().all(|r| !r.signed_out));
}

#[test]
fn the_tick_follows_a_switch_and_nobody_is_ticked_when_nobody_is_picked() {
    let none = std::collections::HashMap::new();
    let mut accounts = three(Some("a1"));
    accounts.play_as("c3").unwrap();
    let listed = rows(&accounts, &none);
    assert_eq!(
        listed.iter().position(|r| r.plays),
        Some(2),
        "the row switched to"
    );
    assert_eq!(listed.iter().filter(|r| r.plays).count(), 1);
    assert!(rows(&three(None), &none).iter().all(|r| !r.plays));
}

#[test]
fn signed_out_accounts_are_flagged_in_the_menu() {
    let mut status = std::collections::HashMap::new();
    status.insert(
        "a1".into(),
        account::session::Status {
            state: account::session::State::SignInRequired,
            expires_at: None,
            message: "Sign in again in your browser.".into(),
        },
    );
    status.insert(
        "b2".into(),
        account::session::Status {
            state: account::session::State::Ready,
            expires_at: None,
            message: "Ready".into(),
        },
    );
    let listed = rows(&three(Some("b2")), &status);
    let flags: Vec<_> = listed.iter().map(|r| r.signed_out).collect();
    assert_eq!(flags, [true, false, false]);
}

#[test]
fn adding_keeps_the_menu_and_every_other_action_closes_it() {
    assert_eq!(sheet_after(&Action::Add), Some(Sheet::AccountMenu));
    for closes in [
        Action::PlayAs("a1".into()),
        Action::Manage,
        Action::Settings,
        Action::SignInAgain("a1".into()),
        Action::CopyUuid("a1".into()),
        Action::ManageSkin("a1".into()),
        Action::Remove("a1".into()),
    ] {
        assert_eq!(sheet_after(&closes), None, "{closes:?}");
    }
}

#[test]
fn the_menu_is_back_when_the_sign_in_it_started_ends() {
    let open = sheet_after(&Action::Add);
    assert!(!shows(open, true), "the sign-in is in front");
    // Done, refused or cancelled: the sign-in screen closes and nothing else changed.
    assert!(shows(open, false));
    assert!(!shows(None, false));
    assert!(!shows(Some(Sheet::Build), false));
}

#[test]
fn a_sign_in_from_the_menu_adds_an_account_and_never_starts_the_game() {
    assert!(!plays_after_sign_in(None, Some("sunrise")));
    // One a Play asked for does, for the setup it was pressed on.
    assert!(plays_after_sign_in(Some("sunrise".into()), Some("sunrise")));
    assert!(!plays_after_sign_in(Some("pack".into()), Some("sunrise")));
    assert!(!plays_after_sign_in(Some("pack".into()), None));
}

#[test]
fn the_sign_in_opened_to_add_an_account_is_titled_so() {
    assert_eq!(sign_in_title(false, false, 3), "Add a Microsoft account");
    // Nobody signed in yet, a Play that needs it, or one account signing in again.
    assert_eq!(sign_in_title(false, false, 0), "Sign in to play");
    assert_eq!(sign_in_title(true, false, 3), "Sign in to play");
    assert_eq!(sign_in_title(false, true, 3), "Sign in to play");
}

#[test]
fn the_menu_hangs_under_the_chip_and_stays_inside_the_window() {
    let chip = Bounds::new(
        gpui::point(px(1082.), px(20.)),
        gpui::size(px(172.), px(44.)),
    );
    assert_eq!(place(Some(chip), 1280.), (70., 26.));
    // A chip against the window's edge: the menu keeps its margin.
    let flush = Bounds::new(gpui::point(px(628.), px(0.)), gpui::size(px(172.), px(44.)));
    assert_eq!(place(Some(flush), 800.), (50., EDGE));
    assert_eq!(
        place(None, 800.),
        (tokens::FRAME_INSET + tokens::HEADER + GAP, EDGE)
    );
}
