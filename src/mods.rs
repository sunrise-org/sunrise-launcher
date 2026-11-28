//! Mods: Discover, Installed and a mod's page.
//!
//! **Real:** the registry list, search, categories, fit, kind and sort (`browser::sunrise`'s index,
//! read in a worker); a mod's page from the index (versions and their fit, size, licence,
//! authors, source, mirrored screenshots, what it may do); Add and single Update through the
//! existing install path; Installed from `mod-browser.json` and `sunrise-mods.txt` with on/off,
//! selection, filters, bulk Turn on, Turn off, Remove and Update N mods (a reviewed plan, applied
//! by a worker); Build… hands over to the one native build path.
//!
//! **Not here, said on screen or in the handoff:** Featured (the registry carries no editorial
//! list), a version picker (the registry installs the newest that fits), dependency lists for
//! plain mods (the index lists a mod's needs for packs only), "Add from a file…", resource,
//! shader and data packs, and registry modpacks.
//!
//! **Writes** happen only for an action the player took: install, update, on/off, remove. Showing
//! the screens reads the index (its own cache refresh, in a worker), `mod-browser.json`,
//! `sunrise-mods.txt` and file times. Nothing turns off or repairs a mod while drawing: taking
//! down what the registry revoked is left to `before_play` and the row says so.
use super::*;
use crate::frame::Page;
use crate::frame::Reach as _;
use crate::line_edit::{self, Key, LineEdit};
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use sunrise_launcher::browser::{
    installed::{self, Entry},
    mod_list, sunrise, Kind, SourceId,
};

mod actions;
mod build_modal;
pub(crate) mod build_run;
mod collection_v2;
mod data_packs;
mod discover;
mod installed_view;
mod model;
mod nightly;
mod pack_discover;
mod pack_project;
mod packs;
mod ports;
pub(crate) use crate::sheets::PortSwitch;
mod project;
mod rating;
pub(crate) mod refused;
mod release_builds;
mod search;
mod ui;
pub(crate) use data_packs::names;
pub(crate) use search::Installed;
mod viewer;

mod catalog;
mod installed_model;
mod messages;
mod navigation;
mod rows;
mod state;
mod view;

use catalog::*;
use installed_model::*;
pub(crate) use installed_model::{HomeModFacts, InstalledDrawn};
use messages::*;
pub(crate) use state::ModsState;
use state::*;
use std::{rc::Rc, sync::Arc};
use sunrise_launcher::native_mods::local_fit;
pub(crate) use ui::{checkbox, choices, empty_state, field_theme, reveal, switch, Choose};

pub(crate) use crate::frame::segmented;

use crate::paint::palette;
use model::Row;

const ADDED_PENDING: &str = crate::i18n::tr_key("Added · builds when you play");

/// The read-only Home view uses the same Installed classification as the full rows.
pub(crate) use model::ManualBadge;
pub(crate) use ui::manual_badge_pill;

#[cfg(test)]
mod enabled_tests {
    use super::*;

    #[test]
    fn reopening_the_same_mod_discards_previous_detail_replies() {
        let scope = |profile: &str, target: &str, game: &str| OpenScope {
            profile: Some(profile.into()),
            home: PathBuf::from(profile),
            target: Some(target.into()),
            game: game.into(),
        };
        let current = scope("current", "new-build", "26.3");
        let mut open = Open {
            id: "ruby".into(),
            installed: Some(InstalledProject::Registry),
            collection: false,
            v2: false,
            request: 2,
            scope: scope("current", "new-build", "26.3"),
            from: Page::Installed,
            v2_page: None,
            parent: None,
            removing: Vec::new(),
        };
        open.finish_v2(1, &current, Err("earlier request".into()));
        open.finish_v2(
            2,
            &scope("old", "new-build", "26.3"),
            Err("old profile".into()),
        );
        open.finish_v2(
            2,
            &scope("current", "old-build", "26.3"),
            Err("old build".into()),
        );
        open.finish_v2(
            2,
            &scope("current", "new-build", "26.2"),
            Err("old game".into()),
        );
        assert!(open.v2_page.is_none());
        open.finish_v2(2, &current, Err("current reply".into()));
        open.finish_v2(1, &current, Err("earlier request".into()));
        assert!(matches!(open.v2_page, Some(Err(why)) if why == "current reply"));
    }

    #[test]
    fn enabled_ids_come_from_the_current_packages_only() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join("sunrise-mods.txt"),
            "mod.kept: true\nmod.gone: true\nmod.off: false\n",
        )
        .unwrap();
        let local = |id: &str| model::Local {
            id: id.into(),
            file: format!("{id}.smod"),
            name: id.into(),
            version: String::new(),
            github: None,
            native: false,
            served_by: None,
            unreadable: false,
        };
        // "gone" was a package of the list known before and is no longer there.
        let got = enabled_ids(home.path(), &[], &[local("kept"), local("off")]);
        assert_eq!(got, HashSet::from(["kept".to_string()]));
    }
}

#[cfg(test)]
pub(crate) mod home_tests;

#[cfg(test)]
mod installed_refresh_tests {
    use super::*;

    #[test]
    fn the_benchmark_times_installed_only_once_rows_read_afresh_are_drawn() {
        use crate::perf::Target;
        let laid = Some((-10., 400.));
        // A refresh draws the last read's rows while it reads: they are waited past, not timed.
        let retained = InstalledDrawn::of(true, false, 12);
        assert_eq!(retained, InstalledDrawn::Reading);
        assert_eq!(installed_target(Some(retained), laid), Target::LayingOut);
        // Then the fresh rows: timed from the frame that drew them.
        let fresh = InstalledDrawn::of(false, false, 14);
        assert_eq!(fresh, InstalledDrawn::Rows);
        assert_eq!(
            installed_target(Some(fresh), laid),
            Target::Laid {
                offset: -10.,
                max: 400.
            }
        );
        assert_eq!(installed_target(Some(fresh), None), Target::LayingOut);
        // An empty profile draws no list: it stops at once, saying so, not at the timeout.
        let empty = InstalledDrawn::of(false, false, 0);
        assert_eq!(empty, InstalledDrawn::Empty);
        assert_eq!(installed_target(Some(empty), None), Target::Empty);
        // A failed read over the rows from before it stops too, with its own reason.
        let failed = InstalledDrawn::of(false, true, 12);
        assert_eq!(failed, InstalledDrawn::Failed);
        assert_eq!(installed_target(Some(failed), laid), Target::Failed);
        // Not drawn yet.
        assert_eq!(installed_target(None, laid), Target::LayingOut);
    }

    #[test]
    fn busy_publication_keeps_the_settled_hold_and_disables_actions_until_refresh() {
        use sunrise_launcher_smod::{
            locks::{LockMode, Release},
            runtime_leases::Registry,
        };
        const ACTOR: &str = "INSTALLED_BUSY_PUBLICATION_ACTOR";
        let Some(root) = std::env::var_os(ACTOR).map(PathBuf::from) else {
            let scratch = tempfile::tempdir().unwrap();
            let root = scratch.path();
            assert!(std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "mods::installed_refresh_tests::busy_publication_keeps_the_settled_hold_and_disables_actions_until_refresh"])
                .env(ACTOR, root).env("SUNRISE_LAUNCHER_HOME", root.join("launcher"))
                .status().unwrap().success());
            return;
        };
        let home = root.join("game");
        let registry = Registry::with_release(&backend::data_dir(), Release::Close).unwrap();
        let publication = registry.publication(LockMode::Exclusive).unwrap();
        let mut snapshot = Snapshot {
            key: "viewed fingerprint".into(),
            home: home.clone(),
            rows: InstalledRows {
                switch_held: std::collections::HashMap::from([(
                    "ruby".into(),
                    sunrise_launcher::managed_play::HeldReason::NoFit {
                        detail: None,
                        build: "fixture".into(),
                    },
                )]),
                ..Default::default()
            },
            reading: true,
            error: None,
            backup_notice: None,
        };
        assert!(!snapshot.finish(InstalledRows::read(
            &home,
            native_mods::Profile::Optimized,
            None
        )));
        assert!(!snapshot.ready());
        assert!(matches!(
            InstalledRefresh::of(Some(&snapshot)),
            InstalledRefresh::Reading
        ));
        assert!(snapshot.switch_held.contains_key("ruby"));
        assert!(
            snapshot.key.is_empty(),
            "the same fingerprint must be reread"
        );
        drop(publication);
        assert!(snapshot.finish(InstalledRows::read(
            &home,
            native_mods::Profile::Optimized,
            None
        )));
        assert!(snapshot.ready());
    }

    #[test]
    fn retry_after_a_game_journal_error_keeps_the_last_good_rows_until_success() {
        let home = tempfile::tempdir().unwrap();
        let journal = home
            .path()
            .join(sunrise_launcher_smod::index::browser_state::PENDING);
        std::fs::write(&journal, b"not a journal").unwrap();
        let mut snapshot = Snapshot {
            key: String::new(),
            home: home.path().into(),
            rows: InstalledRows {
                enabled: HashSet::from(["ruby".into()]),
                ..Default::default()
            },
            reading: true,
            error: None,
            backup_notice: None,
        };
        assert!(matches!(
            InstalledRefresh::of(None),
            InstalledRefresh::Reading
        ));
        assert!(matches!(
            InstalledRefresh::of(Some(&snapshot)),
            InstalledRefresh::Reading
        ));
        snapshot.finish(InstalledRows::read(
            home.path(),
            native_mods::Profile::Optimized,
            None,
        ));
        assert!(snapshot.enabled.contains("ruby"));
        assert!(matches!(
            InstalledRefresh::of(Some(&snapshot)),
            InstalledRefresh::Failed(InstalledReadError::GameInstall(_), _)
        ));
        assert_eq!(std::fs::read(&journal).unwrap(), b"not a journal");
        std::fs::remove_file(journal).unwrap();
        snapshot.finish(InstalledRows::read(
            home.path(),
            native_mods::Profile::Optimized,
            None,
        ));
        assert!(matches!(
            InstalledRefresh::of(Some(&snapshot)),
            InstalledRefresh::Ready
        ));
        assert!(snapshot.enabled.is_empty());
    }

    #[test]
    fn the_backup_advisory_is_the_last_fallback() {
        let backup = "backup kept".to_owned();
        let mut mods = ModsState {
            snapshot: Some(Snapshot {
                key: String::new(),
                home: PathBuf::new(),
                rows: InstalledRows::default(),
                reading: false,
                error: None,
                backup_notice: Some(backup.clone()),
            }),
            ..ModsState::default()
        };
        let install = Err("shared install error".to_owned());
        assert_eq!(mods.visible_notice(Some(&install)), Some(install.clone()));
        let batch = Err("batch error".to_owned());
        mods.notice = Some(batch.clone());
        assert_eq!(mods.visible_notice(Some(&install)), Some(batch));
        mods.notice = None;
        assert_eq!(mods.visible_notice(None), Some(Ok(backup)));
    }

    #[test]
    fn home_mod_labels_invalidate_when_unfit_updates_revision_changes() {
        let cached = HomeModLabels {
            inventory: 7,
            unfit_revision: 3,
            rows: Arc::new(HashMap::new()),
        };
        assert!(cached.matches(7, 3));
        assert!(
            !cached.matches(7, 4),
            "a newly recorded unfit update must refresh Home's mod status even with unchanged inventory"
        );
    }
}
