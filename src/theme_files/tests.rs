use super::*;

const TIDEPOOL: &str = r##"version = 1
name = "Tidepool"

[palette]
card = "#1A3135"
chrome = "#11262B"
text = "#F0F8F4"
muted = "#C3D7D2"
background = "#102328"
accent = "#8EDBCB"
"##;

fn theme(root: &Path, slug: &str, text: &str) -> PathBuf {
    let folder = root.join(slug);
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join(FILE), text).unwrap();
    folder
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::RgbaImage::from_pixel(w, h, image::Rgba([40, 90, 120, 255]))
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    bytes
}

fn why(text: &str) -> Reason {
    parse(text).unwrap_err()
}

#[test]
fn the_example_file_is_six_colours_and_a_name() {
    let parsed = parse(TIDEPOOL).unwrap();
    assert_eq!(parsed.name, "Tidepool");
    assert_eq!(parsed.picture, None);
    assert_eq!(
        parsed.roles,
        Roles {
            background: 0x102328,
            card: 0x1a3135,
            chrome: 0x11262b,
            text: 0xf0f8f4,
            muted: 0xc3d7d2,
            accent: 0x8edbcb,
            // Left out, the lines are the text's colour.
            line: 0xf0f8f4,
            ..Roles::DAWN
        }
    );
    // The published example is this file, with its picture line commented out.
    let example: String = include_str!("../../docs/themes/tidepool/theme.toml")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(|line| format!("{line}\n"))
        .collect();
    assert_eq!(example, TIDEPOOL);
}

#[test]
fn a_colour_left_out_is_dawns_and_one_is_enough() {
    let parsed =
        parse("version = 1\nname = \" Ember \"\n[palette]\naccent = \"#ff8800\"\n").unwrap();
    assert_eq!(parsed.name, "Ember");
    assert_eq!(
        parsed.roles,
        Roles {
            accent: 0xff8800,
            ..Roles::DAWN
        }
    );
    assert_eq!(
        why("version = 1\nname = \"x\"\n[palette]\n"),
        Reason::NoColours
    );
    assert_eq!(
        why("version = 1\nname = \"x\"\n"),
        Reason::Missing("[palette]")
    );
}

#[test]
fn only_version_one_with_a_plain_name_is_read() {
    let with = |head: &str| format!("{head}\n[palette]\naccent = \"#ff8800\"\n");
    assert_eq!(why(&with("name = \"x\"")), Reason::Missing("version"));
    assert_eq!(why(&with("version = 2\nname = \"x\"")), Reason::Version);
    assert_eq!(why(&with("version = \"1\"\nname = \"x\"")), Reason::Version);
    assert_eq!(why(&with("version = 1")), Reason::Missing("name"));
    assert_eq!(why(&with("version = 1\nname = 3")), Reason::Name);
    assert_eq!(why(&with("version = 1\nname = \"  \"")), Reason::Name);
    assert_eq!(
        why(&with(&format!(
            "version = 1\nname = \"{}\"",
            "n".repeat(65)
        ))),
        Reason::Name
    );
    assert!(parse(&with(&format!(
        "version = 1\nname = \"{}\"",
        "é".repeat(64)
    )))
    .is_ok());
    // Two lines, a control character, or a character that turns the text around.
    for name in ["a\\nb", "a\\u0007b", "a\\u202Eb", "a\\u2066b"] {
        assert_eq!(
            why(&with(&format!("version = 1\nname = \"{name}\""))),
            Reason::Name,
            "{name}"
        );
    }
}

#[test]
fn a_key_the_launcher_does_not_know_refuses_the_file() {
    let base = "version = 1\nname = \"x\"\n";
    let unknown = |text: String, key: &str| assert_eq!(why(&text), Reason::Unknown(key.into()));
    unknown(
        format!("{base}opacity = 0.5\n[palette]\naccent = \"#ff8800\"\n"),
        "opacity",
    );
    unknown(
        format!("{base}[palette]\naccent = \"#ff8800\"\n[glass]\nblur = 4\n"),
        "glass",
    );
    // Play's label is white in every theme: neither spelling is a key.
    unknown(
        format!("{base}[palette]\naccent = \"#ff8800\"\naccent_on = \"#000000\"\n"),
        "palette.accent_on",
    );
    unknown(
        format!("{base}[palette]\naccent = \"#ff8800\"\naccent-on = \"#000000\"\n"),
        "palette.accent-on",
    );
    unknown(
        format!("{base}[palette]\naccent = \"#ff8800\"\ntint = \"#000000\"\n"),
        "palette.tint",
    );
    // A key written twice, or text that isn't TOML, is the reader's error.
    assert!(matches!(
        why(&format!(
            "{base}name = \"y\"\n[palette]\naccent = \"#ff8800\"\n"
        )),
        Reason::Toml(_)
    ));
    assert!(matches!(why("version = "), Reason::Toml(_)));
}

#[test]
fn a_colour_is_exactly_a_hash_and_six_hex_digits() {
    let with = |value: &str| format!("version = 1\nname = \"x\"\n[palette]\naccent = {value}\n");
    assert_eq!(parse(&with("\"#aBcDeF\"")).unwrap().roles.accent, 0xabcdef);
    for value in [
        "\"abcdef\"",
        "\"#abc\"",
        "\"#abcdefff\"",
        "\"#abcdeg\"",
        "\"teal\"",
        "\" #abcdef\"",
        "0xabcdef",
        "\"#+bcdef\"",
    ] {
        assert_eq!(
            why(&with(value)),
            Reason::Colour("palette.accent".into()),
            "{value}"
        );
    }
}

#[test]
fn colours_that_do_not_read_on_each_other_are_taken_as_they_are() {
    // A white card over a black page: the player's choice, painted as it is.
    let text =
        "version = 1\nname = \"x\"\n[palette]\ncard = \"#ffffff\"\nbackground = \"#000000\"\n";
    let parsed = parse(text).unwrap();
    assert_eq!(parsed.roles.card, 0xffffff);
    assert_eq!(parsed.roles.background, 0x000000);
    // Light all through is as good as dark all through.
    let light = parse("version = 1\nname = \"x\"\n[palette]\ncard = \"#ffffff\"\nbackground = \"#f2f2f2\"\nchrome = \"#e8e8e8\"\ntext = \"#111111\"\nmuted = \"#444444\"\n").unwrap();
    assert!(!light.roles.night().dark);
    assert!(parse(TIDEPOOL).unwrap().roles.night().dark);
}

#[test]
fn a_picture_is_one_name_in_the_themes_own_folder() {
    let with = |name: &str| {
        format!("version = 1\nname = \"x\"\nbackground = {name}\n[palette]\naccent = \"#ff8800\"\n")
    };
    assert_eq!(
        parse(&with("\"shore.webp\"")).unwrap().picture.as_deref(),
        Some("shore.webp")
    );
    assert_eq!(why(&with("7")), Reason::NotString("background".into()));
    for name in [
        "",
        ".",
        "..",
        "../shore.png",
        "a/shore.png",
        "a\\\\shore.png",
        "/etc/shore.png",
        "C:\\\\shore.png",
        "C:shore.png",
        "\\\\\\\\host\\\\share\\\\shore.png",
        "https://example.org/shore.png",
        "file:shore.png",
        "shore.png:stream",
        "NUL",
        "con.png",
        "LPT1.webp",
        "shore.png.",
        "shore.png ",
        "sho\\u0000re.png",
    ] {
        assert_eq!(
            why(&with(&format!("\"{name}\""))),
            Reason::PictureName,
            "{name}"
        );
    }
}

#[test]
fn a_folders_name_is_a_plain_slug() {
    for good in ["tidepool", "a", "9-lives", "my-theme-2", &"a".repeat(48)] {
        assert!(slug_ok(good), "{good}");
    }
    for bad in [
        "",
        "-lead",
        "Tidepool",
        "tide pool",
        "tide_pool",
        "tide.pool",
        "nul",
        "com1",
        "thème",
        &"a".repeat(49),
    ] {
        assert!(!slug_ok(bad), "{bad}");
    }
}

#[test]
fn a_bad_theme_is_listed_beside_the_good_ones_with_its_reason() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(scan(&root.path().join("absent")), []);
    theme(root.path(), "tidepool", TIDEPOOL);
    theme(
        root.path(),
        "broken",
        "version = 1\nname = \"x\"\n[palette]\naccent = \"red\"\n",
    );
    theme(root.path(), "Loud", TIDEPOOL);
    fs::create_dir_all(root.path().join("empty")).unwrap();
    theme(
        root.path(),
        "long",
        &format!("{TIDEPOOL}#{}\n", "x".repeat(MAX_TOML as usize)),
    );
    fs::write(
        theme(root.path(), "binary", "").join(FILE),
        [0xff, 0xfe, 0x00],
    )
    .unwrap();
    // A file beside the folders is not a theme, and is not listed.
    fs::write(root.path().join("notes.txt"), "hello").unwrap();
    let found = scan(root.path());
    let bad = |folder: &str, reason: Reason| Entry::Bad {
        folder: folder.into(),
        reason,
    };
    assert_eq!(
        found,
        [
            bad("Loud", Reason::Slug),
            bad("binary", Reason::NotText),
            bad("broken", Reason::Colour("palette.accent".into())),
            bad("empty", Reason::NoFile),
            bad("long", Reason::TooLong),
            Entry::Good(Local {
                slug: "tidepool".into(),
                name: "Tidepool".into(),
                roles: parse(TIDEPOOL).unwrap().roles,
                picture: None,
            }),
        ]
    );
    // Every reason has words, and nothing rewrote a file.
    for entry in &found {
        if let Entry::Bad { reason, .. } = entry {
            assert!(!reason.text().is_empty());
        }
    }
    assert_eq!(
        fs::read_to_string(root.path().join("broken").join(FILE)).unwrap(),
        "version = 1\nname = \"x\"\n[palette]\naccent = \"red\"\n"
    );
    // Repaired, the next read has it.
    theme(root.path(), "broken", TIDEPOOL);
    assert!(scan(root.path())
        .iter()
        .any(|e| matches!(e, Entry::Good(l) if l.slug == "broken")));
}

#[test]
fn webp_backgrounds_must_be_still_regardless_of_filename() {
    // Two valid lossless 64×64 frames, solid red then blue.
    let animated = include_bytes!("fixtures/two-colours.webp");
    let reason = Reason::AnimatedWebP("shore.png".into());
    assert_eq!(kind(animated, "shore.png"), Err(reason.clone()));
    assert_eq!(still(animated, "shore.png"), Err(reason));

    let mut bytes = std::io::Cursor::new(Vec::new());
    image::RgbImage::new(64, 64)
        .write_to(&mut bytes, image::ImageFormat::WebP)
        .unwrap();
    assert_eq!(
        still(bytes.get_ref(), "shore.png").unwrap().dimensions(),
        (64, 64)
    );
}

#[test]
fn a_picture_is_checked_by_what_it_holds_and_how_big_it_is() {
    let root = tempfile::tempdir().unwrap();
    let with = TIDEPOOL.replace(
        "name = \"Tidepool\"",
        "name = \"Tidepool\"\nbackground = \"shore.png\"",
    );
    let folder = theme(root.path(), "tidepool", &with);
    let read = || match scan(root.path()).remove(0) {
        Entry::Good(local) => Ok(local),
        Entry::Bad { reason, .. } => Err(reason),
    };
    assert_eq!(read(), Err(Reason::PictureMissing("shore.png".into())));
    fs::write(folder.join("shore.png"), png(320, 200)).unwrap();
    let local = read().unwrap();
    assert_eq!(local.picture, Some(folder.join("shore.png")));
    assert_eq!(
        picture_in(root.path(), &folder.join("shore.png"))
            .unwrap()
            .dimensions(),
        (320, 200)
    );
    // Its name says PNG; what it holds decides.
    fs::write(
        folder.join("shore.png"),
        b"<svg xmlns='http://www.w3.org/2000/svg'/>",
    )
    .unwrap();
    assert_eq!(read(), Err(Reason::PictureKind("shore.png".into())));
    let mut gif = Vec::new();
    image::RgbaImage::new(80, 80)
        .write_to(&mut std::io::Cursor::new(&mut gif), image::ImageFormat::Gif)
        .unwrap();
    fs::write(folder.join("shore.png"), gif).unwrap();
    assert_eq!(read(), Err(Reason::PictureKind("shore.png".into())));
    fs::write(folder.join("shore.png"), png(63, 200)).unwrap();
    assert_eq!(
        read(),
        Err(Reason::PictureSize("shore.png".into(), 63, 200))
    );
    fs::write(folder.join("shore.png"), png(8193, 64)).unwrap();
    assert_eq!(
        read(),
        Err(Reason::PictureSize("shore.png".into(), 8193, 64))
    );
    // Within each side's limit, but over 16 megapixels.
    fs::write(folder.join("shore.png"), png(4100, 4000)).unwrap();
    assert_eq!(
        read(),
        Err(Reason::PictureSize("shore.png".into(), 4100, 4000))
    );
    // A truncated picture is refused when it is decoded, without a panic.
    let cut = png(320, 200);
    fs::write(folder.join("shore.png"), &cut[..cut.len() / 2]).unwrap();
    assert!(picture_in(root.path(), &folder.join("shore.png")).is_err());
    // A wide one is scaled down for the window.
    fs::write(folder.join("shore.png"), png(5120, 1000)).unwrap();
    assert_eq!(
        picture_in(root.path(), &folder.join("shore.png"))
            .unwrap()
            .width(),
        2560
    );
}

#[cfg(unix)]
#[test]
fn discovery_refuses_non_regular_theme_files_and_pictures() {
    use std::os::unix::ffi::OsStrExt as _;
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a");
    fs::create_dir(&a).unwrap();
    let b = theme(
        root.path(),
        "b",
        &TIDEPOOL.replace("[palette]", "background = \"shore.png\"\n[palette]"),
    );
    for path in [a.join(FILE), b.join("shore.png")] {
        let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: path is a live, nul-terminated pathname in this test's private directory.
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    }
    theme(root.path(), "c", TIDEPOOL);
    let found = scan(root.path());
    assert!(matches!(&found[..], [
        Entry::Bad { reason: Reason::NotRegular("theme.toml"), .. },
        Entry::Bad { reason: Reason::NotRegular("picture"), .. },
        Entry::Good(local),
    ] if local.slug == "c"));
}

#[cfg(unix)]
#[test]
fn a_linked_folder_file_or_picture_is_refused() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let real = theme(outside.path(), "real", TIDEPOOL);
    fs::write(outside.path().join("secret.png"), png(100, 100)).unwrap();
    symlink(&real, root.path().join("linked")).unwrap();
    let folder = root.path().join("file");
    fs::create_dir_all(&folder).unwrap();
    symlink(real.join(FILE), folder.join(FILE)).unwrap();
    let with = TIDEPOOL.replace(
        "name = \"Tidepool\"",
        "name = \"Tidepool\"\nbackground = \"shore.png\"",
    );
    let pictured = theme(root.path(), "picture", &with);
    symlink(
        outside.path().join("secret.png"),
        pictured.join("shore.png"),
    )
    .unwrap();
    let reasons: Vec<_> = scan(root.path())
        .into_iter()
        .map(|e| match e {
            Entry::Bad { folder, reason } => (folder, reason),
            Entry::Good(local) => panic!("{local:?}"),
        })
        .collect();
    assert_eq!(
        reasons,
        [
            ("file".to_string(), Reason::Linked("theme.toml")),
            ("linked".to_string(), Reason::Linked("folder")),
            ("picture".to_string(), Reason::Linked("picture")),
        ]
    );
}

/// A link or junction where the themes folder should be is listed as that, and nothing behind
/// it is read; a theme's folder swapped for a link after the themes were read leads nowhere.
#[cfg(unix)]
#[test]
fn a_linked_themes_folder_or_a_folder_swapped_for_a_link_is_never_read_through() {
    use std::os::unix::fs::symlink;
    let with = TIDEPOOL.replace(
        "name = \"Tidepool\"",
        "name = \"Tidepool\"\nbackground = \"shore.png\"",
    );
    // Someone else's themes, with a picture.
    let outside = tempfile::tempdir().unwrap();
    let theirs = theme(outside.path(), "tidepool", &with);
    fs::write(theirs.join("shore.png"), png(320, 200)).unwrap();

    // The whole themes folder is a link to them.
    let data = tempfile::tempdir().unwrap();
    let root = data.path().join("themes");
    symlink(outside.path(), &root).unwrap();
    assert!(root_linked(&root));
    assert_eq!(
        scan(&root),
        [Entry::Bad {
            folder: "themes".into(),
            reason: Reason::Linked("folder"),
        }]
    );
    assert_eq!(
        picture_in(&root, &root.join("tidepool").join("shore.png")),
        Err(Reason::Linked("folder"))
    );

    // A real themes folder, read while its theme was real.
    let data = tempfile::tempdir().unwrap();
    let root = data.path().join("themes");
    let mine = theme(&root, "tidepool", &with);
    fs::write(mine.join("shore.png"), png(320, 200)).unwrap();
    assert!(!root_linked(&root));
    let Entry::Good(local) = scan(&root).remove(0) else {
        panic!("the theme is good while it is real");
    };
    let kept = local.picture.unwrap();
    assert!(picture_in(&root, &kept).is_ok());
    // Then the theme's folder is replaced with a link: the saved path still names a picture,
    // but Home's read stops at the link.
    fs::remove_dir_all(&mine).unwrap();
    symlink(&theirs, &mine).unwrap();
    assert!(kept.is_file(), "the path alone would be followed");
    assert_eq!(picture_in(&root, &kept), Err(Reason::Linked("folder")));
    // And a path that isn't a theme's own picture is no picture at all.
    assert_eq!(
        picture_in(&root, &outside.path().join("tidepool").join("shore.png")),
        Err(Reason::PictureMissing("shore.png".into()))
    );
}

/// A folder's name is kept as it was found, so a refused choice can name it; what goes to a
/// terminal has its control characters written out, not sent.
#[cfg(unix)]
#[test]
fn a_folder_named_with_control_characters_reaches_a_terminal_as_text() {
    let root = tempfile::tempdir().unwrap();
    let evil = "evil\u{1b}[2J\u{7}\r\u{202e}";
    fs::create_dir_all(root.path().join(evil)).unwrap();
    theme(root.path(), "tidepool", TIDEPOOL);
    let found = scan(root.path());
    assert!(matches!(&found[0], Entry::Bad { folder, reason: Reason::Slug } if folder == evil));
    set_loaded(found);
    let mut settings = crate::backend::Settings::parse("{}");
    let said = crate::appearance::select_theme(&mut settings, "local:nope").unwrap_err();
    assert!(said.contains(evil), "the message names the folder as found");
    let shown = for_terminal(&said);
    assert!(
        !shown.chars().any(|c| c.is_control() || turns_text(c)),
        "{shown:?}"
    );
    assert!(
        shown.contains("evil\\u{1b}[2J\\u{7}\\u{d}\\u{202e}"),
        "{shown}"
    );
    assert!(shown.contains("local:tidepool"), "the rest reads as before");
    // Line breaks in a message stay line breaks.
    assert_eq!(for_terminal("one\ntwo\ttab"), "one\ntwo\\u{9}tab");
}

#[test]
fn lines_success_danger_and_the_look_are_read_and_default_sensibly() {
    let full = parse(
        "version = 1\nname = \"x\"\nlook = \"light\"\n[palette]\ntext = \"#111111\"\nline = \"#abcdef\"\nsuccess = \"#00aa00\"\ndanger = \"#cc0000\"\n",
    )
    .unwrap();
    assert_eq!(full.roles.line, 0xabcdef);
    assert_eq!(full.roles.success, 0x00aa00);
    assert_eq!(full.roles.danger, 0xcc0000);
    assert!(full.roles.light);
    assert_eq!(full.roles.night().stroke, 0xabcdef);
    assert_eq!(full.roles.night().danger, 0xcc0000);
    // Said nothing of: the lines follow the text, the look is the night.
    let plain = parse("version = 1\nname = \"x\"\n[palette]\ntext = \"#f0f8f4\"\n").unwrap();
    assert_eq!(plain.roles.line, 0xf0f8f4);
    assert!(!plain.roles.light);
    assert_eq!(plain.roles.success, Roles::DAWN.success);
    // The look is the day or the night, and only those.
    let look = |v: &str| {
        why(&format!(
            "version = 1\nname = \"x\"\nlook = {v}\n[palette]\ntext = \"#111111\"\n"
        ))
    };
    assert_eq!(look("\"dusk\""), Reason::NotString("look".into()));
    assert_eq!(look("1"), Reason::NotString("look".into()));
}

#[test]
fn sunrises_day_is_a_light_palette() {
    assert!(!Roles::DAWN_LIGHT.night().dark);
    assert_eq!(Roles::DAWN_LIGHT.night().page, DAWN_LIGHT.page);
}
