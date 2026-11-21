use super::{check_source_set, SourceError};
use crate::native_mods::Cancellation;
use std::{fs, path::Path};
use sunrise_launcher_smod::{dependencies, manifest, mod_list, Package};

fn package(home: &Path, id: &str, rules: &str, native: bool) {
    let mut manifest = format!(
        "id='{id}'\nname='{id}'\nversion='1.0.0'\napi='{}'\nside='both'\ncapabilities=['registries']\n{rules}\n",
        manifest::API,
    );
    let (root, files): (_, &[(&str, &[u8])]) = if native {
        manifest += &format!(
            "[native]\ngame='{}'\nsunrise='{}'\nmixins='{}'\n",
            crate::backend::PINNED_GAME,
            "a".repeat(40),
            "b".repeat(40),
        );
        (
            mod_list::NATIVE_DIR,
            &[
                (
                    "Cargo.toml",
                    b"[package]\nname='graph_fixture'\nversion='1.0.0'\n",
                ),
                ("src/lib.rs", b"pub fn inert() {}\n"),
            ],
        )
    } else {
        (mod_list::DIR, &[])
    };
    let root = home.join(root);
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join(format!("{id}.smod")),
        Package::write_files(&manifest, files).unwrap(),
    )
    .unwrap();
}

const BASE: &str = "developer-graph-fixture";

#[test]
fn rejects_dispatch_cycle_with_enabled_content_and_restores_trial_selection() {
    let home = tempfile::tempdir().unwrap();
    let home = home.path();
    package(home, "graph_lantern", "load_before=['graph_terrain']", true);
    package(
        home,
        "graph_terrain",
        "load_before=['graph_lantern']",
        false,
    );
    let selection = home.join(mod_list::FILE);
    let before = "mod.graph_terrain:true\nmod.graph_lantern:false\n";
    fs::write(&selection, before).unwrap();

    assert_eq!(
        check_source_set(
            home,
            &["graph_lantern".into()],
            BASE,
            &Cancellation::default()
        ),
        Err(SourceError::Dependency {
            required: None,
            detail: "Mod dependency cycle: graph_lantern -> graph_terrain -> graph_lantern".into(),
        }),
    );
    assert_eq!(fs::read_to_string(selection).unwrap(), before);
}

#[test]
fn rejects_code_link_to_content_even_with_dispatch_override() {
    let home = tempfile::tempdir().unwrap();
    let home = home.path();
    package(
        home,
        "graph_lantern",
        "[dependencies]\ngraph_terrain='^1'",
        true,
    );
    package(home, "graph_terrain", "", false);
    fs::write(
        home.join(mod_list::FILE),
        "mod.graph_lantern:true\nmod.graph_terrain:true\n",
    )
    .unwrap();
    fs::write(
        home.join(mod_list::DIR).join(dependencies::OVERRIDES),
        "[graph_lantern]\ngraph_terrain='drop'\n",
    )
    .unwrap();

    assert_eq!(
        check_source_set(home, &[], BASE, &Cancellation::default()),
        Err(SourceError::Dependency {
            required: None,
            detail:
                "code link requirements refused: graph_lantern requires missing mod graph_terrain"
                    .into(),
        }),
    );
}
