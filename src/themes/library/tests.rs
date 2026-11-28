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
    assert_eq!(
        slug_for(root, "My Colours"),
        "my-colours-2",
        "the working copy's folder is not for the library"
    );
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
    // The working copy is a theme, but not one of the library's.
    write(root, WORKING, "Custom", &NIGHT_ROLES, None).unwrap();
    assert_eq!(saved(root).len(), 2);
    assert!(theme_files::scan(root).len() == 3);

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
    let grey = Roles {
        background: 0x777777,
        card: 0x777777,
        chrome: 0x777777,
        ..NIGHT_ROLES
    };
    assert!(save_new(root, "Mud", &grey).is_err());
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
fn the_colours_painted_are_the_editors_start_and_a_reset_goes_back_to_sunrise() {
    let dark = current(&Settings::parse("{}"));
    assert_eq!(dark, roles_of(&themes::DAWN_DARK));
    assert!(!dark.light);
    let light = current(&Settings::parse(r#"{"theme": "dawn_light"}"#));
    assert!(light.light);
    assert_eq!(light.card, themes::DAWN_LIGHT.card);

    let mut settings = Settings::parse(r#"{"theme": "nord"}"#);
    reset(&mut settings, true).unwrap();
    assert_eq!(settings.theme.as_deref(), Some("dawn_light"));
    reset(&mut settings, false).unwrap();
    assert_eq!(settings.theme, None);
}

#[test]
fn the_working_copy_can_be_chosen_and_is_then_what_is_painted() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), WORKING, "Custom", &NIGHT_ROLES, None).unwrap();
    theme_files::set_loaded(theme_files::scan(root.path()));
    assert_eq!(WORKING_ID, format!("{}{WORKING}", theme_files::PREFIX));
    let mut settings = Settings::parse("{}");
    crate::appearance::select_theme(&mut settings, WORKING_ID).unwrap();
    assert_eq!(current(&settings), NIGHT_ROLES);
}
