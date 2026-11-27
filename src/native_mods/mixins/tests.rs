use super::*;
use crate::native_mods::{self, Profile, Target};

const BUILD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_BUILD: &str = "cccccccccccccccccccccccccccccccccccccccc";
const EXPLICIT: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const FROM_SDK: &str = "dddddddddddddddddddddddddddddddddddddddd";

fn project(dir: &Path, id: &str, build: &str, pin: Option<&str>) -> Bundle {
    native_mods::tests::write_bundle(dir, id, "");
    let manifest = dir.join(NATIVE_MANIFEST);
    let text = fs::read_to_string(&manifest).unwrap();
    let text = text
        .replace(BUILD, build)
        .replace(&format!("mixins='{EXPLICIT}'\n"), "");
    fs::write(
        manifest,
        text + &pin.map_or_else(String::new, |pin| format!("mixins='{pin}'\n")),
    )
    .unwrap();
    native_mods::bundle_in(dir).unwrap().unwrap()
}

#[test]
fn only_omitted_pins_look_up_their_own_sdk() {
    let scratch = tempfile::tempdir().unwrap();
    let mods = [
        project(
            &scratch.path().join("explicit"),
            "explicit",
            BUILD,
            Some(EXPLICIT),
        ),
        project(
            &scratch.path().join("implicit"),
            "implicit",
            OTHER_BUILD,
            None,
        ),
        project(&scratch.path().join("same"), "same", OTHER_BUILD, None),
    ];
    let mut looked_up = Vec::new();
    let pins = resolve(&mods, |commit| {
        looked_up.push(commit.to_owned());
        Ok(FROM_SDK.into())
    })
    .unwrap();
    assert_eq!(looked_up, [OTHER_BUILD]);
    assert_eq!(pins, [EXPLICIT, FROM_SDK, FROM_SDK]);
    assert_eq!(
        for_key(&pins[..1], |_| panic!("a lone pin needs no date")).unwrap(),
        EXPLICIT
    );
    assert_eq!(
        for_key(&pins, |pin| Ok(if pin == FROM_SDK {
            "2026-11-20"
        } else {
            "2026-10-20"
        }
        .into()))
        .unwrap(),
        FROM_SDK
    );
    let error = resolve(&mods, |_| Err(String::from("SDK unavailable").into()))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("implicit") && error.contains("SDK unavailable"),
        "{error}"
    );
}

#[test]
fn an_inherited_revision_must_be_an_exact_commit() {
    let scratch = tempfile::tempdir().unwrap();
    let package = project(&scratch.path().join("implicit"), "implicit", BUILD, None);
    for rev in [
        String::new(),
        "f".repeat(39),
        "F".repeat(40),
        "g".repeat(40),
    ] {
        let err = resolve(std::slice::from_ref(&package), |_| Ok(rev.clone()))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("implicit") && err.contains("mixins.rev"),
            "{err}"
        );
    }
}

#[test]
fn the_resolved_pin_is_passed_to_older_tools_and_changes_the_key() {
    let scratch = tempfile::tempdir().unwrap();
    let project = project(
        &scratch.path().join("source"),
        "implicit",
        OTHER_BUILD,
        None,
    );
    let original = fs::read(project.dir.join(NATIVE_MANIFEST)).unwrap();
    let staged = scratch.path().join("staged");
    native_mods::copy_bundle(&project, &staged).unwrap();
    let key = |revision: &str| {
        let pins = resolve(std::slice::from_ref(&project), |_| Ok(revision.into())).unwrap();
        materialize(&project, &staged, &pins[0]).unwrap();
        let staged = native_mods::bundle_in(&staged).unwrap().unwrap();
        assert_eq!(staged.native().mixins.as_deref(), Some(revision));
        let mixins = for_key(&pins, |_| panic!("a lone pin needs no date")).unwrap();
        // Even on a different game build, the key and tool input use the mod's pinned SDK.
        native_mods::version_for(
            BUILD,
            &mixins,
            Profile::Optimized,
            Target::Client,
            None,
            &[staged],
            &[],
        )
        .unwrap()
    };
    assert_ne!(key(FROM_SDK), key(EXPLICIT));
    assert_eq!(key(FROM_SDK), key(FROM_SDK));
    assert_eq!(
        fs::read(project.dir.join(NATIVE_MANIFEST)).unwrap(),
        original
    );
}
