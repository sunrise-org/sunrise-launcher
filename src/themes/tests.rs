use super::*;
use sunrise_launcher_theme::night::{DAWN, NIGHT};

fn blend(top: u32, under: u32, alpha: f32) -> u32 {
    let channel = |shift: u32| {
        let (a, b) = ((top >> shift & 0xff) as f32, (under >> shift & 0xff) as f32);
        ((a * alpha + b * (1. - alpha)).round() as u32) << shift
    };
    channel(16) | channel(8) | channel(0)
}

#[test]
fn every_accent_reads_as_well_as_the_violet() {
    for base in [
        NIGHT.blocks(),
        DAWN.blocks(),
        NORD_DARK.blocks(),
        TAIGA_NIGHT.blocks(),
    ] {
        for accent in Accent::ALL
            .into_iter()
            .chain((0..720).map(|i| Accent::custom(i as f32 * 0.5)))
        {
            let p = accent.apply(base);
            let what = format!(
                "{} {}",
                accent.name(),
                if p.dark { "dark" } else { "light" }
            );
            let on_face = contrast(p.on_face, p.face);
            assert!(on_face >= 4.5, "{what}: text on the face {on_face:.2}");
            for g in [p.page, p.raised, p.button, p.panel, p.well] {
                let ring = contrast(p.accent, g);
                assert!(ring >= 3.0, "{what}: accent on {g:06x} {ring:.2}");
            }
            // Text over a selected suggestion (the launcher's theme::SELECTION).
            let selected = contrast(p.ink, blend(p.accent, p.panel, 0.22));
            assert!(selected >= 4.5, "{what}: text on a selection {selected:.2}");
            let (_, _, face) = hsl(p.face);
            assert!(
                hsl(p.face_bevel).2 > face && hsl(p.face_lip).2 < face,
                "{what}"
            );
            if let Some(want) = accent.hue() {
                let (h, _, _) = hsl(p.face);
                let off = ((h - want).rem_euclid(360.)).min((want - h).rem_euclid(360.));
                assert!(off < 3., "{what}: face hue {h:.0}, not {want}");
            }
        }
        let violet = Accent::VIOLET.apply(base);
        assert_eq!((violet.accent, violet.face), (base.accent, base.face));
    }
}

#[test]
fn colours_go_to_hsl_and_back() {
    for c in [
        0x5b3fd6, 0x9a86ff, 0x000000, 0xffffff, 0x45c27a, 0xf0b84f, 0x123456,
    ] {
        let (h, s, l) = hsl(c);
        assert_eq!(from_hsl(h, s, l), c, "{c:06x}");
    }
    assert_eq!(Accent::parse("Teal"), Some(Accent::TEAL));
    assert_eq!(Accent::parse("purple"), Some(Accent::VIOLET));
    assert_eq!(Accent::parse("chartreuse"), None);
    assert_eq!(serde_json::to_string(&Accent::AMBER).unwrap(), "\"amber\"");
}

#[test]
fn every_theme_palette_reads_over_its_sky() {
    use sunrise_launcher_theme::night::{over, UNCARDED_STOPS};

    let dark = THEMES.iter().filter_map(|t| t.palette.map(|p| (t.id, *p)));
    let light = THEMES
        .iter()
        .filter_map(|t| t.light_palette.map(|p| (t.id, *p)));
    for (id, p) in dark.chain(light) {
        let look = format!("{id} {}", if p.dark { "dark" } else { "light" });
        let texts = [
            ("ink", p.ink),
            ("ink_soft", p.ink_soft),
            ("muted", p.muted),
            ("accent_hi", p.accent_hi),
            ("success", p.success),
            ("warn", p.warn),
            ("danger", p.danger),
        ];
        for (i, &sky) in p.sky.iter().enumerate() {
            for (plate, top, alpha) in [
                ("card", p.card, p.card_alpha),
                ("card_hi", p.card_hi, p.card_hi_alpha),
            ] {
                let bg = over(top, alpha, sky);
                for (tn, fg) in texts {
                    let ratio = contrast(fg, bg);
                    assert!(
                        ratio >= 4.5,
                        "{look} {tn} on {plate} over sky {i}: {ratio:.2}"
                    );
                }
            }
        }
        for (i, &sky) in p.sky[..UNCARDED_STOPS].iter().enumerate() {
            for (tn, fg) in &texts[..4] {
                let ratio = contrast(*fg, sky);
                assert!(ratio >= 4.5, "{look} {tn} on sky {i}: {ratio:.2}");
            }
        }
        for (bn, bg) in [("page", p.page), ("side", p.side)] {
            for (tn, fg) in &texts[..4] {
                let ratio = contrast(*fg, bg);
                assert!(ratio >= 4.5, "{look} {tn} on {bn}: {ratio:.2}");
            }
        }
        let label = contrast(p.on_accent, p.accent);
        assert!(label >= 4.5, "{look} the main action's label: {label:.2}");
    }
}

#[test]
fn a_background_is_a_picture_kept_once() {
    let root = std::env::temp_dir().join(format!("sunrise-background-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let picture = |name: &str, w: u32, h: u32| {
        let path = root.join(name);
        image::RgbImage::new(w, h).save(&path).unwrap();
        path
    };
    let dir = root.join("data");
    let png = picture("dusk.png", 160, 90);
    let first = keep_background(&dir, &png).unwrap();
    assert!(
        first.starts_with("background-") && first.ends_with(".png"),
        "{first}"
    );
    let jpg = picture("noon.jpg", 160, 90);
    let second = keep_background(&dir, &jpg).unwrap();
    assert!(second.ends_with(".jpg") && second != first, "{second}");
    assert!(dir.join(&first).exists(), "the older one stays");
    assert_eq!(background(&dir, Some(&second)), Some(dir.join(&second)));
    assert_eq!(background(&dir, Some("../noon.jpg")), None);
    assert_eq!(background(&dir, None), None);

    let tiny = picture("tiny.png", 16, 16);
    assert!(keep_background(&dir, &tiny).unwrap_err().contains("16×16"));
    let text = root.join("notes.txt");
    std::fs::write(&text, "hi").unwrap();
    assert!(keep_background(&dir, &text)
        .unwrap_err()
        .contains("PNG, JPEG or WebP"));
    let fake = root.join("fake.png");
    std::fs::write(&fake, "not a picture").unwrap();
    assert!(keep_background(&dir, &fake)
        .unwrap_err()
        .contains("PNG, JPEG or WebP"));
    assert!(
        dir.join(&second).exists(),
        "a refused file keeps the old one"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_background_is_drawn_no_larger_than_the_gpu_holds_and_a_huge_one_is_refused() {
    let root = std::env::temp_dir().join(format!("sunrise-background-big-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dir = root.join("data");
    std::fs::create_dir_all(&dir).unwrap();
    // Wider than any texture atlas, as a saved choice from before sizes were checked: past
    // the pixel limits, so it is an error and Home shows what comes after it.
    let wide = dir.join("background-0123456789ab.png");
    image::GrayImage::new(20_000, 70).save(&wide).unwrap();
    assert!(background_picture(&dir, "background-0123456789ab.png").is_err());
    // A picture within the limits is still never handed over larger than the GPU holds.
    image::GrayImage::new(8000, 70)
        .save(dir.join("background-wide.png"))
        .unwrap();
    for limit in [1024, 2048, 16384] {
        let drawn = crate::theme_files::fitted(
            background_picture(&dir, "background-wide.png").unwrap(),
            limit,
        );
        assert_eq!(drawn.width(), limit.min(2560), "at limit {limit}");
        assert!(drawn.height() >= 1);
    }
    // Not a picture: an error, for the caller to show its own scenery instead.
    std::fs::write(dir.join("background-broken.png"), "no").unwrap();
    assert!(background_picture(&dir, "background-broken.png").is_err());
    assert!(background_picture(&dir, "background-missing.png").is_err());

    // The importer measures pixels, not only bytes: a small file claiming 30000 × 30000.
    let claimed = root.join("claimed.png");
    image::GrayImage::new(64, 64).save(&claimed).unwrap();
    let mut bytes = std::fs::read(&claimed).unwrap();
    // IHDR's data starts after the 8-byte signature, the chunk's length and its name.
    bytes[16..20].copy_from_slice(&30_000u32.to_be_bytes());
    bytes[20..24].copy_from_slice(&30_000u32.to_be_bytes());
    let mut crc = flate2::Crc::new();
    crc.update(&bytes[12..29]);
    bytes[29..33].copy_from_slice(&crc.sum().to_be_bytes());
    std::fs::write(&claimed, bytes).unwrap();
    assert!(keep_background(&dir, &claimed)
        .unwrap_err()
        .contains("30000×30000"));
    assert!(wide.exists(), "a refused file keeps the old one");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_picked_hue_is_any_degree_and_survives_settings_json() {
    assert_eq!(Accent::parse("blue"), Some(Accent::BLUE));
    assert_eq!(Accent::parse("217"), Some(Accent::custom(217.)));
    assert_eq!(Accent::custom(370.).name(), "10");
    assert_eq!(Accent::custom(212.).name(), "blue");
    assert_eq!(Accent::VIOLET.name(), "violet");
    let saved = serde_json::to_string(&Accent::custom(217.5)).unwrap();
    assert_eq!(saved, "\"217.5\"");
    assert_eq!(
        serde_json::from_str::<Accent>(&saved).unwrap(),
        Accent::custom(217.5)
    );
}

#[test]
fn a_typed_colour_is_worn_as_is_and_survives_settings_json() {
    // The mockup's swatches are the named accents.
    assert_eq!(Accent::parse("#2563EB"), Some(Accent::BLUE));
    assert_eq!(Accent::BLUE.hex(), "#2563eb");
    assert_eq!(Accent::VIOLET.hex(), "#4f60e6");
    // Any other colour keeps its exact value for the glass look, and its hue for the light one.
    let typed = Accent::parse("#ff8800").unwrap();
    assert_eq!(typed.rgb(), 0xff8800);
    assert!((typed.hue().unwrap() - 32.).abs() < 0.5);
    let saved = serde_json::to_string(&typed).unwrap();
    assert_eq!(saved, "\"#ff8800\"");
    assert_eq!(serde_json::from_str::<Accent>(&saved).unwrap(), typed);
    // Half-typed or not hex: no accent.
    for wrong in ["#ff88", "#ff88001", "#gg8800", "#"] {
        assert_eq!(Accent::parse(wrong), None, "{wrong}");
    }
    // A picked hue wears the default accent's saturation and lightness.
    let (h, s, l) = hsl(Accent::custom(120.).rgb());
    let (_, vs, vl) = hsl(Accent::VIOLET.rgb());
    assert!((h - 120.).abs() < 1. && (s - vs).abs() < 0.02 && (l - vl).abs() < 0.02);
}

#[test]
fn each_theme_brings_back_its_own_accent() {
    let mut kept = std::collections::BTreeMap::new();
    // No theme saved is Dawn; Sunrise's own sky is picked by name.
    assert_eq!(starting_accent(Some(TWILIGHT), &kept), Accent::VIOLET);
    assert_eq!(starting_accent(None, &kept).rgb(), 0xc2552d);
    assert_eq!(starting_accent(Some("dawn"), &kept).rgb(), 0xc2552d);
    kept.insert("dawn".to_string(), Accent::TEAL);
    kept.insert(TWILIGHT.to_string(), Accent::PINK);
    assert_eq!(starting_accent(Some("dawn"), &kept), Accent::TEAL);
    assert_eq!(starting_accent(None, &kept), Accent::TEAL);
    assert_eq!(starting_accent(Some(TWILIGHT), &kept), Accent::PINK);
    // A theme this launcher doesn't know is Sunrise's own.
    assert_eq!(accent_key(Some("from_the_future")), TWILIGHT);
    assert_eq!(
        starting_accent(Some("from_the_future"), &kept),
        Accent::PINK
    );
    // The older themes keep their ids.
    assert!(theme(Some("first_light")).is_some() && theme(Some("nord")).is_some());
}

/// Choosing a picture never writes through what already has its name: a link there is refused
/// and its target untouched, and the same picture chosen again keeps the copy as it is.
#[cfg(unix)]
#[test]
fn a_kept_picture_is_never_written_through_a_link_or_over_the_copy_in_use() {
    use std::os::unix::fs::MetadataExt as _;
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("data");
    let source = root.path().join("dusk.png");
    image::RgbImage::new(160, 90).save(&source).unwrap();

    let name = keep_background(&dir, &source).unwrap();
    let kept = dir.join(&name);
    let first = std::fs::metadata(&kept).unwrap();
    // Chosen again: the copy in use is the same file, never emptied on the way.
    assert_eq!(keep_background(&dir, &source).unwrap(), name);
    let again = std::fs::metadata(&kept).unwrap();
    assert_eq!((first.ino(), first.len()), (again.ino(), again.len()));
    assert_eq!(
        std::fs::read(&kept).unwrap(),
        std::fs::read(&source).unwrap()
    );

    // A link under the name leads to a file of the player's: refused, and that file is whole.
    let theirs = root.path().join("diary.txt");
    std::fs::write(&theirs, "mine").unwrap();
    std::fs::remove_file(&kept).unwrap();
    std::os::unix::fs::symlink(&theirs, &kept).unwrap();
    assert!(keep_background(&dir, &source).is_err());
    assert_eq!(std::fs::read_to_string(&theirs).unwrap(), "mine");
    // So is a file with other bytes under the name.
    std::fs::remove_file(&kept).unwrap();
    std::fs::write(&kept, "not this picture").unwrap();
    assert!(keep_background(&dir, &source).is_err());
    assert_eq!(std::fs::read_to_string(&kept).unwrap(), "not this picture");
    // Nothing of the launcher's is left half-made.
    let left: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, [name]);
}

/// What Home is given to show is bounded by what the file holds, not by its name: only a
/// still PNG, JPEG or WebP within the pixel limits is kept, and a kept copy that isn't one
/// is an error for Home, never a picture.
#[test]
fn only_a_bounded_still_picture_is_kept_or_shown() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("data");
    let write = |name: &str, format: image::ImageFormat, w: u32, h: u32| {
        let path = root.path().join(name);
        let mut bytes = Vec::new();
        image::RgbImage::new(w, h)
            .write_to(&mut std::io::Cursor::new(&mut bytes), format)
            .unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    };
    // An animation under a permitted name is refused by what it is.
    let gif = write("party.png", image::ImageFormat::Gif, 160, 90);
    assert!(keep_background(&dir, &gif)
        .unwrap_err()
        .contains("PNG, JPEG or WebP"));
    // So is a picture past the pixel limits, however small its file.
    let wide = write("wide.png", image::ImageFormat::Png, 9000, 64);
    assert!(keep_background(&dir, &wide)
        .unwrap_err()
        .contains("9000×64"));
    // And one whose header is fine but whose pixels are not there.
    let whole = std::fs::read(write("cut.png", image::ImageFormat::Png, 320, 200)).unwrap();
    let cut = root.path().join("cut.png");
    std::fs::write(&cut, &whole[..whole.len() / 2]).unwrap();
    assert!(keep_background(&dir, &cut).is_err());
    assert!(!dir.exists() || std::fs::read_dir(&dir).unwrap().next().is_none());

    // The kind kept is the kind held, whatever the name says.
    let jpeg = write("dusk.png", image::ImageFormat::Jpeg, 3200, 200);
    let name = keep_background(&dir, &jpeg).unwrap();
    assert!(name.ends_with(".jpg"), "{name}");
    // Home gets one still picture, scaled to what a window shows.
    assert_eq!(background_picture(&dir, &name).unwrap().width(), 2560);

    // A copy an older launcher kept, which can't be decoded: an error, not a picture.
    std::fs::write(dir.join("background-old.png"), &whole[..whole.len() / 2]).unwrap();
    assert!(background_picture(&dir, "background-old.png").is_err());
    std::fs::write(dir.join("background-gif.png"), std::fs::read(&gif).unwrap()).unwrap();
    assert!(background_picture(&dir, "background-gif.png").is_err());
    assert!(background_picture(&dir, "background-gone.png").is_err());
}

#[test]
fn daybreak_and_parchment_are_the_only_light_choices() {
    for id in ["dawn_light", "parchment"] {
        assert!(is_light(Some(id)), "{id}");
    }
    for id in [
        None,
        Some("dawn"),
        Some("nord"),
        Some("twilight"),
        Some("midnight"),
    ] {
        assert!(!is_light(id), "{id:?}");
    }
    for t in THEMES.iter() {
        let light = t.id == "dawn_light" || t.id == "parchment";
        assert_eq!(t.is_light(), light, "{}", t.id);
        assert_eq!(t.is_sunrise(), t.id.starts_with("dawn"), "{}", t.id);
    }
    assert_eq!(accent_key(Some("dawn_light")), "dawn_light");
    assert_eq!(own_accent(Some("dawn_light")).rgb(), 0xc2552d);
}

/// The gallery shows these eight, in this order, under these saved ids.
#[test]
fn the_gallery_lists_eight_themes_in_order() {
    let shown: Vec<(&str, &str)> = GALLERY.iter().map(|t| (t.id, t.name)).collect();
    assert_eq!(
        shown,
        [
            ("dawn", "Sunrise"),
            ("midnight", "Midnight"),
            ("grove", "Grove"),
            ("amethyst", "Amethyst"),
            ("ember", "Ember"),
            ("graphite", "Graphite"),
            ("parchment", "Parchment"),
            ("dawn_light", "Daybreak"),
        ]
    );
    for t in GALLERY {
        assert!(THEMES.iter().any(|known| known.id == t.id), "{}", t.id);
    }
    // Sunrise and Daybreak are the palettes they always were.
    assert_eq!(DAWN_THEME.palette.map(|p| p.page), Some(0x2b2324));
    assert_eq!(DAWN_LIGHT_THEME.palette.map(|p| p.page), Some(0xf7f1ee));
}

/// Each new built-in resolves to a palette with its own text, cards and accent (the hexes its
/// tile is drawn in), and is not one of a player's own-colour themes.
#[test]
fn the_new_built_ins_resolve_to_their_colours() {
    // id, background, sidebar, cards, text, accent
    let colours = [
        ("midnight", 0x141a2b, 0x0f1422, 0x1c2438, 0xe8ecf6, 0x6c8cff),
        ("grove", 0x1a221d, 0x141a16, 0x212b25, 0xe6efe8, 0x6fbf73),
        ("amethyst", 0x1e1a2b, 0x171421, 0x262137, 0xefeaf8, 0xb18cff),
        ("ember", 0x1f1715, 0x18110f, 0x2a1e1b, 0xf5e9e3, 0xff7a3d),
        ("graphite", 0x1c1c1e, 0x141416, 0x252527, 0xededee, 0xd4d4d8),
        (
            "parchment",
            0xfbf7f0,
            0xf1ebe0,
            0xffffff,
            0x2b2824,
            0x8a5a2b,
        ),
    ];
    for (id, page, side, card, ink, accent) in colours {
        let Resolved::Builtin(t) = resolve(Some(id)) else {
            panic!("{id} is not a built-in");
        };
        assert_eq!(t.id, id);
        let p = t.palette.expect("a palette");
        assert_eq!((p.page, p.side, p.card), (page, side, card), "{id}");
        assert_eq!(p.ink, ink, "{id}");
        assert_eq!(p.accent, accent, "{id}");
        assert_eq!(p.dark, id != "parchment", "{id}");
        assert_eq!(own_accent(Some(id)).rgb(), accent, "{id}");
    }
}

/// A theme saved before the gallery still paints, and the new Midnight is not Midnight Aurora.
#[test]
fn the_older_built_ins_still_resolve() {
    for id in [
        "twilight",
        "nord",
        "taiga",
        "hypr_neon",
        "deep_ocean",
        "autumn_forest",
        "cherry_blossom",
        "midnight_aurora",
        "overworld",
        "first_light",
    ] {
        match resolve(Some(id)) {
            Resolved::Twilight => assert_eq!(id, "twilight"),
            Resolved::Builtin(t) => assert_eq!(t.id, id),
            _ => panic!("{id} no longer resolves"),
        }
        if id != "twilight" {
            assert!(
                !GALLERY.iter().any(|t| t.id == id),
                "{id} is in the gallery"
            );
        }
    }
    let (new, old) = (theme(Some("midnight")), theme(Some("midnight_aurora")));
    assert_ne!(new.map(|t| t.id), old.map(|t| t.id));
}

#[test]
fn hsv_and_rgb_agree_on_the_primaries_and_greys() {
    use super::hsv::Hsv;
    let hsv = |rgb| Hsv::from_rgb(rgb);
    assert_eq!(
        hsv(0xff0000),
        Hsv {
            h: 0.,
            s: 1.,
            v: 1.
        }
    );
    assert_eq!(
        hsv(0x00ff00),
        Hsv {
            h: 120.,
            s: 1.,
            v: 1.
        }
    );
    assert_eq!(
        hsv(0x0000ff),
        Hsv {
            h: 240.,
            s: 1.,
            v: 1.
        }
    );
    assert_eq!(
        hsv(0xffffff),
        Hsv {
            h: 0.,
            s: 0.,
            v: 1.
        }
    );
    assert_eq!(
        hsv(0x000000),
        Hsv {
            h: 0.,
            s: 0.,
            v: 0.
        }
    );
    assert_eq!(
        Hsv {
            h: 60.,
            s: 1.,
            v: 1.
        }
        .to_rgb(),
        0xffff00
    );
    assert_eq!(
        Hsv {
            h: 180.,
            s: 1.,
            v: 1.
        }
        .to_rgb(),
        0x00ffff
    );
    assert_eq!(
        Hsv {
            h: 300.,
            s: 1.,
            v: 1.
        }
        .to_rgb(),
        0xff00ff
    );
    // A hue past a full turn, or below zero, is the same hue.
    assert_eq!(
        Hsv {
            h: 480.,
            s: 1.,
            v: 1.
        }
        .to_rgb(),
        0x00ff00
    );
    assert_eq!(
        Hsv {
            h: -120.,
            s: 1.,
            v: 1.
        }
        .to_rgb(),
        0x0000ff
    );
    assert_eq!(Hsv::pure(0.), 0xff0000);
}

#[test]
fn every_rgb_survives_the_trip_through_hsv() {
    use super::hsv::Hsv;
    for r in (0..=255u32).step_by(15) {
        for g in (0..=255u32).step_by(17) {
            for b in (0..=255u32).step_by(5) {
                let rgb = r << 16 | g << 8 | b;
                assert_eq!(Hsv::from_rgb(rgb).to_rgb(), rgb, "{rgb:06x}");
            }
        }
    }
}

#[test]
fn a_pointer_on_a_bar_is_a_share_of_it_held_to_the_bar() {
    use super::hsv::{fraction, hue_pick, square_pick};
    assert_eq!(fraction(110., 10., 200.), 0.5);
    assert_eq!(fraction(-5., 10., 200.), 0.);
    assert_eq!(fraction(900., 10., 200.), 1.);
    assert_eq!(fraction(5., 10., 0.), 0.);
    assert_eq!(hue_pick(10., 10., 360.), 0.);
    assert_eq!(hue_pick(190., 10., 360.), 180.);
    assert!(hue_pick(5000., 10., 360.) < 360.);
    // Top right of the square is the pure hue; bottom left is black; the left edge is grey.
    let top_right = square_pick(120., (110., 20.), (10., 20.), (100., 50.));
    assert_eq!(top_right.to_rgb(), 0x00ff00);
    let bottom_left = square_pick(120., (10., 70.), (10., 20.), (100., 50.));
    assert_eq!(bottom_left.to_rgb(), 0x000000);
    let left_middle = square_pick(120., (10., 45.), (10., 20.), (100., 50.));
    assert_eq!((left_middle.s, left_middle.v), (0., 0.5));
}
