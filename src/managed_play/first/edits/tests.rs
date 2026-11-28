use super::*;
use crate::{profile_txn::Boundary, worlds};
use std::{cell::Cell, collections::BTreeMap, process::Command};
use sunrise_launcher_nbt::{self as nbt, Limits, NamedTag, Tag};

#[test]
fn world_locks_span_live_publication_and_undo() {
    const CASE: &str =
        "managed_play::first::edits::tests::world_locks_span_live_publication_and_undo";
    const ROLE: &str = "SUNRISE_EDIT_WORLD_LOCK_ACTOR";
    let actor = |role| {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", CASE, "--nocapture"])
            .env(ROLE, role);
        command
    };
    let Ok(role) = std::env::var(ROLE) else {
        let temp = tempfile::tempdir().unwrap();
        let root = backend::plain(temp.path().canonicalize().unwrap());
        assert!(actor("publish")
            .env("SUNRISE_HOME", root.join("game"))
            .env("SUNRISE_LAUNCHER_HOME", root.join("launcher"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .status()
            .unwrap()
            .success());
        return;
    };
    let home = backend::game_home();
    let world = home.join("saves/basin");
    if role != "publish" {
        // Like the game, open the save lock independently, without launcher admission.
        let file = fs::File::options()
            .write(true)
            .open(world.join("session.lock"))
            .unwrap();
        match role.as_str() {
            "blocked" => assert!(matches!(file.try_lock(), Err(fs::TryLockError::WouldBlock))),
            "released" => file.try_lock().unwrap(),
            _ => panic!("unknown world-lock actor: {role}"),
        }
        return;
    }

    const BASE: &str = "v0.1.7";
    const POSTIMAGE: &[u8] = br#"{"entries":[]}"#;
    const INTERRUPTED: &str = "injected failure after publishing the pointer";
    let version = backend::version_dir(BASE);
    let resources = backend::resources(BASE);
    fs::create_dir_all(&resources).unwrap();
    fs::write(resources.join("sunrise-resources.json"), b"{}").unwrap();
    fs::write(version.join(".installed"), []).unwrap();
    fs::write(version.join(backend::CLIENT), b"local fixture client").unwrap();
    fs::create_dir_all(&world).unwrap();
    let bytes = nbt::write_named(
        &NamedTag {
            name: "".into(),
            tag: Tag::compound([(
                "Data",
                Tag::compound([("LevelName", Tag::String("Basin".into()))]),
            )]),
        },
        Limits::saved(),
    )
    .unwrap();
    fs::write(world.join("level.dat"), bytes).unwrap();

    let mut profile = Profile::open(&home, None).unwrap();
    let cancellation = native_mods::Cancellation::default();
    let old = profile
        .begin_first(|| FirstBuild::installed(BASE), |_| {})
        .unwrap()
        .commit(BASE, &cancellation)
        .unwrap();
    let before = profile_txn::references(&home).unwrap().pointer.unwrap();
    let live_manifest = home.join(sunrise_launcher_smod::index::browser_state::MANIFEST);
    let sources = BTreeMap::from([("lantern".into(), BTreeSet::new())]);
    let reviewed = worlds::review_risk(&home, &sources).unwrap();
    assert_eq!(reviewed.len(), 1);
    assert!(!reviewed[0].malformed);

    for undo in [true, false] {
        let edit = profile
            .begin_edit_cancellable(|| panic!("must use active base"), |_| {}, None)
            .unwrap();
        let candidate = edit.candidate().unwrap();
        fs::write(
            candidate
                .path()
                .join(sunrise_launcher_smod::index::browser_state::MANIFEST),
            POSTIMAGE,
        )
        .unwrap();
        let proof = edit
            .prove(
                candidate.path(),
                native_mods::Profile::Optimized,
                &cancellation,
                |_| {},
                None,
            )
            .unwrap();
        let mut before_first_rename = false;
        let mut after_pointer_rename = false;
        let settled_at_drop = Cell::new(false);
        let result = edit.publish_inner(
            proof,
            &cancellation,
            (None, &BTreeSet::new(), None),
            || {
                let locks = worlds::lock_reviewed(&reviewed)?;
                assert_eq!(worlds::review_risk(&home, &sources)?, reviewed);
                Ok(Authorization {
                    _locks: locks,
                    home: &home,
                    settled_at_drop: &settled_at_drop,
                })
            },
            || Ok(()),
            |step, boundary| {
                let refs = profile_txn::references(&home)?;
                if step == 0 && boundary == Boundary::BeforeRename {
                    assert_eq!(refs.pointer.as_deref(), Some(before.as_slice()));
                    assert!(!live_manifest.exists());
                    assert!(actor("blocked").status().unwrap().success());
                    before_first_rename = true;
                }
                if boundary == Boundary::Renamed
                    && refs.pointer.as_deref() != Some(before.as_slice())
                {
                    assert!(refs.pointer.is_some());
                    assert_eq!(refs.pointer, refs.after);
                    assert_eq!(fs::read(&live_manifest).unwrap(), POSTIMAGE);
                    assert!(actor("blocked").status().unwrap().success());
                    after_pointer_rename = true;
                    if undo {
                        return Err(INTERRUPTED.into());
                    }
                }
                Ok(())
            },
        );
        assert!(before_first_rename && after_pointer_rename);
        assert!(
            settled_at_drop.get(),
            "authorization dropped before the journal settled"
        );
        assert!(actor("released").status().unwrap().success());
        if undo {
            assert_eq!(result.unwrap_err(), INTERRUPTED);
            assert_eq!(profile.active().unwrap().as_ref(), Some(&old));
            assert!(!live_manifest.exists());
        } else {
            let active = result.unwrap();
            assert_ne!(active, old);
            assert_eq!(profile.active().unwrap(), Some(active));
            assert_eq!(fs::read(&live_manifest).unwrap(), POSTIMAGE);
        }
    }
}

struct Authorization<'a> {
    _locks: worlds::ReviewedWorldLocks,
    home: &'a Path,
    settled_at_drop: &'a Cell<bool>,
}

impl Drop for Authorization<'_> {
    fn drop(&mut self) {
        // Observe the journal while the real world guard is still held.
        self.settled_at_drop
            .set(profile_txn::references(self.home).is_ok_and(|refs| refs.after.is_none()));
    }
}
