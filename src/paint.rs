//! The launcher's paint: every colour a screen draws with, as named roles. A screen asks
//! [`palette`] for a role (`palette().text`), never for a hex value or a dark/light branch.
//!
//! The roles are Xmilon's 1.0.x glass material (a theme-tinted plate over the scenery, white
//! text, one accent) under the names of the theme-file contract, for the player's theme and
//! accent. [`begin`] sets the palette once per frame from the settings and the theme in use;
//! [`Palette::of`] computes it. The accent the player (or the theme) picked is never changed:
//! [`Palette::accent`] is that colour, and each role that must read is a separate fit of it, toward
//! white or black (`fit`), so a pale or dark accent keeps its character.
//!
//! Every `u32` is an opaque `0xRRGGBB`, so a field, a menu, a selection or a tooltip reads on
//! its own, whatever is behind it. Only the plates are see-through: [`Palette::card`] and
//! [`Palette::chrome_plate`] take the scene or photo's tint. Automatic opacity guards against
//! the theme's sky and stays solid over a photo. An explicit opacity fits ink against the worst
//! possible backdrop instead. Without scenery the plates stay solid.
use crate::{settings::HomePhoto, theme::Theme};
use gpui::{rgb, Hsla};
use std::cell::Cell;
use sunrise_launcher::backend::Settings;
use sunrise_launcher_theme::contrast;
use sunrise_launcher_theme::glass::{fill, from_oklab, luminance, mix_oklab, oklab, SCRIM};
use sunrise_launcher_theme::night::{over, Night};

/// What a dialog's scrim may dim a bright page by: the opacity is the one in this range that
/// sets the dialog furthest apart from the dimmed page.
const SCRIM_ALPHAS: std::ops::RangeInclusive<u32> = 45..=75;
/// The brightest pages a scrim is judged over (`BRIGHT_BACKDROP`, and white).
const BRIGHT: [u32; 2] = [sunrise_launcher_theme::BRIGHT_BACKDROP, 0xffffff];
/// The glass plates' own opacity, as the mockup's `.hzl-glass` mixes it (`Glass::new`).
const PLATE_BASE: f32 = 0.81;
/// How far Play's colour is pulled toward the theme's own chrome (OKLab), so it sits in the theme
/// instead of shouting over it. The white label's fit comes after, unchanged.
pub(crate) const PLAY_TONE: f32 = 0.3;
/// Text reads at this contrast on its surface; edges, marks and the focus ring at the other.
const TEXT: f32 = 4.5;
const MARK: f32 = 3.;
/// The fully shaded part of the Home backdrop, directly behind the unboxed profile.
pub(crate) const HERO_SHADE_ALPHA: f32 = 0.58;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HeroInk {
    pub text: u32,
    pub warning: u32,
    pub link: u32,
    pub shade: u32,
}

/// What the info colour starts from (a blue), on dark and on light themes, before it is fitted.
const INFO_ON_DARK: u32 = 0x7fb0ff;
const INFO_ON_LIGHT: u32 = 0x395b84;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Palette {
    /// The page, and normal list rows.
    pub background: u32,
    /// The window around the page: the sidebar and the bottom panel (see [`chrome_plate`]).
    ///
    /// [`chrome_plate`]: Palette::chrome_plate
    pub chrome: u32,
    /// Plates and captions (see [`card`](Palette::card) for the see-through one).
    pub card_color: u32,
    /// Menus, dialogs and tooltips.
    pub raised: u32,
    /// Text inputs, search and code wells.
    pub field: u32,
    /// A hovered row or secondary action.
    pub hover: u32,
    /// A selected row or segment, and the text selection. Always with `text`.
    pub selected: u32,
    pub text: u32,
    pub muted: u32,
    /// The edge of an input or a secondary control.
    pub line: u32,
    /// Passive rules between rows.
    pub divider: u32,
    /// A selected or hovered control on the chrome.
    pub chrome_raised: u32,
    pub chrome_line: u32,
    pub chrome_text: u32,
    pub chrome_muted: u32,
    /// Text drawn without a plate over the shaded Home scene.
    pub hero: HeroInk,
    /// The accent as picked: never changed. Marks, active indicators, progress.
    pub accent: u32,
    /// Text and glyphs on the accent: white, always.
    pub accent_on: u32,
    /// Play's two opaque accent stops, both fitted to hold white text.
    play_stops: [u32; 2],
    pub play_glow: u32,
    /// The accent as text, a link or a tick on the surfaces above (4.5:1).
    pub accent_ink: u32,
    /// The keyboard ring, on every surface (3:1).
    pub focus: u32,
    pub danger: u32,
    pub info: u32,
    pub success: u32,
    pub warning: u32,
    /// How opaque the plates are: see the module's note.
    pub alpha: f32,
    /// How opaque the dialogs' scrim is.
    pub scrim_alpha: f32,
}

/// The colours of a picture's stand-in ([`Palette::tile`]): its fill, and the letter on it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Tile {
    pub fill: u32,
    pub ink: u32,
}

/// How far a tile's fill stands from the card (a contrast ratio): enough to be a shape of its
/// own, not enough to shout.
const TILE_APART: f32 = 1.4;
/// How much of its hue a tile's fill carries over the card, and its letter over the text.
const TILE_HUE: f32 = 0.24;
const TILE_INK_HUE: f32 = 0.3;

/// The hue `id` always gets, as a turn (0 to 1): FNV-1a over its bytes, then mixed (the last
/// step of MurmurHash3) so that ids a letter apart land far apart.
fn tile_hue(id: &str) -> f32 {
    let mut hash = id.bytes().fold(0x811c_9dc5_u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });
    hash = (hash ^ hash >> 16).wrapping_mul(0x85eb_ca6b);
    hash = (hash ^ hash >> 13).wrapping_mul(0xc2b2_ae35);
    (hash >> 16) as f32 / 65536.
}

/// The full colour at `turn` of the hue wheel.
fn hue_colour(turn: f32) -> u32 {
    let channel = |shift: f32| {
        let wave = ((turn - shift) * std::f32::consts::TAU).cos();
        ((0.5 + 0.5 * wave) * 255.).round() as u32
    };
    channel(0.) << 16 | channel(1. / 3.) << 8 | channel(2. / 3.)
}

impl Inputs {
    fn of(
        settings: &Settings,
        theme: &Theme,
        photo: Option<&HomePhoto>,
        preview: Option<u8>,
    ) -> Inputs {
        // Backdrop renders either a decoded picture or a scene, even in performance mode;
        // a missing picture falls back to a scene. Only turning scenery off removes both.
        let plates = if !settings.scenery() || settings.reduced_graphics() {
            Plates::Opaque
        } else if let Some(percent) = preview.or(settings.panel_opacity()) {
            Plates::Custom(percent)
        } else if theme.photo {
            Plates::Opaque
        } else {
            Plates::Automatic
        };
        Inputs {
            night: theme.night,
            scene_tone: theme.scene_tone,
            photo_tone: photo.and_then(|p| p.tone),
            accent: theme.accent_choice.rgb(),
            play_seed: theme.play_seed,
            tint: settings.glass_tint(),
            plates,
        }
    }
}

thread_local! {
    static CURRENT: Cell<(Inputs, Palette)> = Cell::new({
        let settings = Settings::default();
        let theme = crate::theme::from_settings(&settings, gpui::WindowAppearance::Dark);
        let inputs = Inputs::of(&settings, &theme, None, None);
        (inputs, inputs.palette())
    });
}

/// The palette of the frame being drawn.
pub(crate) fn palette() -> Palette {
    CURRENT.get().1
}

/// Starts a frame: the palette its screens draw with, computed again only for new inputs.
pub(crate) fn begin(
    settings: &Settings,
    theme: &Theme,
    photo: Option<&HomePhoto>,
    preview: Option<u8>,
) {
    let inputs = Inputs::of(settings, theme, photo, preview);
    if CURRENT.get().0 != inputs {
        CURRENT.set((inputs, inputs.palette()));
    }
}

/// A test for [`fit`]: `c` reads at `min` on each of `on`.
fn reads(on: &[u32], min: f32) -> impl Fn(u32) -> bool + '_ {
    move |c| on.iter().all(|&s| contrast(c, s) >= min)
}

/// `c`, mixed toward white (`lighter`) or black, which keeps its hue, by the least that makes
/// `reads` hold: found by bisection; `c` itself when it already reads.
fn fit(c: u32, lighter: bool, reads: impl Fn(u32) -> bool) -> u32 {
    fit_toward(c, if lighter { 0xffffff } else { 0x000000 }, reads)
}

/// Bring a colour toward a known readable end without going further than the check needs.
fn fit_toward(c: u32, end: u32, reads: impl Fn(u32) -> bool) -> u32 {
    if reads(c) || !reads(end) {
        return if reads(c) { c } else { end };
    }
    let (mut lo, mut hi) = (0., 1.);
    for _ in 0..10 {
        let mid = (lo + hi) / 2.;
        if reads(over(end, mid, c)) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    over(end, hi, c)
}

/// Check contrast at the worst backdrop in the direction that keeps it safe at every pixel.
fn reads_over(ink: f32, plate: f32, lighter: bool, min: f32) -> bool {
    let (light, dark) = if lighter { (ink, plate) } else { (plate, ink) };
    (light + 0.05) / (dark + 0.05) >= min
}

#[derive(Clone, Copy, PartialEq)]
enum Plates {
    Opaque,
    Automatic,
    Custom(u8),
}

/// Everything a palette depends on: it is computed again only when one of these changes.
#[derive(Clone, Copy, PartialEq)]
struct Inputs {
    night: Night,
    /// If neither tone is available, use the theme file's authored plates.
    scene_tone: Option<u32>,
    photo_tone: Option<u32>,
    accent: u32,
    play_seed: Option<u32>,
    tint: f32,
    plates: Plates,
}

impl Inputs {
    fn palette(&self) -> Palette {
        let Inputs {
            night: n,
            scene_tone,
            photo_tone,
            accent: raw,
            play_seed,
            tint,
            plates,
        } = *self;
        // Light text on dark surfaces fits lighter; dark text on light ones fits darker.
        let lighter = n.dark;
        // The theme's own plates with the player's Glass: white (or the light theme's ink)
        // at `tint`.
        let veil = |plate: u32| {
            over(
                n.ink,
                (tint - sunrise_launcher_theme::glass::TINT).max(0.),
                plate,
            )
        };
        let bases = |tone: Option<u32>| {
            if let Some(tone) = tone {
                let [l, a, b] = oklab(tone);
                let chroma = a.hypot(b);
                let levels = if n.dark {
                    [0.30, 0.35, 0.42]
                } else {
                    [0.86, 0.90, 0.94]
                };
                // Keep the source's hue. Scale chroma with lightness, with a minimum to keep a
                // pale sky recognisably tinted, and a maximum so vivid sources aren't loud.
                // Exact neutral RGB has no hue; don't amplify the matrix's rounding noise.
                let neutral = tone == (tone & 255) * 0x01_01_01;
                [0, 1, 2].map(|i| {
                    let level = levels[i];
                    let gain = if neutral {
                        0.
                    } else {
                        let floor = if i == 0 { 0.046 } else { 0.055 };
                        (chroma * level / l.max(0.25)).clamp(floor, 0.10) / chroma
                    };
                    from_oklab([level, a * gain, b * gain])
                })
            } else {
                [n.side, n.card, n.card_hi]
            }
        };
        let [chrome_base, card_base, raised_base] = bases(photo_tone.or(scene_tone));
        let background = n.page;
        let card_color = veil(card_base);
        let raised = veil(raised_base);
        let field = over(n.ink, fill::FIELD, card_color);
        let hover = over(n.ink, fill::ACTIVE, card_color);
        // The selection is the accent washed into the plate, as strong as the text on it allows.
        let selected = (0..14)
            .map(|i| over(raw, 0.30 - 0.02 * i as f32, card_color))
            .find(|&s| contrast(n.ink, s) >= TEXT)
            .unwrap_or(hover);
        let surfaces = [background, card_color, raised, field, hover, selected];
        let chrome = veil(chrome_base);
        let chrome_raised = over(n.ink, fill::ACTIVE, chrome);
        let on_chrome = [chrome, chrome_raised];
        let everywhere = [
            background,
            card_color,
            raised,
            field,
            hover,
            selected,
            chrome,
            chrome_raised,
        ];

        // Solid fills blend encoded sRGB (`over`); white and black bound all possible backdrop
        // pixels, including scenery. Use that bound rather than trusting the picture's wash.
        let backdrop = if lighter { 0xffffff } else { 0 };
        let ink_end = if lighter { 1. } else { 0. };
        let custom = if let Plates::Custom(percent) = plates {
            // An installed local theme can need more opacity than any built-in palette does.
            (percent..=100).find_map(|percent| {
                let alpha = f32::from(percent) / 100.;
                let card = luminance(over(card_color, alpha, backdrop));
                let chrome = luminance(over(chrome, alpha, backdrop));
                let worst = if lighter {
                    card.max(chrome)
                } else {
                    card.min(chrome)
                };
                reads_over(ink_end, worst, lighter, TEXT).then_some((alpha, worst))
            })
        } else {
            None
        };
        let ink = |color, solid: &[u32], min| {
            fit(color, lighter, |c| {
                reads(solid, min)(c)
                    && custom.is_none_or(|(_, plate)| reads_over(luminance(c), plate, lighter, min))
            })
        };
        let text = ink(n.ink, &surfaces, TEXT);
        let muted = ink(n.muted, &surfaces, TEXT);
        let line = ink(over(n.ink, 0.4, card_color), &surfaces[..4], MARK);
        let divider = over(n.stroke, n.rule_alpha, card_color);
        let chrome_text = ink(n.ink, &on_chrome, TEXT);
        let chrome_muted = ink(n.muted, &on_chrome, TEXT);
        let chrome_line = ink(over(n.ink, 0.4, chrome), &on_chrome, MARK);

        // Keep as much of the original control's quarter-white lift as the label allows.
        // On bright accents, shade the accent itself instead of first washing it out with white.
        // Leave room beyond 4.5:1 for renderer rounding.
        let accent_on = 0xffffff;
        let play_text = |c| contrast(accent_on, c) >= 4.65;
        // Play takes the theme's own colour: its accent (or the theme's authored face) is pulled
        // [`PLAY_TONE`] toward the theme's chrome, never a photo's, before the fits below.
        let theme_chrome = veil(bases(scene_tone)[0]);
        let toned = |c: u32| mix_oklab(theme_chrome, c, PLAY_TONE);
        let play_glow = toned(play_seed.unwrap_or(raw));
        let base = fit(play_glow, false, play_text);
        // An authored vivid face already has the right lift. Mixing white into it would
        // put back the dusty colour this shade is meant to avoid.
        let light = if play_seed.is_some() {
            base
        } else {
            fit_toward(mix_oklab(accent_on, toned(raw), 0.25), base, play_text)
        };
        let dark = fit(play_glow, false, |c| {
            contrast(accent_on, c) >= contrast(accent_on, light) + 1.5
        });
        let accent_ink = ink(raw, &surfaces[..5], TEXT);
        let focus = ink(raw, &everywhere, MARK);
        let status = |c: u32| ink(c, &surfaces[..5], TEXT);
        let warning = status(n.warn);

        // On a pale photo, a light feather and dark type leave much more of the picture
        // visible than a dark shadow. Either way fit against the worst sRGB pixel, not the
        // photo's average (individual pixels may have any colour).
        let light_ink = photo_tone.map_or(lighter, |tone| luminance(tone) < 0.54);
        let shade = if light_ink { 0 } else { 0xffffff };
        let worst = luminance(over(shade, HERO_SHADE_ALPHA, 0xffffff - shade));
        let hero_ink = |c| {
            fit(c, light_ink, |c| {
                reads_over(luminance(c), worst, light_ink, TEXT)
            })
        };
        let hero = HeroInk {
            text: hero_ink(text),
            warning: hero_ink(warning),
            link: hero_ink(accent_ink),
            shade,
        };

        // Keep the legacy automatic path unchanged. An unsatisfiable local override stays solid.
        let alpha = if let Some((alpha, _)) = custom {
            alpha
        } else if plates == Plates::Automatic {
            let base = tint + (1. - tint) * PLATE_BASE;
            let holds = |a: f32| {
                n.sky.iter().all(|&sky| {
                    [
                        (text, card_color),
                        (muted, card_color),
                        (chrome_text, chrome),
                        (chrome_muted, chrome),
                    ]
                    .iter()
                    .all(|&(ink, plate)| contrast(ink, over(plate, a, sky)) >= TEXT)
                })
            };
            (0..=100)
                .map(|i| i as f32 / 100.)
                .find(|&a| a >= base && holds(a))
                .unwrap_or(1.)
        } else {
            1.
        };

        let scrim_alpha = SCRIM_ALPHAS
            .map(|i| i as f32 / 100.)
            .max_by(|&a, &b| {
                let apart = |al: f32| {
                    let seen = |page| contrast(raised, over(SCRIM, al, page));
                    seen(BRIGHT[0]).min(seen(BRIGHT[1]))
                };
                apart(a).total_cmp(&apart(b))
            })
            .unwrap_or(0.55);

        Palette {
            background,
            chrome,
            card_color,
            raised,
            field,
            hover,
            selected,
            text,
            muted,
            line,
            divider,
            chrome_raised,
            chrome_line,
            chrome_text,
            chrome_muted,
            hero,
            accent: raw,
            accent_on,
            play_stops: [dark, light],
            play_glow,
            accent_ink,
            focus,
            danger: status(n.danger),
            info: status(if n.dark { INFO_ON_DARK } else { INFO_ON_LIGHT }),
            success: status(n.success),
            warning,
            alpha,
            scrim_alpha,
        }
    }
}

impl Palette {
    /// The palette of `night` under `accent` with opaque plates: a theme's preview, whatever
    /// theme is in use.
    pub(crate) fn plain(night: Night, accent: u32, tint: f32, tone: Option<u32>) -> Palette {
        Inputs {
            night,
            scene_tone: tone,
            photo_tone: None,
            accent,
            play_seed: None,
            tint,
            plates: Plates::Opaque,
        }
        .palette()
    }

    /// The palette `begin` would set for these settings and this theme.
    #[cfg(test)]
    pub(crate) fn of(settings: &Settings, theme: &Theme) -> Palette {
        Inputs::of(settings, theme, None, None).palette()
    }

    /// What stands in for the picture of a mod or pack that has none: a quiet colour of its own
    /// from `id`, the same every time, under its first letter. The fill is the card washed with
    /// the id's hue and set [`TILE_APART`] from the card, lighter on a dark theme and darker
    /// on a light one, so every tile of a theme weighs the same; the letter is the text washed
    /// with the same hue, and reads on it (4.5:1).
    pub(crate) fn tile(&self, id: &str) -> Tile {
        let hue = hue_colour(tile_hue(id));
        let card = self.card_color;
        let lighter = luminance(self.text) > luminance(card);
        // The one luminance every tile of this theme has. A card too bright (or too dark) to
        // step away from in its own direction is stepped away from in the other.
        let (up, down) = (
            (luminance(card) + 0.05) * TILE_APART - 0.05,
            (luminance(card) + 0.05) / TILE_APART - 0.05,
        );
        let level = match lighter {
            true if up <= 1. => up,
            false if down < 0. => up,
            _ => down,
        };
        // Keep the card's lightness, but not its hue: a contrasting tile hue can otherwise
        // cancel the scene tint out and turn the stand-in gray.
        let neutral = from_oklab([oklab(card)[0], 0., 0.]);
        let washed = over(hue, TILE_HUE, neutral);
        let brighten = luminance(washed) < level;
        let fill = fit(washed, brighten, |c| (luminance(c) >= level) == brighten);
        let reads = |c: u32| contrast(c, fill) >= TEXT;
        let letter = over(hue, TILE_INK_HUE, self.text);
        let ink = [lighter, !lighter]
            .into_iter()
            .map(|l| fit(letter, l, reads))
            .find(|&c| reads(c))
            .unwrap_or(self.text);
        Tile { fill, ink }
    }

    /// A card, caption or plate: the theme's tint at the plates' opacity.
    pub(crate) fn card(&self) -> Hsla {
        Hsla::from(rgb(self.card_color)).opacity(self.alpha)
    }

    /// The material the sidebar and the bottom panel share.
    pub(crate) fn chrome_plate(&self) -> Hsla {
        Hsla::from(rgb(self.chrome)).opacity(self.alpha)
    }

    /// A text input's fill, and the account chip's: the field colour, part see-through, so
    /// the glass behind it shows.
    pub(crate) fn input_fill(&self) -> Hsla {
        Hsla::from(rgb(self.field)).opacity(0.55)
    }

    /// Play's opaque accent gradient. Linear sRGB interpolation cannot produce luminance above
    /// the brighter stop, so the white label holds between the stops as well as on them.
    pub(crate) fn play_gradient(&self) -> gpui::Background {
        let [dark, light] = self.play_stops.map(rgb);
        gpui::linear_gradient(
            135.,
            gpui::linear_color_stop(light, 0.),
            gpui::linear_color_stop(dark, 1.),
        )
        .color_space(gpui::ColorSpace::Srgb)
    }

    /// A resting input's outline: faint. The focus ring, not this line, marks the one in use.
    pub(crate) fn input_edge(&self) -> Hsla {
        Hsla::from(rgb(self.text)).opacity(0.14)
    }

    /// What a dialog dims the launcher with.
    pub(crate) fn scrim(&self) -> Hsla {
        Hsla::from(rgb(SCRIM)).opacity(self.scrim_alpha)
    }
}

#[cfg(test)]
mod tests {
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
            assert_eq!((p.accent, p.accent_on), (0xb4552f, 0xffffff));
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
        assert_eq!(shown.hero.shade, 0xffffff, "shade a pale photo lightly");
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
                    for percent in [PANEL_OPACITY_MIN, PANEL_OPACITY_MAX] {
                        settings.set_panel_opacity(Some(percent));
                        for tone in std::iter::once(None).chain(photo_tones.map(Some)) {
                            photo.tone = tone;
                            let shown = tone.map(|_| &photo);
                            let p = Inputs::of(&settings, &theme, shown, None).palette();
                            let name = format!("{id}, {accent:?}, photo {tone:06x?}, Glass {tint}, opacity {percent}%");
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
                            for behind in [0, 0xffffff, 0x808080].into_iter().chain(theme.night.sky)
                            {
                                for plate in [p.card_color, p.chrome] {
                                    let surface = over(plate, p.alpha, behind);
                                    for (ink, role) in [
                                        (p.text, "body"),
                                        (p.muted, "muted"),
                                        (p.accent_ink, "accent"),
                                    ] {
                                        check(surface, ink, TEXT, role);
                                    }
                                    for (ink, role) in
                                        [(p.focus, "focus"), (p.line, "control edge")]
                                    {
                                        check(surface, ink, MARK, role);
                                    }
                                }
                                let chrome = over(p.chrome, p.alpha, behind);
                                check(chrome, p.chrome_text, TEXT, "chrome body");
                                check(chrome, p.chrome_muted, TEXT, "chrome muted");
                                check(chrome, p.chrome_line, MARK, "chrome edge");
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
        assert_eq!(light.hero.shade, 0xffffff);
        for p in themes.chain([light]) {
            for pixel in [0, 0xffffff] {
                let behind = over(p.hero.shade, HERO_SHADE_ALPHA, pixel);
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
}
