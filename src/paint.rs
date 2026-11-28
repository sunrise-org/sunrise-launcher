//! The launcher's paint: every colour a screen draws with, as named roles. A screen asks
//! [`palette`] for a role (`palette().text`), never for a hex value or a dark/light branch.
//!
//! The roles are the redesign's flat material (a solid chrome, one solid page panel, opaque
//! cards) under the names of the theme-file contract, for the player's theme and accent. [`begin`]
//! sets the palette once per frame from the theme in use; [`Palette::of`] computes it. The accent
//! the player (or the theme) picked is never changed: [`Palette::accent`] is that colour, and each
//! role that must read is a separate fit of it, toward white or black (`fit`), so a pale or dark
//! accent keeps its character.
//!
//! Every `u32` is an opaque `0xRRGGBB`, so a field, a menu, a selection or a tooltip reads on its
//! own. Nothing the palette names is see-through: the scenery stands only in the Home banner.
use crate::theme::Theme;
use gpui::RenderImage;
use gpui::{rgb, Hsla};
use std::cell::Cell;
use std::sync::Arc;
use sunrise_launcher::backend::Settings;
use sunrise_launcher_theme::contrast;
use sunrise_launcher_theme::glass::{fill, from_oklab, luminance, mix_oklab, oklab, SCRIM};
use sunrise_launcher_theme::night::{over, Night};

/// What stands behind Home, as the frame's backdrop draws it: the player's own picture or
/// their theme's, decoded to one still picture off the window's thread. The window is never
/// handed a file to decode itself.
#[derive(Clone)]
pub(crate) struct HomePhoto {
    pub image: Arc<RenderImage>,
}

/// What a dialog's scrim may dim a bright page by: the opacity is the one in this range that
/// sets the dialog furthest apart from the dimmed page.
const SCRIM_ALPHAS: std::ops::RangeInclusive<u32> = 45..=75;
/// The brightest pages a scrim is judged over (`BRIGHT_BACKDROP`, and white).
const BRIGHT: [u32; 2] = [sunrise_launcher_theme::BRIGHT_BACKDROP, 0xffffff];
/// How far Play's colour is pulled toward the theme's own chrome (OKLab), so it sits in the theme
/// instead of shouting over it. The white label's fit comes after, unchanged.
pub(crate) const PLAY_TONE: f32 = 0.15;
/// Text reads at this contrast on its surface; edges, marks and the focus ring at the other.
const TEXT: f32 = 4.5;
const MARK: f32 = 3.;
/// The least the Home backdrop is shaded directly behind the unboxed profile.
pub(crate) const HERO_SHADE_ALPHA: f32 = 0.58;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HeroInk {
    pub text: u32,
    pub warning: u32,
    pub link: u32,
    /// The theme's chrome where it can hold the ink, so the shade reads as the panels' material
    /// and not a black (or white) patch; else black or white.
    pub shade: u32,
    /// How strongly `shade` lies behind the text, from [`HERO_SHADE_ALPHA`] up.
    pub shade_alpha: f32,
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
    /// Cards and captions.
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
    /// The window is flat (docs/redesign.md): the scenery stands only in the profile banner, so
    /// every surface is solid and in the theme's own colours, never tinted by the scene.
    fn of(theme: &Theme) -> Inputs {
        Inputs {
            night: theme.night,
            accent: theme.accent_choice.rgb(),
            play_seed: theme.play_seed,
        }
    }
}

thread_local! {
    static CURRENT: Cell<(Inputs, Palette)> = Cell::new({
        let settings = Settings::default();
        let theme = crate::theme::from_settings(&settings, gpui::WindowAppearance::Dark);
        let inputs = Inputs::of(&theme);
        (inputs, inputs.palette())
    });
}

/// The palette of the frame being drawn.
pub(crate) fn palette() -> Palette {
    CURRENT.get().1
}

/// Starts a frame: the palette its screens draw with, computed again only for new inputs.
pub(crate) fn begin(theme: &Theme) {
    let inputs = Inputs::of(theme);
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

/// Everything a palette depends on: it is computed again only when one of these changes.
#[derive(Clone, Copy, PartialEq)]
struct Inputs {
    night: Night,
    accent: u32,
    play_seed: Option<u32>,
}

impl Inputs {
    fn palette(&self) -> Palette {
        let Inputs {
            night: n,
            accent: raw,
            play_seed,
        } = *self;
        // Light text on dark surfaces fits lighter; dark text on light ones fits darker.
        let lighter = n.dark;
        let [chrome_base, card_base, raised_base] = [n.side, n.card, n.card_hi];
        let background = n.page;
        let card_color = card_base;
        let raised = raised_base;
        let field = over(n.ink, fill::FIELD, card_color);
        let hover = over(n.ink, fill::ACTIVE, card_color);
        // The selection is the raised surface (Figma bg/selected), as long as the text reads on
        // it; else the hover's lift.
        let selected = Some(raised)
            .filter(|&s| contrast(n.ink, s) >= TEXT)
            .unwrap_or(hover);
        let surfaces = [background, card_color, raised, field, hover, selected];
        let chrome = chrome_base;
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

        let ink = |color, solid: &[u32], min| fit(color, lighter, reads(solid, min));
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
        let theme_chrome = chrome_base;
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

        // The banner's art may have any colour under the text, so fit against the worst sRGB
        // pixel: the ink's own end.
        let light_ink = lighter;
        // The worst pixel is the ink's own end: white behind light text, black behind dark.
        let end: u32 = if light_ink { 0xffffff } else { 0 };
        let holds = |shade: u32| {
            (0..=21)
                .map(|step| (HERO_SHADE_ALPHA + step as f32 * 0.02).min(1.))
                .find(|&a| {
                    reads_over(
                        luminance(end),
                        luminance(over(shade, a, end)),
                        light_ink,
                        TEXT + 0.15,
                    )
                })
                .map(|a| (shade, a))
        };
        let (shade, shade_alpha) = holds(chrome).unwrap_or((0xffffff - end, HERO_SHADE_ALPHA));
        let worst = luminance(over(shade, shade_alpha, end));
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
            shade_alpha,
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
            scrim_alpha,
        }
    }
}

impl Palette {
    /// The palette of `night` under `accent`: a theme's preview, whatever theme is in use.
    #[cfg(test)]
    pub(crate) fn plain(night: Night, accent: u32) -> Palette {
        Inputs {
            night,
            accent,
            play_seed: None,
        }
        .palette()
    }

    /// The palette `begin` would set for these settings and this theme.
    #[cfg(test)]
    pub(crate) fn of(theme: &Theme) -> Palette {
        Inputs::of(theme).palette()
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

    /// A rule between rows on a card, or a card's outline: the divider.
    pub(crate) fn rule(&self) -> Hsla {
        Hsla::from(rgb(self.divider))
    }

    /// A card or caption.
    pub(crate) fn card(&self) -> Hsla {
        Hsla::from(rgb(self.card_color))
    }

    /// A text input's fill, and the account chip's: the field colour, part see-through, so
    /// the glass behind it shows.
    pub(crate) fn input_fill(&self) -> Hsla {
        Hsla::from(rgb(self.field)).opacity(0.55)
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
mod tests;
