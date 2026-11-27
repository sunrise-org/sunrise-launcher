use super::*;
use crate::theme::{self, from_settings};
use gpui::WindowAppearance;
use sunrise_launcher::backend::Appearance;
use sunrise_launcher::themes::{self, Accent};
use sunrise_launcher_theme::glass;

fn look(id: Option<&str>, accent: Accent, glass: f32, scenery: bool) -> (Settings, Theme) {
    let settings = Settings {
        accent,
        theme: id.map(Into::into),
        glass: Some(glass),
        scenery_on: Some(scenery),
        ..Settings::default()
    };
    let theme = from_settings(&settings, WindowAppearance::Dark);
    (settings, theme)
}

/// The classes of look: each theme at its own accent; one theme in every named accent and
/// in a pale, a dark, a grey and a saturated typed one; the least and most Glass; scenery
/// off; and the light appearance. `all` is every theme with every accent and setting.
fn looks(all: bool) -> Vec<(String, Settings, Theme)> {
    let ids = std::iter::once(None).chain(themes::THEMES.iter().map(|t| Some(t.id)));
    let typed = [0xffe39a, 0x101040, 0x808080, 0xff0000].map(Accent::from_rgb);
    let mut looks = Vec::new();
    for id in ids {
        let own = themes::starting_accent(id, &Default::default());
        let accents = if all || id.is_none() {
            [vec![own], Accent::ALL.to_vec(), typed.to_vec()].concat()
        } else {
            vec![own]
        };
        for accent in accents {
            for (glass, scenery) in [(6., true), (0., true), (16., true), (6., false)] {
                if all || id.is_none() || (glass == 6. && scenery) {
                    let (settings, theme) = look(id, accent, glass, scenery);
                    looks.push((
                        format!("{id:?} {} {glass} {scenery}", accent.name()),
                        settings,
                        theme,
                    ));
                }
            }
        }
    }
    for accent in Accent::ALL {
        let theme = Theme::for_appearance(Appearance::Light, accent, WindowAppearance::Light);
        looks.push((
            format!("light {}", accent.name()),
            Settings {
                accent,
                ..Settings::default()
            },
            theme,
        ));
    }
    looks
}

/// Ids as the registry and the packs' folders spell them.
const IDS: [&str; 12] = [
    "jei",
    "coords_hud",
    "lanterns",
    "sodium",
    "iris",
    "waystones",
    "minimap",
    "splitscreen",
    "replay",
    "Faithful 32x.zip",
    "complementary-reimagined",
    "",
];

/// The channel-wise hue, in radians.
fn hue(c: u32) -> f32 {
    let [r, g, b] = [16, 8, 0].map(|s| ((c >> s) & 255) as f32);
    (3f32.sqrt() * (g - b)).atan2(2. * r - g - b)
}

// The ordinary ramp scales RGB channels, while the authored vibrant tones are matched
// to the accent in OKLab so their hue does not drift as chroma goes up.
fn play_hue(c: u32, vivid: bool) -> f32 {
    if vivid {
        let [_, a, b] = oklab(c);
        b.atan2(a)
    } else {
        hue(c)
    }
}

fn spread(c: u32) -> u32 {
    let ch = [16, 8, 0].map(|s| (c >> s) & 255);
    ch.iter().max().unwrap() - ch.iter().min().unwrap()
}

/// Everything the palette promises, for one look.
fn check(name: &str, settings: &Settings, theme: &Theme) {
    let p = Palette::of(settings, theme);
    let reads = |role: &str, fg: u32, on: &[u32], min: f32| {
        for &s in on {
            let r = contrast(fg, s);
            assert!(r >= min, "{name}: {role} on {s:06x} {r:.2}");
        }
    };
    // The swatch stays as picked; the gradient holds the white label at every point.
    assert_eq!(p.accent, theme.accent_choice.rgb(), "{name}: raw accent");
    assert_eq!(p.accent_on, 0xffffff, "{name}: Play's text is white");
    let [dark, light] = p.play_stops;
    let ramp = std::array::from_fn::<_, 101, _>(|i| over(light, i as f32 / 100., dark));
    reads("white on Play", p.accent_on, &ramp, TEXT);
    assert!(luminance(light) > luminance(dark));
    // Play is its accent (or the theme's authored face) pulled toward the theme's chrome.
    let toned = mix_oklab(p.chrome, theme.play_seed.unwrap_or(p.accent), PLAY_TONE);
    assert_eq!(p.play_glow, toned, "{name}: Play's colour");
    if spread(p.accent) > 60 && spread(dark) > 40 {
        let vivid = theme.play_seed.is_some();
        let drift = (play_hue(dark, vivid) - play_hue(toned, vivid)).abs();
        assert!(
            drift.min(std::f32::consts::TAU - drift) < 0.1,
            "{name}: hue {drift}"
        );
    }
    let surfaces = [
        p.background,
        p.card_color,
        p.raised,
        p.field,
        p.hover,
        p.selected,
    ];
    reads("text", p.text, &surfaces, TEXT);
    reads("muted", p.muted, &surfaces, TEXT);
    for (role, fg) in [
        ("accent ink", p.accent_ink),
        ("danger", p.danger),
        ("info", p.info),
        ("success", p.success),
        ("warning", p.warning),
    ] {
        reads(role, fg, &surfaces[..5], TEXT);
    }
    reads(
        "focus ring",
        p.focus,
        &[&surfaces[..], &[p.chrome, p.chrome_raised]].concat(),
        MARK,
    );
    reads("edge", p.line, &surfaces[..4], MARK);
    let chrome = [p.chrome, p.chrome_raised];
    reads("chrome text", p.chrome_text, &chrome, TEXT);
    reads("chrome muted", p.chrome_muted, &chrome, TEXT);
    reads("chrome edge", p.chrome_line, &chrome, MARK);

    // Plates: opaque without scenery, else at least the Glass setting, and text on them
    // reads over each band of the theme's sky.
    assert_eq!(
        (p.card().a, p.chrome_plate().a),
        (p.alpha, p.alpha),
        "{name}"
    );
    if !settings.scenery() {
        assert_eq!(p.alpha, 1., "{name}: opaque without scenery");
    }
    let tint = settings.glass_tint();
    assert!(
        p.alpha + f32::EPSILON >= (tint + (1. - tint) * PLATE_BASE).min(1.),
        "{name}"
    );
    for sky in theme.night.sky {
        for (fg, plate) in [
            (p.text, p.card_color),
            (p.muted, p.card_color),
            (p.chrome_text, p.chrome),
            (p.chrome_muted, p.chrome),
        ] {
            reads(
                "text on a plate over the sky",
                fg,
                &[over(plate, p.alpha, sky)],
                TEXT,
            );
        }
    }
    // A tile without a picture: its letter reads, and every tile stands the same small way
    // off the card (lighter on a dark theme, darker on a light one), in a quiet colour.
    let lighter = luminance(p.text) > luminance(p.card_color);
    for id in IDS {
        let t = p.tile(id);
        reads(
            &format!("the letter of {id}'s tile"),
            t.ink,
            &[t.fill],
            TEXT,
        );
        let apart = contrast(t.fill, p.card_color);
        assert!(
            (TILE_APART - 0.08..=TILE_APART + 0.08).contains(&apart),
            "{name}: {id}'s tile {:06x} is {apart:.2} from the card",
            t.fill
        );
        // Only a card with no room on its own side (a white one under pale text, say)
        // sends its tiles the other way.
        let room = match lighter {
            true => (luminance(p.card_color) + 0.05) * TILE_APART <= 1.05,
            false => (luminance(p.card_color) + 0.05) / TILE_APART >= 0.05,
        };
        assert_eq!(
            luminance(t.fill) > luminance(p.card_color),
            lighter == room,
            "{name}: {id}'s tile {:06x} is on the wrong side of the card",
            t.fill
        );
        assert!(
            spread(t.fill) <= spread(p.card_color) + 64,
            "{name}: {id}'s tile {:06x} is loud",
            t.fill
        );
    }
    // Fields, selections, tooltips and menus are opaque: a bright page behind changes nothing.
    reads("field text", p.text, &[p.field], TEXT);
    reads("placeholder", p.muted, &[p.field], TEXT);
    reads("tooltip", p.text, &[p.raised], TEXT);
    if spread(p.accent) > 30 {
        assert_ne!(p.selected, p.field, "{name}: a selection shows in a field");
    }
    assert_eq!(p.scrim().a, p.scrim_alpha);
    for page in BRIGHT {
        let edge = contrast(p.raised, over(SCRIM, p.scrim_alpha, page));
        assert!(
            edge >= 1.5,
            "{name}: dialog over the scrim over {page:06x} {edge:.2}"
        );
    }
}

#[test]
fn a_sample_of_every_class_of_look_holds_the_palette_contract() {
    for (name, settings, theme) in looks(false) {
        check(&name, &settings, &theme);
    }
}

#[test]
#[ignore = "exhaustive: every theme with every accent and setting"]
fn every_look_holds_the_palette_contract() {
    for (name, settings, theme) in looks(true) {
        check(&name, &settings, &theme);
    }
}

/// A mod without a picture keeps one colour of its own: the same whenever it is asked for,
/// another mod's is another colour, and the ids' hues go all the way round the wheel.
#[test]
fn a_tile_is_one_colour_per_id() {
    let ids = [Some("twilight"), None]
        .into_iter()
        .chain(themes::THEMES.iter().map(|t| Some(t.id)));
    for id in ids {
        let own = themes::starting_accent(id, &Default::default());
        let (settings, theme) = look(id, own, 6., true);
        let p = Palette::of(&settings, &theme);
        let tiles = IDS.map(|mod_id| p.tile(mod_id));
        assert_eq!(tiles, IDS.map(|mod_id| p.tile(mod_id)), "{id:?}: stable");
        for (i, a) in tiles.iter().enumerate() {
            for b in &tiles[i + 1..] {
                assert_ne!(a.fill, b.fill, "{id:?}: {tiles:06x?}");
            }
        }
        // Coloured, not the grey every tile used to be.
        assert!(
            tiles.iter().all(|t| spread(t.fill) >= 12),
            "{id:?}: {tiles:06x?}"
        );
    }
    assert_eq!(tile_hue("jei"), tile_hue("jei"));
    // Ids a letter apart land far apart, and a thousand ids leave no twelfth of the wheel
    // empty.
    let apart = (tile_hue("mod-a") - tile_hue("mod-b")).abs();
    assert!(apart.min(1. - apart) > 0.02, "{apart}");
    let mut sectors = [0; 12];
    for n in 0..1000 {
        sectors[(tile_hue(&format!("mod-{n}")) * 12.) as usize] += 1;
    }
    assert!(sectors.iter().all(|&n| n >= 40), "{sectors:?}");
}

/// Old Dawn settings use the brand orange, not the retired pink, under the glass.
#[test]
fn dawns_play_is_the_brand_orange_under_white() {
    for json in [
        "{}",
        r#"{"theme":"dawn"}"#,
        r##"{"accent":"#c2366f"}"##,
        r##"{"theme":"dawn","accent":"#c2366f"}"##,
    ] {
        let settings = Settings::parse(json);
        let theme = from_settings(&settings, WindowAppearance::Dark);
        let p = Palette::of(&settings, &theme);
        assert_eq!((p.accent, p.accent_on), (0xc2552d, 0xffffff));
        assert!(contrast(p.accent_on, p.play_stops[1]) >= TEXT);
    }
    assert!(
        contrast(0xffffff, 0xc4623a) < TEXT,
        "the site's hover orange is too light"
    );
}

#[test]
fn every_builtin_play_is_its_accent_toned_to_the_theme_at_readable_white_contrast() {
    let ids = [themes::TWILIGHT]
        .into_iter()
        .chain(themes::THEMES.iter().map(|t| t.id));
    for id in ids {
        let own = themes::own_accent(Some(id)).rgb();
        let settings = Settings::parse(&format!(r#"{{"theme":"{id}"}}"#));
        let theme = from_settings(&settings, WindowAppearance::Dark);
        let p = Palette::of(&settings, &theme);
        assert_eq!(p.accent, own, "{id}: the theme's accent is unchanged");
        assert_eq!(p.accent_on, 0xffffff, "{id}: white label and icon");
        let [dark, light] = p.play_stops;
        let bright_contrast = contrast(p.accent_on, light);
        let dark_contrast = contrast(p.accent_on, dark);
        assert!(bright_contrast >= TEXT, "{id}: {bright_contrast:.3}");
        assert!(
            dark_contrast >= bright_contrast + 1.5,
            "{id}: flat gradient"
        );
        // The accent (or the authored face) pulled exactly PLAY_TONE toward the theme's
        // chrome, and both stops keep that blend's hue.
        let toned = mix_oklab(p.chrome, theme.play_seed.unwrap_or(own), PLAY_TONE);
        assert_eq!(p.play_glow, toned, "{id}: Play's colour");
        let vivid = theme.play_seed.is_some();
        let drift = (play_hue(dark, vivid) - play_hue(toned, vivid)).abs();
        assert!(
            drift.min(std::f32::consts::TAU - drift) < 0.02,
            "{id}: hue drift {drift:.3}"
        );
    }
}

#[test]
fn cherry_and_nord_keep_their_swatches_with_vivid_primary_actions() {
    let chroma = |c: u32| {
        let [_, a, b] = oklab(c);
        a.hypot(b)
    };
    for (id, accent, seed) in [
        ("cherry_blossom", 0xf59ac0, 0xc8297e),
        ("nord", 0x88c0d0, 0x087d95),
    ] {
        let settings = Settings::parse(&format!(r#"{{"theme":"{id}"}}"#));
        let theme = from_settings(&settings, WindowAppearance::Dark);
        let p = Palette::of(&settings, &theme);
        assert_eq!(p.accent, accent);
        // The authored face, toned like every Play, stays more vivid than the toned swatch.
        assert_eq!(p.play_glow, mix_oklab(p.chrome, seed, PLAY_TONE), "{id}");
        let swatch = chroma(mix_oklab(p.chrome, accent, PLAY_TONE));
        for stop in p.play_stops {
            assert!(chroma(stop) > swatch, "{id}: faded stop {stop:06x}");
        }
        assert_eq!(p.play_glow, p.play_stops[1]);
    }

    let custom = Accent::from_rgb(0x206fcc);
    let (settings, theme) = look(Some("cherry_blossom"), custom, 6., true);
    let p = Palette::of(&settings, &theme);
    assert_eq!(
        p.play_glow,
        mix_oklab(p.chrome, custom.rgb(), PLAY_TONE),
        "custom accents keep their own glow, toned like every Play"
    );
}

/// A player's theme holds the same contract as a built-in one, whatever six colours it
/// was let in with: dark, light, a pale accent, surfaces that differ.
#[test]
fn a_players_theme_holds_the_palette_contract() {
    use sunrise_launcher::theme_files::parse;
    let files = [
        (
            "tidepool",
            r##"card = "#1A3135"
chrome = "#11262B"
text = "#F0F8F4"
muted = "#C3D7D2"
background = "#102328"
accent = "#8EDBCB""##,
        ),
        (
            "paper",
            r##"card = "#ffffff"
chrome = "#e8e4dc"
text = "#1b1b1b"
muted = "#555555"
background = "#f4f1ea"
accent = "#ffd84a""##,
        ),
        (
            "faint inks",
            r##"card = "#202020"
text = "#3a3a3a"
muted = "#303030""##,
        ),
        (
            "apart",
            r##"card = "#3a2a10"
chrome = "#000000"
background = "#101820"
accent = "#101010""##,
        ),
        ("accent only", r##"accent = "#ff66cc""##),
    ];
    for (name, palette) in files {
        let text = format!("version = 1\nname = \"{name}\"\n[palette]\n{palette}\n");
        let roles = parse(&text)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"))
            .roles;
        let accent = Accent::from_rgb(roles.accent);
        for scenery in [true, false] {
            let settings = Settings {
                accent,
                scenery_on: Some(scenery),
                ..Settings::default()
            };
            let theme = Theme::for_appearance(Appearance::Dark, accent, WindowAppearance::Dark)
                .with_night(roles.night());
            check(
                &format!("local {name}, scenery {scenery}"),
                &settings,
                &theme,
            );
            let p = Palette::of(&settings, &theme);
            assert_eq!(p.accent, roles.accent, "{name}");
            assert_eq!(p.accent_on, 0xffffff, "{name}");
            assert_eq!(p.background, roles.background, "{name}");
        }
    }
}

#[test]
fn reduced_graphics_makes_glass_opaque_without_hiding_the_scenery() {
    let mut settings = Settings::parse(r#"{"scenery":true,"blur":8.5,"panel_opacity":82}"#);
    let theme = from_settings(&settings, WindowAppearance::Dark);
    assert!(Palette::of(&settings, &theme).alpha < 1.);
    assert_eq!(theme.look.blur, 8.5);

    settings.reduced_graphics = Some(true);
    let theme = from_settings(&settings, WindowAppearance::Dark);
    let p = Palette::of(&settings, &theme);
    assert_eq!(p.card().a, 1.);
    assert_eq!(p.chrome_plate().a, 1.);
    assert!(Inputs::of(&settings, &theme, None, Some(82)).plates == Plates::Opaque);
    assert_eq!(theme.look.blur, 0.);
    assert!(settings.scenery());
}

/// A picture behind Home, the player's or their theme's, may be any colour: the plates
/// over it are opaque.
#[test]
fn plates_use_the_displayed_photo_only_and_stay_opaque_over_it() {
    let settings = Settings {
        background: Some("background-0123456789ab.png".into()),
        ..Settings::default()
    };
    let theme = from_settings(&settings, WindowAppearance::Dark);
    // On Settings, or while the Home image is loading, the scene still supplies the tint.
    // Keep the existing automatic opacity independently of which tint is used.
    let scene = Palette::of(&settings, &theme);
    assert_eq!(scene.alpha, 1.);
    let mut photo = HomePhoto {
        image: std::sync::Arc::new(gpui::RenderImage::new([image::Frame::new(
            image::RgbaImage::new(1, 1),
        )])),
        tone: None,
    };
    let transparent = Inputs::of(&settings, &theme, Some(&photo), None).palette();
    assert_eq!(transparent.card_color, scene.card_color);
    assert_eq!(transparent.chrome, scene.chrome);
    photo.tone = Some(0xe0ebf8);
    let shown = Inputs::of(&settings, &theme, Some(&photo), None).palette();
    assert_ne!(shown.card_color, scene.card_color);
    assert_ne!(shown.chrome, scene.chrome);
    assert_eq!(shown.play_stops, scene.play_stops);
    assert_eq!(shown.card().a, 1.);
    assert!(
        luminance(shown.hero.shade) > 0.5,
        "shade a pale photo lightly"
    );
    for ink in [shown.text, shown.muted, shown.warning, shown.accent_ink] {
        assert!(contrast(ink, shown.card_color) >= TEXT, "{ink:06x}");
    }
    let off = Settings {
        scenery_on: Some(false),
        ..settings
    };
    let theme = from_settings(&off, WindowAppearance::Dark);
    assert!(!theme.photo);
    assert_eq!(Palette::of(&off, &theme).alpha, 1.);
}

/// Any wallpaper pixel lies between black and white. Include the actual sky stops and the
/// opaque interactive fills too, so tuning transparency can't break the rest of the palette.
#[test]
fn panel_opacity_contrast_matrix() {
    use sunrise_launcher::backend::{PANEL_OPACITY_MAX, PANEL_OPACITY_MIN};
    // From 82% up every built-in theme's plate text reads over any backdrop pixel. Below that
    // the opacity is still drawn as chosen: plate text reads where the plate allows and is
    // otherwise at its end (white, or black on a light theme), and the opaque fills still hold.
    const READS_FROM: u8 = 82;
    let ids = std::iter::once(themes::TWILIGHT).chain(themes::THEMES.map(|t| t.id));
    let mut photo = HomePhoto {
        image: std::sync::Arc::new(gpui::RenderImage::new([image::Frame::new(
            image::RgbaImage::new(1, 1),
        )])),
        tone: None,
    };
    // The source changes both plates, while any backdrop pixel still lies within the bounds.
    let photo_tones = [
        0xe5e9ef, 0x15223b, 0xfefefe, 0xed2424, 0x35eb56, 0xffff00, 0x2754fc,
    ];
    for id in ids {
        let own = themes::own_accent(Some(id));
        let accents = [own, Accent::from_rgb(0x000000), Accent::from_rgb(0xffffff)]
            .into_iter()
            .chain(Accent::ALL);
        for accent in accents {
            for tint in [0., 6., 16.] {
                let (mut settings, theme) = look(Some(id), accent, tint, true);
                for percent in [PANEL_OPACITY_MIN, 40, READS_FROM, PANEL_OPACITY_MAX] {
                    settings.set_panel_opacity(Some(percent));
                    for tone in std::iter::once(None).chain(photo_tones.map(Some)) {
                        photo.tone = tone;
                        let shown = tone.map(|_| &photo);
                        let p = Inputs::of(&settings, &theme, shown, None).palette();
                        let name = format!(
                            "{id}, {accent:?}, photo {tone:06x?}, Glass {tint}, opacity {percent}%"
                        );
                        assert_eq!(p.alpha, f32::from(percent) / 100., "{name}");
                        assert_eq!(p.card().a, p.alpha);
                        assert_eq!(p.chrome_plate().a, p.alpha);
                        let check = |surface: u32, ink: u32, min: f32, role: &str| {
                            let ratio = contrast(ink, surface);
                            assert!(
                                ratio >= min,
                                "{name}: {role} {ink:06x} on {surface:06x}: {ratio:.3}"
                            );
                        };
                        let end = if theme.night.dark { 0xffffff } else { 0 };
                        // On a plate, below READS_FROM: readable, or the most any ink can do.
                        let on_plate = |surface: u32, ink: u32, min: f32, role: &str| {
                            if percent >= READS_FROM || contrast(ink, surface) >= min {
                                check(surface, ink, min, role);
                            } else {
                                assert_eq!(ink, end, "{name}: {role} on {surface:06x}");
                            }
                        };
                        for behind in [0, 0xffffff, 0x808080].into_iter().chain(theme.night.sky) {
                            for plate in [p.card_color, p.chrome] {
                                let surface = over(plate, p.alpha, behind);
                                for (ink, role) in [
                                    (p.text, "body"),
                                    (p.muted, "muted"),
                                    (p.accent_ink, "accent"),
                                ] {
                                    on_plate(surface, ink, TEXT, role);
                                }
                                for (ink, role) in [(p.focus, "focus"), (p.line, "control edge")] {
                                    on_plate(surface, ink, MARK, role);
                                }
                            }
                            let chrome = over(p.chrome, p.alpha, behind);
                            on_plate(chrome, p.chrome_text, TEXT, "chrome body");
                            on_plate(chrome, p.chrome_muted, TEXT, "chrome muted");
                            on_plate(chrome, p.chrome_line, MARK, "chrome edge");
                        }
                        for surface in [p.raised, p.field, p.hover] {
                            for (ink, role) in [
                                (p.text, "body"),
                                (p.muted, "muted"),
                                (p.accent_ink, "accent"),
                            ] {
                                check(surface, ink, TEXT, role);
                            }
                            check(surface, p.focus, MARK, "focus");
                        }
                        for ink in [p.text, p.muted] {
                            check(p.selected, ink, TEXT, "selection");
                        }
                        check(p.chrome_raised, p.chrome_text, TEXT, "raised chrome body");
                        check(p.chrome_raised, p.chrome_muted, TEXT, "raised chrome muted");
                        check(p.chrome_raised, p.focus, MARK, "raised chrome focus");
                    }
                }
            }
        }
    }
}

#[test]
fn panel_opacity_preview_reset_and_backdrop_follow_the_frame() {
    use sunrise_launcher::backend::PANEL_OPACITY_MIN;
    let floor = f32::from(PANEL_OPACITY_MIN) / 100.;
    let mut settings = Settings::default();
    let theme = from_settings(&settings, WindowAppearance::Dark);
    let automatic = Palette::of(&settings, &theme);
    begin(&settings, &theme, None, Some(PANEL_OPACITY_MIN));
    assert_eq!(palette().alpha, floor);
    assert_eq!(settings.panel_opacity(), None, "preview is not a setting");
    begin(&settings, &theme, None, None);
    assert_eq!(palette(), automatic);
    settings.set_panel_opacity(Some(PANEL_OPACITY_MIN));
    assert_eq!(Palette::of(&settings, &theme).alpha, floor);
    settings.set_panel_opacity(None);
    assert_eq!(Palette::of(&settings, &theme), automatic);

    settings.background = Some("background-0123456789ab.png".into());
    let photo = from_settings(&settings, WindowAppearance::Dark);
    assert_eq!(Palette::of(&settings, &photo).alpha, 1.);
    settings.set_panel_opacity(Some(PANEL_OPACITY_MIN));
    assert_eq!(Palette::of(&settings, &photo).alpha, floor);
    // The renderer can substitute a scene for an unavailable picture.
    assert_eq!(Palette::of(&settings, &theme).alpha, floor);
    settings.scenery_on = Some(false);
    assert_eq!(Palette::of(&settings, &photo).alpha, 1.);
}

#[test]
fn even_a_pale_blue_source_makes_slate_blue_plates() {
    let pale_blue = 0xe5e9ef;
    let tint = Palette::plain(
        theme::night::NIGHT,
        glass::ACCENT,
        glass::TINT,
        Some(pale_blue),
    );
    for plate in [tint.chrome, tint.card_color, tint.raised] {
        let [_, a, b] = oklab(plate);
        assert!(b < 0., "{plate:06x} should be blue");
        assert!(a.hypot(b) >= 0.044, "{plate:06x} should not read as gray");
        assert!((hue(plate) - hue(pale_blue)).abs() < 0.1, "{plate:06x}");
    }
    let gray = Palette::plain(
        theme::night::NIGHT,
        glass::ACCENT,
        glass::TINT,
        Some(0xe9e9e9),
    );
    for plate in [gray.chrome, gray.card_color, gray.raised] {
        assert_eq!(spread(plate), 0);
    }
}

#[test]
fn unboxed_home_text_reads_over_the_shaded_backdrop() {
    let themes = looks(false)
        .into_iter()
        .map(|(_, s, t)| Palette::of(&s, &t));
    let light = Palette::plain(theme::night::DAWN, 0x6a4910, glass::TINT, None);
    assert!(luminance(light.hero.shade) > 0.5);
    for p in themes.chain([light]) {
        for pixel in [0, 0xffffff] {
            let behind = over(p.hero.shade, p.hero.shade_alpha, pixel);
            for ink in [p.hero.text, p.hero.warning, p.hero.link] {
                assert!(contrast(ink, behind) >= TEXT, "{ink:06x} on {behind:06x}");
            }
        }
    }
}

#[test]
fn built_in_palettes_take_their_hue_from_their_scene() {
    use std::collections::HashSet;
    let mut cards = HashSet::new();
    for id in [themes::TWILIGHT]
        .into_iter()
        .chain(themes::THEMES.iter().map(|t| t.id))
    {
        let settings = Settings {
            theme: Some(id.into()),
            ..Settings::default()
        };
        let theme = from_settings(&settings, WindowAppearance::Dark);
        let tone = theme.scene_tone.expect("built-in scene supplies a tone");
        let p = Palette::of(&settings, &theme);
        assert!(cards.insert(p.card_color), "{id}: distinct palette");
        assert!(spread(p.card_color) > 16, "{id}: chromatic palette");
        let drift = (hue(p.card_color) - hue(tone)).abs();
        assert!(
            drift.min(std::f32::consts::TAU - drift) < 0.16,
            "{id}: hue drift {drift}"
        );
    }
    assert_eq!(cards.len(), 11);
}

#[test]
fn white_text_on_a_pale_accent_darkens_the_fill_only() {
    let (settings, theme) = look(None, Accent::from_rgb(0xffe39a), 6., true);
    let p = Palette::of(&settings, &theme);
    assert_eq!(p.accent, 0xffe39a);
    assert_ne!(p.play_stops[0], p.accent);
    let (settings, theme) = look(None, Accent::from_rgb(0x372890), 6., true);
    let p = Palette::of(&settings, &theme);
    let toned = mix_oklab(p.chrome, p.accent, PLAY_TONE);
    assert_eq!(p.play_stops[0], toned, "a dark accent needs no shading");
    assert_eq!(p.play_stops[1], mix_oklab(p.accent_on, toned, 0.25));
}

/// The theme and the player's accent decide the palette; a frame reads what it began, and
/// the same inputs are not computed again.
#[test]
fn begin_sets_the_frames_palette_from_the_inputs() {
    let (violet, a) = look(None, Accent::default(), 6., true);
    let nord = Settings {
        theme: Some("nord".into()),
        accent: Accent::from_rgb(0x88c0d0),
        ..Settings::default()
    };
    let b = from_settings(&nord, WindowAppearance::Dark);
    let (pa, pb) = (Palette::of(&violet, &a), Palette::of(&nord, &b));
    assert_eq!(
        (pb.background, pb.accent),
        (0x2e3440, 0x88c0d0),
        "Nord's night and frost"
    );
    assert_ne!(
        pb.play_stops[0], pb.accent,
        "pale frost cannot hold white text"
    );
    begin(&nord, &b, None, None);
    assert_eq!(palette(), pb);
    let kept = CURRENT.get().1;
    begin(&nord, &b, None, None);
    assert_eq!(CURRENT.get().1, kept);
    begin(&violet, &a, None, None);
    assert_eq!(palette(), pa);
    assert_eq!(pa.accent, glass::ACCENT);
}

/// Under a chosen opacity a plate's outlines and rules fade along its square root: slower than
/// the plate, so a thin panel keeps its edge, and gone with it at 0%. Without a chosen opacity
/// they are solid.
#[test]
fn panel_lines_fade_slower_than_the_plates_and_go_with_them_at_zero() {
    let near = |a: f32, b: f32| assert!((a - b).abs() < 1e-3, "{a} != {b}");
    let mut settings = Settings::default();
    let theme = from_settings(&settings, WindowAppearance::Dark);
    assert_eq!(Palette::of(&settings, &theme).rule_alpha, 1.);
    for (percent, lines) in [
        (100, 1.),
        (82, 0.906),
        (50, 0.707),
        (25, 0.5),
        (4, 0.2),
        (0, 0.),
    ] {
        settings.set_panel_opacity(Some(percent));
        let p = Palette::of(&settings, &theme);
        near(p.rule_alpha, lines);
        near(p.rule().a, lines);
        near(p.plate_edge().a, p.input_edge().a * lines);
        assert!(
            p.rule_alpha >= p.alpha,
            "{percent}%: lines never fainter than the plate"
        );
    }
    settings.set_panel_opacity(None);
    assert_eq!(Palette::of(&settings, &theme).rule_alpha, 1.);
}
