use super::*;
use crate::theme::{self, from_settings};
use gpui::WindowAppearance;
use sunrise_launcher::backend::Appearance;
use sunrise_launcher::themes::{self, Accent};
use sunrise_launcher_theme::glass;

fn look(id: Option<&str>, accent: Accent) -> Theme {
    let settings = Settings {
        accent,
        theme: id.map(Into::into),
        ..Settings::default()
    };
    from_settings(&settings, WindowAppearance::Dark)
}

/// The classes of look: each theme at its own accent; one theme in every named accent and
/// in a pale, a dark, a grey and a saturated typed one; and the light appearance. `all` is
/// every theme with every accent.
fn looks(all: bool) -> Vec<(String, Theme)> {
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
            looks.push((format!("{id:?} {}", accent.name()), look(id, accent)));
        }
    }
    for accent in Accent::ALL {
        let theme = Theme::for_appearance(Appearance::Light, accent, WindowAppearance::Light);
        looks.push((format!("light {}", accent.name()), theme));
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
fn check(name: &str, theme: &Theme) {
    let p = Palette::of(theme);
    let reads = |role: &str, fg: u32, on: &[u32], min: f32| {
        for &s in on {
            let r = contrast(fg, s);
            assert!(r >= min, "{name}: {role} on {s:06x} {r:.2}");
        }
    };
    // The swatch stays as picked; the gradient holds the white label at every point.
    assert_eq!(p.accent, theme.accent_choice.rgb(), "{name}: raw accent");
    // The label on the accent is white where white reads, else a dark ink; it reads either way.
    reads("the label on the accent", p.accent_on, &[p.accent], TEXT);
    if contrast(0xffffff, p.accent) >= TEXT {
        assert_eq!(p.accent_on, 0xffffff, "{name}: white where it reads");
    }
    let [dark, light] = p.play_stops;
    let ramp = std::array::from_fn::<_, 101, _>(|i| over(light, i as f32 / 100., dark));
    reads("white on Play's gradient", 0xffffff, &ramp, TEXT);
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

    // Cards and captions are solid: nothing shows through them, so nothing else to fit.
    assert_eq!(p.card().a, 1., "{name}");
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
    for (name, theme) in looks(false) {
        check(&name, &theme);
    }
}

#[test]
#[ignore = "exhaustive: every theme with every accent and setting"]
fn every_look_holds_the_palette_contract() {
    for (name, theme) in looks(true) {
        check(&name, &theme);
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
        let theme = look(id, own);
        let p = Palette::of(&theme);
        let tiles = IDS.map(|mod_id| p.tile(mod_id));
        assert_eq!(tiles, IDS.map(|mod_id| p.tile(mod_id)), "{id:?}: stable");
        // Two ids whose hues land a hair apart may round to one fill on some cards; their
        // tiles still differ (the letter keeps the hue), and no two ids share a whole tile.
        for (i, a) in tiles.iter().enumerate() {
            for b in &tiles[i + 1..] {
                assert_ne!((a.fill, a.ink), (b.fill, b.ink), "{id:?}: {tiles:06x?}");
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
        let p = Palette::of(&theme);
        assert_eq!((p.accent, p.accent_on), (0xc2552d, 0xffffff));
        assert!(contrast(0xffffff, p.play_stops[1]) >= TEXT);
    }
    assert!(
        contrast(0xffffff, 0xc4623a) < TEXT,
        "the site's hover orange is too light"
    );
}

/// Sunrise in the light look paints its pages from `DAWN_LIGHT` (a light background), and in the
/// dark look from `DAWN_DARK`.
#[test]
fn sunrise_in_the_light_look_paints_from_dawn_light() {
    use sunrise_launcher::themes::{DAWN_DARK, DAWN_LIGHT};
    let light = Settings {
        theme: Some("dawn".into()),
        appearance: Appearance::Light,
        ..Settings::default()
    };
    let theme = from_settings(&light, WindowAppearance::Dark);
    assert_eq!(theme.night, DAWN_LIGHT);
    assert!(!theme.dark);
    let p = Palette::of(&theme);
    assert_eq!(p.background, DAWN_LIGHT.page);
    assert!(contrast(p.text, p.background) >= TEXT);

    let dark = Settings {
        theme: Some("dawn".into()),
        ..Settings::default()
    };
    let theme = from_settings(&dark, WindowAppearance::Light);
    assert_eq!(theme.night, DAWN_DARK);
    assert!(theme.dark);
    assert_eq!(Palette::of(&theme).background, DAWN_DARK.page);
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
        let p = Palette::of(&theme);
        assert_eq!(p.accent, own, "{id}: the theme's accent is unchanged");
        let [dark, light] = p.play_stops;
        let bright_contrast = contrast(0xffffff, light);
        let dark_contrast = contrast(0xffffff, dark);
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
        let p = Palette::of(&theme);
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
    let theme = look(Some("cherry_blossom"), custom);
    let p = Palette::of(&theme);
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
        let theme = Theme::for_appearance(Appearance::Dark, accent, WindowAppearance::Dark)
            .with_night(roles.night());
        check(&format!("local {name}"), &theme);
        let p = Palette::of(&theme);
        assert_eq!(p.accent, roles.accent, "{name}");
        assert!(contrast(p.accent_on, p.accent) >= TEXT, "{name}");
        assert_eq!(p.background, roles.background, "{name}");
    }
}

#[test]
fn unboxed_home_text_reads_over_the_shaded_backdrop() {
    let themes = looks(false).into_iter().map(|(_, t)| Palette::of(&t));
    let light = Palette::plain(theme::night::DAWN, 0x6a4910);
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
fn white_text_on_a_pale_accent_darkens_the_fill_only() {
    let theme = look(None, Accent::from_rgb(0xffe39a));
    let p = Palette::of(&theme);
    assert_eq!(p.accent, 0xffe39a);
    assert_ne!(p.play_stops[0], p.accent);
    let theme = look(None, Accent::from_rgb(0x372890));
    let p = Palette::of(&theme);
    let toned = mix_oklab(p.chrome, p.accent, PLAY_TONE);
    assert_eq!(p.play_stops[0], toned, "a dark accent needs no shading");
    assert_eq!(p.play_stops[1], mix_oklab(0xffffff, toned, 0.25));
}

/// The theme and the player's accent decide the palette; a frame reads what it began, and
/// the same inputs are not computed again.
#[test]
fn begin_sets_the_frames_palette_from_the_inputs() {
    let a = look(None, Accent::default());
    let nord = Settings {
        theme: Some("nord".into()),
        accent: Accent::from_rgb(0x88c0d0),
        ..Settings::default()
    };
    let b = from_settings(&nord, WindowAppearance::Dark);
    let (pa, pb) = (Palette::of(&a), Palette::of(&b));
    assert_eq!(
        (pb.background, pb.accent),
        (0x2e3440, 0x88c0d0),
        "Nord's night and frost"
    );
    assert_ne!(
        pb.play_stops[0], pb.accent,
        "pale frost cannot hold white text"
    );
    begin(&b);
    assert_eq!(palette(), pb);
    let kept = CURRENT.get().1;
    begin(&b);
    assert_eq!(CURRENT.get().1, kept);
    begin(&a);
    assert_eq!(palette(), pa);
    assert_eq!(pa.accent, glass::ACCENT);
}

/// Every surface of a player's theme follows its own colours: the text is their Text, the
/// cards their Cards, the sidebar and bar their Sidebar, the rules their Lines. Colours that
/// read badly on each other are painted as chosen all the same.
#[test]
fn a_players_colours_reach_every_role_as_chosen() {
    use sunrise_launcher::theme_files::Roles;
    let roles = Roles {
        background: 0x912634,
        card: 0xee3749,
        chrome: 0xe96033,
        text: 0x060200,
        muted: 0xb9aba6,
        accent: 0xc2552d,
        line: 0xf7c9a2,
        success: 0x7dc98c,
        danger: 0xec8d8d,
        light: false,
    };
    let night = roles.night();
    let p = Palette::plain(night, roles.accent);
    assert_eq!(p.text, roles.text);
    assert_eq!(p.muted, roles.muted);
    assert_eq!(p.background, roles.background);
    assert_eq!(p.card_color, roles.card);
    assert_eq!(p.chrome, roles.chrome);
    assert_eq!(p.divider, over(roles.line, night.rule_alpha, roles.card));
    // The selected row and the hover are made from the cards and the text, not a fixed grey.
    assert_ne!(
        p.selected,
        Palette::plain(Roles::DAWN.night(), roles.accent).selected
    );
    assert_eq!(p.hover, over(roles.text, glass::fill::ACTIVE, roles.card));
    // The theme the older parts of the window draw with follows the text and the cards too.
    let theme = Theme::for_appearance(
        Appearance::Dark,
        Accent::from_rgb(roles.accent),
        WindowAppearance::Dark,
    )
    .with_night(night)
    .with_solid_plates();
    assert_eq!(theme.look.plate, roles.card);
    assert_eq!(
        theme.ink,
        gpui::Hsla::from(gpui::rgb(roles.text)).opacity(sunrise_launcher_theme::glass::ink::FULL)
    );
}

/// A light theme of the player's is painted from its own colours too, plates included.
#[test]
fn a_players_light_theme_has_its_own_plates() {
    use sunrise_launcher::theme_files::Roles;
    let roles = Roles {
        card: 0xfff0f0,
        background: 0xf0e0e0,
        chrome: 0xe0d0d0,
        text: 0x201010,
        ..Roles::DAWN_LIGHT
    };
    let theme = Theme::for_appearance(
        Appearance::Dark,
        Accent::from_rgb(roles.accent),
        WindowAppearance::Dark,
    )
    .with_night(roles.night())
    .with_solid_plates();
    assert_eq!(theme.look.plate, roles.card);
    assert_eq!(theme.look.plate_alpha, 1.);
    let p = Palette::of(&theme);
    assert_eq!(p.card_color, roles.card);
    assert_eq!(p.text, roles.text);
}

/// The eight themes of the gallery, each painted as the launcher paints it (the light ones in the
/// light look), with the pairs a player reads, measured on the final palette: after the fitting,
/// not on the hexes the theme was defined with.
fn gallery_palettes() -> Vec<(&'static str, Palette)> {
    themes::GALLERY
        .iter()
        .map(|t| {
            let light = themes::is_light(Some(t.id));
            // Chosen as the gallery chooses it, so the theme's own accent comes with it.
            let mut settings = Settings {
                appearance: if light {
                    Appearance::Light
                } else {
                    Appearance::Dark
                },
                ..Settings::default()
            };
            sunrise_launcher::appearance::select_theme(&mut settings, t.id)
                .expect("a gallery theme can be chosen");
            let system = if light {
                WindowAppearance::Light
            } else {
                WindowAppearance::Dark
            };
            (t.id, Palette::of(&from_settings(&settings, system)))
        })
        .collect()
}

/// Every pair of every built-in theme that is text on a surface, at the contrast the palette
/// promises. A failure lists all the pairs that fell short, with their ratios.
#[test]
fn every_gallery_theme_reads() {
    let palettes = gallery_palettes();
    assert_eq!(palettes.len(), 8);
    let mut short = Vec::new();
    for (id, p) in palettes {
        let cards = [p.card_color, p.raised];
        let page = [
            p.background,
            p.card_color,
            p.raised,
            p.field,
            p.hover,
            p.selected,
        ];
        let chrome = [p.chrome, p.chrome_raised];
        let accent = [p.accent];
        let pairs: [(&str, u32, &[u32]); 8] = [
            ("text", p.text, &page[..]),
            ("muted", p.muted, &page[..]),
            ("chrome text", p.chrome_text, &chrome[..]),
            ("chrome muted", p.chrome_muted, &chrome[..]),
            ("accent ink", p.accent_ink, &page[..5]),
            ("success", p.success, &cards[..]),
            ("danger", p.danger, &cards[..]),
            ("Play's label", p.accent_on, &accent[..]),
        ];
        for (role, fg, on) in pairs {
            for &bg in on {
                let ratio = contrast(fg, bg);
                if ratio < TEXT {
                    short.push(format!("{id}: {role} {fg:06x} on {bg:06x} {ratio:.2}"));
                }
            }
        }
    }
    assert!(short.is_empty(), "{}", short.join("\n"));
}

/// White stays the label wherever it reads on the accent; on a pale accent it is dark. The
/// label is not one of the player's colours, so their own themes get the same rule and keep
/// every colour they picked.
#[test]
fn the_label_on_the_accent_is_white_only_where_white_reads() {
    for (id, p) in gallery_palettes() {
        if contrast(0xffffff, p.accent) >= TEXT {
            assert_eq!(p.accent_on, 0xffffff, "{id}");
        } else {
            assert!(luminance(p.accent_on) < 0.1, "{id}: {:06x}", p.accent_on);
        }
    }
    let graphite = gallery_palettes()
        .into_iter()
        .find(|(id, _)| *id == "graphite")
        .expect("graphite is in the gallery")
        .1;
    assert_eq!(graphite.accent, 0xd4d4d8, "the silver stays");
    assert_ne!(
        graphite.accent_on, 0xffffff,
        "white on silver cannot be read"
    );
    // A pale accent a player picked keeps its colour; only the label on it is dark.
    let pale = Palette::of(&look(None, Accent::from_rgb(0xffe39a)));
    assert_eq!(pale.accent, 0xffe39a);
    assert!(contrast(pale.accent_on, pale.accent) >= TEXT);
}
