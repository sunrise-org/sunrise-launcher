use super::*;
use crate::backend::Settings;

const NIGHT_ROLES: Roles = Roles {
    background: 0x2b2324,
    card: 0x322829,
    chrome: 0x1e1b1a,
    text: 0xf3eae6,
    muted: 0xb9aba6,
    accent: 0x112233,
    line: 0xf7c9a2,
    success: 0x7dc98c,
    danger: 0xec8d8d,
    light: false,
};

fn names(root: &Path) -> Vec<String> {
    saved(root).into_iter().map(|l| l.name).collect()
}

#[test]
fn every_colour_is_one_slot_and_a_theme_file_says_each() {
    assert_eq!(SLOTS[ACCENT].key, "accent");
    let mut roles = NIGHT_ROLES;
    for (n, slot) in SLOTS.iter().enumerate() {
        (slot.set)(&mut roles, 0x100000 + n as u32);
    }
    for (n, slot) in SLOTS.iter().enumerate() {
        assert_eq!((slot.get)(&roles), 0x100000 + n as u32, "{}", slot.key);
    }
    let text = render("Ember", &roles, None);
    for slot in &SLOTS {
        assert!(
            text.contains(&format!("{} = \"#", slot.key)),
            "{}",
            slot.key
        );
    }
}

#[test]
fn what_is_rendered_is_read_back_as_it_was() {
    for roles in [
        NIGHT_ROLES,
        Roles {
            light: true,
            ..Roles::DAWN_LIGHT
        },
    ] {
        let parsed = theme_files::parse(&render("Ember", &roles, None)).unwrap();
        assert_eq!(parsed.roles, roles);
        assert_eq!(parsed.name, "Ember");
    }
    let odd = "Say \"hi\" \\ é 日本";
    let parsed = theme_files::parse(&render(odd, &NIGHT_ROLES, Some("sky.png"))).unwrap();
    assert_eq!(parsed.name, odd);
    assert_eq!(parsed.picture.as_deref(), Some("sky.png"));
}

#[test]
fn typed_colours_are_read_as_people_write_them() {
    assert_eq!(parse_hex("#C2552D"), Some(0xc2552d));
    assert_eq!(parse_hex("  c2552d "), Some(0xc2552d));
    assert_eq!(parse_hex("#fa0"), Some(0xffaa00));
    for bad in ["", "#", "#12345", "#1234567", "#gg0000", "red", "#12 456"] {
        assert_eq!(parse_hex(bad), None, "{bad:?}");
    }
}

#[test]
fn a_folder_is_named_from_the_name_and_never_twice() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    assert_eq!(slug_for(root, "Ember Glow!"), "ember-glow");
    assert_eq!(slug_for(root, "日本語"), "theme");
    assert_eq!(slug_for(root, "con"), "theme");
    assert_eq!(slug_for(root, "My Colours"), "my-colours");
    assert_eq!(slug_for(root, &"x".repeat(80)).len(), 40);
    std::fs::create_dir(root.join("ember")).unwrap();
    std::fs::create_dir(root.join("ember-2")).unwrap();
    assert_eq!(slug_for(root, "Ember"), "ember-3");
}

#[test]
fn a_name_that_is_taken_gets_a_number() {
    let taken = vec!["Ember".to_string(), "Ember (2)".to_string()];
    assert_eq!(unique_name(&taken, "Ink"), "Ink");
    assert_eq!(unique_name(&taken, "ember"), "ember (3)");
    assert_eq!(unique_name(&taken, "  Ember "), "Ember (3)");
    assert!(unique_name(&taken, &"n".repeat(64)).chars().count() <= 64);
}

#[test]
fn saved_themes_are_listed_renamed_copied_and_removed() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let slug = save_new(root, "Ember", &NIGHT_ROLES).unwrap();
    assert_eq!(slug, "ember");
    // The same name again is Ember (2), in a folder of its own.
    assert_eq!(save_new(root, "Ember", &NIGHT_ROLES).unwrap(), "ember-2");
    assert_eq!(names(root), ["Ember", "Ember (2)"]);

    rename(root, "ember", "  Cinder ").unwrap();
    assert_eq!(names(root), ["Cinder", "Ember (2)"]);
    let copy = duplicate(root, "ember").unwrap();
    assert_eq!(copy, "cinder-2");
    // Listed by folder.
    assert_eq!(names(root), ["Cinder (2)", "Cinder", "Ember (2)"]);

    let taken = take(root, &["ember-2".to_string(), "nothing".to_string()]);
    assert_eq!(
        taken,
        [Custom {
            name: "Ember (2)".into(),
            roles: NIGHT_ROLES
        }]
    );

    remove(root, "ember").unwrap();
    assert!(!root.join("ember").exists());
    assert_eq!(names(root), ["Cinder (2)", "Ember (2)"]);
    assert!(remove(root, "ember").is_err(), "gone is gone");
}

#[test]
fn a_themes_picture_stays_through_a_rename_and_goes_with_the_theme() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let slug = save_new(root, "Ember", &NIGHT_ROLES).unwrap();
    let picture = root.join(&slug).join("sky.png");
    image::RgbaImage::from_pixel(64, 64, image::Rgba([1, 2, 3, 255]))
        .save(&picture)
        .unwrap();
    write(root, &slug, "Ember", &NIGHT_ROLES, Some("sky.png")).unwrap();
    assert_eq!(saved(root)[0].picture.as_deref(), Some(picture.as_path()));
    rename(root, &slug, "Cinder").unwrap();
    assert_eq!(saved(root)[0].picture.as_deref(), Some(picture.as_path()));
    // A copy has the colours, not the picture.
    let copy = duplicate(root, &slug).unwrap();
    assert!(saved(root)
        .iter()
        .any(|l| l.slug == copy && l.picture.is_none()));
    remove(root, &slug).unwrap();
    assert!(!picture.exists() && !root.join(&slug).exists());
}

#[test]
fn a_theme_the_launcher_would_refuse_is_never_written() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    // Colours are never refused, however they read: only a bad name or a path is.
    assert!(save_new(root, "  ", &NIGHT_ROLES).is_err());
    assert!(write(root, "long", &"n".repeat(70), &NIGHT_ROLES, None).is_err());
    assert!(theme_files::scan(root).is_empty());
    assert!(write(root, "../out", "x", &NIGHT_ROLES, None).is_err());
}

#[cfg(unix)]
#[test]
fn nothing_is_written_through_a_link() {
    let root = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), root.path().join("ember")).unwrap();
    assert!(write(root.path(), "ember", "Ember", &NIGHT_ROLES, None).is_err());
    assert!(std::fs::read_dir(elsewhere.path())
        .unwrap()
        .next()
        .is_none());
}

#[test]
fn imported_themes_are_added_under_names_that_are_free() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let incoming = vec![
        Custom {
            name: "Ember".into(),
            roles: NIGHT_ROLES,
        },
        Custom {
            name: "Ember".into(),
            roles: Roles {
                accent: 0x445566,
                ..NIGHT_ROLES
            },
        },
    ];
    assert_eq!(add_all(root, &incoming).unwrap(), 2);
    assert_eq!(add_all(root, &incoming).unwrap(), 2);
    assert_eq!(
        names(root),
        ["Ember", "Ember (2)", "Ember (3)", "Ember (4)"]
    );
}

#[test]
fn the_colours_painted_are_the_editors_start() {
    let dark = current(&Settings::parse("{}"));
    assert_eq!(dark, roles_of(&themes::DAWN_DARK));
    assert!(!dark.light);
    let light = current(&Settings::parse(r#"{"theme": "dawn_light"}"#));
    assert!(light.light);
    assert_eq!(light.card, themes::DAWN_LIGHT.card);
}

/// The folder the colour editor used to paint (`my-colours`, "Custom") is a theme of the
/// library now: listed, chosen and painted like any other, with its colours kept.
#[test]
fn an_old_working_copy_is_a_saved_theme_called_custom() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "my-colours", "Custom", &NIGHT_ROLES, None).unwrap();
    theme_files::set_loaded(theme_files::scan(root.path()));
    assert_eq!(names(root.path()), ["Custom"]);
    let mut settings = Settings::parse("{}");
    crate::appearance::select_theme(&mut settings, "local:my-colours").unwrap();
    assert_eq!(current(&settings), NIGHT_ROLES);
    // Another theme chosen, and back: nothing about it is special.
    crate::appearance::select_theme(&mut settings, "nord").unwrap();
    crate::appearance::select_theme(&mut settings, "local:my-colours").unwrap();
    assert_eq!(current(&settings), NIGHT_ROLES);
    rename(root.path(), "my-colours", "Mine").unwrap();
    assert_eq!(names(root.path()), ["Mine"]);
}

#[test]
fn a_copy_is_a_new_theme_named_after_what_it_was_made_from() {
    let root = tempfile::tempdir().unwrap();
    let copy = |source: &str| make_copy(root.path(), source, &NIGHT_ROLES).unwrap();
    assert_eq!(
        copy("Sunrise"),
        ("sunrise-copy".into(), "Sunrise copy".into())
    );
    assert_eq!(
        copy("Sunrise"),
        ("sunrise-copy-2".into(), "Sunrise copy 2".into())
    );
    assert_eq!(
        copy("sunrise"),
        ("sunrise-copy-3".into(), "sunrise copy 3".into())
    );
    // Copying a copy is a copy of it.
    assert_eq!(
        copy("Sunrise copy"),
        ("sunrise-copy-copy".into(), "Sunrise copy copy".into())
    );
    assert_eq!(
        names(root.path()),
        [
            "Sunrise copy",
            "Sunrise copy 2",
            "sunrise copy 3",
            "Sunrise copy copy"
        ]
    );
    let long = copy(&"x".repeat(100)).1;
    assert!(long.chars().count() <= NAME_MAX, "{long}");
    assert!(theme_files::parse(&render(&long, &NIGHT_ROLES, None)).is_ok());
}

#[test]
fn cancelling_puts_a_theme_back_as_it_was_with_its_picture() {
    let root = tempfile::tempdir().unwrap();
    let was = Custom {
        name: "Ember".into(),
        roles: NIGHT_ROLES,
    };
    std::fs::create_dir_all(root.path().join("ember")).unwrap();
    image::RgbaImage::new(64, 64)
        .save(root.path().join("ember/sky.png"))
        .unwrap();
    write(root.path(), "ember", "Ember", &NIGHT_ROLES, Some("sky.png")).unwrap();
    let edited = Roles {
        accent: 0xabcdef,
        ..NIGHT_ROLES
    };
    write(root.path(), "ember", "Ember 2", &edited, Some("sky.png")).unwrap();
    restore(root.path(), "ember", &was).unwrap();
    let back = find(root.path(), "ember").unwrap();
    assert_eq!((back.name.as_str(), back.roles), ("Ember", NIGHT_ROLES));
    assert!(back.picture.is_some(), "the picture is kept");
}

fn working_copy(root: &Path, name: &str) -> Settings {
    write(root, "my-colours", name, &NIGHT_ROLES, None).unwrap();
    Settings::parse(r#"{"theme": "local:my-colours"}"#)
}

#[test]
fn the_old_working_copy_becomes_a_theme_called_custom_once() {
    let root = tempfile::tempdir().unwrap();
    let mut settings = working_copy(root.path(), "Custom");
    let saves = std::cell::Cell::new(0);
    let save = |_: &Settings| {
        saves.set(saves.get() + 1);
        Ok(())
    };
    assert!(migrate_working_copy(root.path(), &mut settings, save).unwrap());
    assert_eq!(settings.theme.as_deref(), Some("local:custom"));
    assert!(!root.path().join("my-colours").exists());
    assert_eq!(names(root.path()), ["Custom"]);
    assert_eq!(find(root.path(), "custom").unwrap().roles, NIGHT_ROLES);
    // Run again, nothing is left to do and nothing is saved.
    assert!(!migrate_working_copy(root.path(), &mut settings, save).unwrap());
    assert_eq!(
        (saves.get(), settings.theme.as_deref()),
        (1, Some("local:custom"))
    );
}

#[test]
fn the_migrated_theme_takes_a_free_folder_and_keeps_its_accent() {
    let root = tempfile::tempdir().unwrap();
    save_new(root.path(), "Custom", &NIGHT_ROLES).unwrap();
    let mut settings = working_copy(root.path(), "My own name");
    settings
        .theme_accents
        .insert("local:my-colours".into(), crate::themes::Accent::VIOLET);
    assert!(migrate_working_copy(root.path(), &mut settings, |_| Ok(())).unwrap());
    assert_eq!(settings.theme.as_deref(), Some("local:custom-2"));
    assert_eq!(find(root.path(), "custom-2").unwrap().name, "Custom");
    assert!(settings.theme_accents.contains_key("local:custom-2"));
    assert!(!settings.theme_accents.contains_key("local:my-colours"));
}

#[test]
fn the_migration_leaves_other_settings_and_a_missing_folder_alone() {
    let root = tempfile::tempdir().unwrap();
    let mut other = Settings::parse(r#"{"theme": "nord"}"#);
    working_copy(root.path(), "Custom");
    assert!(!migrate_working_copy(root.path(), &mut other, |_| Ok(())).unwrap());
    assert!(root.path().join("my-colours").exists());
    let mut gone = Settings::parse(r#"{"theme": "local:my-colours"}"#);
    let empty = tempfile::tempdir().unwrap();
    assert!(!migrate_working_copy(empty.path(), &mut gone, |_| Ok(())).unwrap());
    assert_eq!(gone.theme.as_deref(), Some("local:my-colours"));
}

#[test]
fn a_migration_that_cannot_be_saved_puts_everything_back() {
    let root = tempfile::tempdir().unwrap();
    let mut settings = working_copy(root.path(), "Custom");
    let failed = migrate_working_copy(root.path(), &mut settings, |_| Err("disk full".into()));
    assert_eq!(failed, Err("disk full".to_string()));
    assert_eq!(settings.theme.as_deref(), Some("local:my-colours"));
    assert!(root.path().join("my-colours").exists());
    assert!(!root.path().join("custom").exists());
}
