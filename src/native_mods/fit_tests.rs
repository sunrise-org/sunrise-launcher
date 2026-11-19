use super::*;

const OTHER_BUILD: &str = "cccccccccccccccccccccccccccccccccccccccc";

#[test]
fn at_most_five_changes_are_named_then_how_many_more() {
    let misfits: Vec<Misfit> = (0..7)
        .map(|n| Misfit {
            id: format!("fn:f{n}"),
            change: Change::Removed,
        })
        .collect();
    let said = changes(&misfits);
    assert!(said.contains("f4"), "{said}");
    assert!(!said.contains("f5"), "{said}");
    assert!(
        said.ends_with("and 2 more (all of them are in the build log)"),
        "{said}"
    );
    assert_eq!(changes(&misfits[..1]), "the function f0 (removed)");
}

#[test]
fn every_message_names_the_mod_the_builds_and_the_worlds() {
    let made_for = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned();
    let misfit = Verdict::DoesNotFit {
        made_for: made_for.clone(),
        misfits: vec![
            Misfit {
                id: "hook:player_logged_in".into(),
                change: Change::Changed,
            },
            Misfit {
                id: "method:ServerHost::give_item".into(),
                change: Change::Removed,
            },
        ],
    };
    let build = "nightly-2026-11-13 (afe471c)";
    assert_eq!(
        refusal("Greeter", &misfit, build),
        "Greeter doesn't fit this game.\nIt was made for another Sunrise version and this profile runs \
         nightly-2026-11-13 (afe471c). Since then the game changed 2 things that Greeter uses: the hook \
         player_logged_in (changed); the method ServerHost::give_item (removed).\nThe game will be \
         built without Greeter, which stays installed. Ask its author for an update made for this game."
    );
    let fits = Verdict::Fits {
        made_for: made_for.clone(),
    };
    assert_eq!(
        line("Greeter", &fits, build),
        "Made for another Sunrise version. Fits this game too."
    );
    assert!(fits.usable() && Verdict::MadeFor.usable() && !misfit.usable());
    assert_eq!(
        worlds_at_stake(
            "Greeter",
            "nightly-2026-11-20 (9a8b7c6)",
            &["Home".into(), "Test island".into()]
        ),
        "Greeter doesn't fit nightly-2026-11-20 (9a8b7c6).\n2 of your worlds use Greeter: “Home” \
         and “Test island”. Opening them without Greeter removes its blocks and what they hold, \
         for good."
    );
    assert!(worlds_at_stake("Greeter", build, &["Home".into()]).contains("One of your worlds"));
}

#[test]
fn a_missing_uses_list_names_the_pin_and_the_author_s_fix() {
    let verdict = Verdict::NoList {
        made_for: OTHER_BUILD.into(),
    };
    let said = line("Greeter", &verdict, "November 20 update");
    assert!(
        said.contains(&format!("Greeter pins build {OTHER_BUILD}")),
        "{said}"
    );
    assert!(said.contains("`cargo sunrise-mod build`"), "{said}");
    assert!(
        said.contains("`cargo sunrise-mod update --sunrise <build>`"),
        "{said}"
    );
    assert!(!said.contains("older tool"), "{said}");
}

#[test]
fn an_update_says_how_many_mods_fit_and_which_will_be_switched_off() {
    let build = "nightly-2026-11-20 (9a8b7c6)";
    assert_eq!(
        update_summary(build, 1, &[]),
        "Move to nightly-2026-11-20 (9a8b7c6)? Your native mod fits it."
    );
    assert_eq!(
        update_summary(build, 4, &[]),
        "Move to nightly-2026-11-20 (9a8b7c6)? All 4 of your native mods fit it."
    );
    let misfit = Verdict::DoesNotFit {
        made_for: OTHER_BUILD.into(),
        misfits: vec![Misfit {
            id: "hook:player_logged_in".into(),
            change: Change::Changed,
        }],
    };
    let off = [("Greeter".to_owned(), short(&misfit))];
    assert_eq!(
        update_summary(build, 4, &off),
        "Move to nightly-2026-11-20 (9a8b7c6)? 3 of your 4 native mods fit it.\nGreeter does \
         not: the hook player_logged_in (changed). It will be switched off until its author makes a version that fits."
    );
    let exact = Verdict::Exact {
        made_for: OTHER_BUILD.into(),
        why: "[[native.mixin]]".into(),
    };
    assert_eq!(short(&exact), "it changes the game's own code");
}

#[test]
fn the_worlds_buttons_and_the_stopped_mod_say_what_is_so() {
    assert_eq!(
        actions_pending("Greeter"),
        "The build is stopped until you choose under Native mod build; Greeter stays on until then."
    );
    assert!(did_not_build_pending("Greeter").contains("stays on until you choose"));
    assert!(did_not_build_last("Greeter").contains("didn't build the last time"));
}

#[test]
fn several_mods_at_stake_are_said_in_two_sentences() {
    let said = mods_at_stake(
        &["butterflies".into(), "fairies".into(), "worms".into()],
        "the Nov 19 update",
        &["New World".into()],
    );
    assert_eq!(said.lines().count(), 2);
    assert!(said.starts_with(
        "3 mods don't work with the Nov 19 update yet: butterflies, fairies and worms."
    ));
    assert!(said.contains("Your world “New World” uses them."));
}

#[test]
fn a_thousand_mods_at_stake_stay_two_short_sentences() {
    let names: Vec<String> = (0..1000).map(|n| format!("mod{n}")).collect();
    let said = mods_at_stake(&names, "the Nov 19 update", &["New World".into()]);
    assert_eq!(said.lines().count(), 2);
    assert!(said.contains("mod0, mod1, mod2, mod3, mod4 and 995 more."));
    assert!(said.len() < 300);
}
