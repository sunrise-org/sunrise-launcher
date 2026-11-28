//! The launcher's view of Sunrise's design tokens (`crates/theme`, described in
//! `docs/design-system.md`): the night look (`sunrise_launcher_theme::night`, a night sky or a dawn with
//! Sunrise's violet) in Bricolage Grotesque, with Geist Mono for paths and codes (all bundled, SIL
//! Open Font License). Screens not rebuilt yet draw the Blocks parts in the night's colours.
use gpui::{rgb, Hsla, WindowAppearance};
use sunrise_launcher::backend::{Appearance, Settings};
use sunrise_launcher::themes::{self, Accent};
use sunrise_launcher_theme as tokens;

pub use tokens::night::{self, Night};
#[cfg_attr(not(windows), allow(unused_imports))] // the Windows setup window
pub use tokens::MICRO;
pub use tokens::UI;
pub const MONO: &str = tokens::MONO;

/// Shapes and controls, shared with the game's menus and the website.
pub use tokens::{DISABLED, PRESSED};

/// Blocks have square corners: this stays for the places that ask for a radius.
pub const RADIUS_PLATE: f32 = 0.;
pub const PAGE_GUTTER: f32 = 32.;

#[derive(Clone, Copy)]
pub struct Theme {
    /// The glass look's colours for the accent (the dark appearance draws it).
    pub look: tokens::glass::Glass,
    /// The page behind everything the sky doesn't cover.
    pub page: Hsla,
    /// Hover plates, fields and the console: the raised panel colour.
    pub plate_soft: Hsla,
    /// Pressed plates, a switch that is off: the button colour.
    pub plate: Hsla,
    pub rule: Hsla,
    pub ink: Hsla,
    pub ink_soft: Hsla,
    pub muted: Hsla,
    pub danger: Hsla,
    /// The blocky parts (`sunrise_launcher_theme::Blocks`).
    pub raised: Hsla,
    pub well: Hsla,
    /// The night look's own tokens (the sky, plates with their opacity), for rebuilt screens.
    pub night: Night,
    /// A stronger version of the default accent for the primary Home action, if authored.
    pub(crate) play_seed: Option<u32>,
    /// The colours are the player's own: their text colours are painted exactly as picked.
    pub(crate) own_colours: bool,
    /// The accent this look was built from, before the glass look's mixes.
    pub accent_choice: Accent,
    pub dark: bool,
    /// Whether the pages draw the redesign's glass look: both appearances do (light with ink
    /// for white and light plates).
    pub glass: bool,
}

/// The light appearance's look: the soft accent (links, ticks, the sliders) is the accent
/// itself, which reads on white where the dark look's half-white mix would not. An accent too
/// light for that (Hypr Neon's lavender, a pale typed colour) is darkened until it reads.
fn light_look(look: tokens::glass::Glass) -> tokens::glass::Glass {
    let mut look = tokens::glass::Glass::new(readable_on_white(look.accent), look.tint, look.blur);
    look.accent_soft = look.accent;
    look
}

/// `c`, darkened in small OKLab steps until it reads at 4.5:1 on white; unchanged if it does.
fn readable_on_white(mut c: u32) -> u32 {
    for _ in 0..60 {
        if tokens::contrast(c, 0xffffff) >= 4.5 {
            break;
        }
        c = tokens::glass::mix_oklab(c, 0x000000, 0.95);
    }
    c
}

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

/// Whether the launcher is pinned dark (`Some(true)`) or light, or follows the system (`None`):
/// `SUNRISE_LAUNCHER_APPEARANCE=light` or `dark` (for screenshots and tests), else the player's
/// choice in Settings.
pub fn pinned_dark(choice: Appearance) -> Option<bool> {
    match std::env::var("SUNRISE_LAUNCHER_APPEARANCE").as_deref() {
        Ok("light") => Some(false),
        Ok("dark") => Some(true),
        _ => match choice {
            Appearance::Dark => Some(true),
            Appearance::Light => Some(false),
            Appearance::System => None,
        },
    }
}

/// Whether the look is dark: the appearance pinned, or the system's when it is followed.
fn is_dark(choice: Appearance, system: WindowAppearance) -> bool {
    let system_dark = !matches!(
        system,
        WindowAppearance::Light | WindowAppearance::VibrantLight
    );
    pinned_dark(choice).unwrap_or(system_dark)
}

/// The look Settings ask for: the accent and launcher theme, A
/// theme is one look, except Sunrise: its night in the dark look, its day (`DAWN_LIGHT`) in the
/// light one. Other themes stay dark in both.
pub fn from_settings(settings: &Settings, system: WindowAppearance) -> Theme {
    // What is painted, which is not always what is saved: a player's theme that is gone is
    // Dawn until it is back, and one without an accent picked under it wears its file's.
    let accent = sunrise_launcher::appearance::accent(settings);
    let dark = is_dark(settings.appearance, system);
    let mut theme = Theme::for_appearance(Appearance::Dark, accent, system);
    theme = match themes::resolve(settings.theme.as_deref()) {
        themes::Resolved::Twilight => theme,
        themes::Resolved::Builtin(picked) => theme.with_launcher_theme(picked, dark),
        themes::Resolved::Local(local) => Theme {
            own_colours: true,
            ..theme.with_night(local.roles.night())
        },
        themes::Resolved::Missing => theme.with_launcher_theme(&themes::DAWN_THEME, dark),
    };
    // Reduced graphics keeps the sky's colours and drops its decorative effects.
    if settings.reduced_graphics() {
        theme.night.art.mood = night::Mood::Plain;
    }
    theme.with_solid_plates()
}

impl Theme {
    /// The pinned look (`pinned_dark`: dark unless the player picked another), the system's when
    /// they asked to follow it, in the player's accent.
    /// A launcher theme from Settings: its sky, or its whole palette when it brings one (in the
    /// light look, its light palette if it has one).
    pub fn with_launcher_theme(mut self, picked: &themes::Theme, dark: bool) -> Self {
        if let Some(night) = picked.palette_in(dark) {
            self = self.with_night(*night);
        } else {
            self.night.sky = picked.sky;
        }
        self.play_seed = match picked.id {
            "cherry_blossom" if self.accent_choice == picked.accent.accent() => Some(0xc8297e),
            "nord" if self.accent_choice == picked.accent.accent() => Some(0x087d95),
            _ => None,
        };
        self
    }

    /// Sunrise's own pages in `night`'s colours: a theme never changes how a page is laid out
    /// or works.
    pub fn with_night(self, night: Night) -> Self {
        let accent = self.accent_choice;
        Self::from(accent.apply(night.blocks()), night, accent).glass(accent)
    }

    /// The dark look's plates as the theme's own solid card: nothing stands behind the pages to
    /// show through them.
    pub fn with_solid_plates(mut self) -> Self {
        self.look.plate = self.night.card;
        self.look.plate_alpha = 1.;
        self
    }

    pub fn for_appearance(choice: Appearance, accent: Accent, system: WindowAppearance) -> Self {
        let dark = is_dark(choice, system);
        let night = if dark { night::NIGHT } else { night::DAWN };
        Self::from(accent.apply(night.blocks()), night, accent).glass(accent)
    }

    /// The dark appearance draws the glass look ("Glass" in `docs/design-system.md`): what the
    /// pages not rebuilt for it yet draw with these tokens comes out as the mockup's parts do,
    /// white text and fills at a few opacities, no borders, no bevels.
    fn glass(self, accent: Accent) -> Self {
        use sunrise_launcher_theme::glass::{fill, ink, Glass, BLUR, DANGER, TINT};
        let dark = self.dark;
        // The theme's own text colour, so a player's theme reaches these too.
        let ink_rgb = self.night.ink;
        let white = |a: f32| c(ink_rgb).opacity(a);
        let look = Glass::new(accent.rgb(), TINT, BLUR);
        let look = if dark { look } else { light_look(look) };
        Self {
            plate_soft: white(fill::ROW_HOVER),
            plate: white(fill::BUTTON),
            rule: white(fill::BUTTON),
            ink: white(ink::FULL),
            ink_soft: white(ink::SOFT),
            muted: white(ink::QUIET),
            // Light: `glass_ui::danger`, `live` and `warn`'s darker shades.
            danger: c(if dark { DANGER } else { 0xdc2626 }),
            raised: white(fill::ROW_HOVER),
            well: white(fill::FIELD),
            look,
            accent_choice: accent,
            glass: true,
            ..self
        }
    }

    fn from(p: tokens::Blocks, night: Night, accent: Accent) -> Self {
        Self {
            look: tokens::glass::Glass::default_look(),
            page: c(p.page),
            plate_soft: c(p.raised),
            plate: c(p.button),
            rule: c(p.rule),
            ink: c(p.ink),
            ink_soft: c(p.ink_soft),
            muted: c(p.muted),
            danger: c(p.danger),
            raised: c(p.raised),
            well: c(p.well),
            night,
            play_seed: None,
            own_colours: false,
            accent_choice: accent,
            dark: p.dark,
            glass: false,
        }
    }
}

/// How strongly a selected suggestion in a text field is tinted with the accent.
#[cfg(test)]
pub const SELECTION: f32 = 0.22;

#[cfg(test)]
mod tests {
    use super::{from_settings, tokens, SELECTION};
    use gpui::WindowAppearance;
    use sunrise_launcher::backend::{Appearance, Settings};
    use sunrise_launcher::themes::Accent;

    /// Nord draws Sunrise's pages in its own polar-night palette with the frost accent.
    #[test]
    fn nord_draws_sunrises_pages_in_its_colours() {
        let settings = Settings {
            appearance: Appearance::Dark,
            accent: Accent::from_rgb(0x88c0d0),
            theme: Some("nord".into()),
            ..Settings::default()
        };
        let theme = from_settings(&settings, WindowAppearance::Dark);
        assert!(theme.glass, "Nord draws Sunrise's pages");
        assert!(theme.dark);
        assert_eq!(theme.night.page, 0x2e3440);
        assert_eq!(theme.night.accent, 0x88c0d0);
        assert_eq!(theme.accent_choice.rgb(), 0x88c0d0);
    }

    /// Every theme keeps Sunrise's pages (so no page changes how it works or what is behind it)
    /// in its one look; only the colours differ, Hypr's page true black.
    #[test]
    fn the_new_families_keep_the_glass_pages() {
        for (id, black, dark) in [
            ("dawn", false, true),
            ("hypr_neon", true, true),
            ("overworld", false, true),
            ("taiga", false, true),
        ] {
            let settings = Settings {
                accent: sunrise_launcher::themes::starting_accent(Some(id), &Default::default()),
                theme: Some(id.into()),
                ..Settings::default()
            };
            let theme = from_settings(&settings, WindowAppearance::Light);
            assert!(theme.glass, "{id}: glass pages");
            assert_eq!(theme.dark, dark, "{id}: one look");
            assert_eq!(
                theme.look.plate, theme.night.card,
                "{id}: its own solid plates"
            );
            if black {
                assert_eq!(theme.night.page, 0x000000);
            }
            // Text on the main action, and the light look's accent on white.
            let label = tokens::contrast(theme.look.on_accent, theme.look.accent);
            assert!(label >= 4.5, "{id}: label {label:.2}");
            if !dark {
                let soft = tokens::contrast(theme.look.accent_soft, 0xffffff);
                assert!(soft >= 4.5, "{id}: accent on white {soft:.2}");
                // Dark text on a frosted plate over the sky's brightest band.
                let worst = tokens::night::over(
                    theme.look.plate,
                    theme.look.plate_alpha,
                    theme.night.sky[5],
                );
                let ratio = tokens::contrast(theme.night.ink, worst);
                assert!(ratio >= 4.5, "{id}: ink on a plate {ratio:.2}");
            }
        }
    }

    /// Dark content plates over the sunset are mostly opaque, darker than the text they hold.
    #[test]
    fn dark_plates_are_steady() {
        let theme = from_settings(&Settings::default(), WindowAppearance::Dark);
        assert!(theme.look.plate_alpha >= 0.6, "{}", theme.look.plate_alpha);
        // White text over the plate at its alpha over the sun's brightest colour still reads.
        let over = |top: u32, a: f32, under: u32| tokens::night::over(top, a, under);
        let worst = over(theme.look.plate, theme.look.plate_alpha, 0xffd98c);
        let ratio = tokens::contrast(0xffffff, worst);
        assert!(ratio >= 4.5, "white on a plate over the sun: {ratio:.2}");
        // Body text is white at 70 %: it reads at 4.5:1 there too, at the default Glass and Blur 0.
        let body = over(0xffffff, tokens::glass::ink::BODY, worst);
        let ratio = tokens::contrast(body, worst);
        assert!(
            ratio >= 4.5,
            "body text on a plate over the sun: {ratio:.2}"
        );
    }

    /// The light look keeps Sunrise's violet as it is and darkens only accents too pale for white.
    #[test]
    fn the_light_accent_reads_on_white() {
        assert_eq!(super::readable_on_white(0x6b4bff), 0x6b4bff);
        let pale = super::readable_on_white(0xcba6f7);
        assert!(tokens::contrast(pale, 0xffffff) >= 4.5, "{pale:06x}");
    }

    /// `top` at `alpha` over `under`, as the GPU blends them.
    fn blend(top: u32, under: u32, alpha: f32) -> u32 {
        let channel = |shift: u32| {
            let (a, b) = ((top >> shift & 0xff) as f32, (under >> shift & 0xff) as f32);
            ((a * alpha + b * (1. - alpha)).round() as u32) << shift
        };
        channel(16) | channel(8) | channel(0)
    }

    /// What the Servers page draws besides text (whose colours sunrise-theme's own test holds to
    /// WCAG AA on every background): the keyboard focus ring, the borders of a focused or refused
    /// field (WCAG 1.4.11 asks 3:1 for those), and text over a selected suggestion.
    #[test]
    fn marks_and_selections_read_clearly() {
        for p in [tokens::night::DAWN.blocks(), tokens::night::NIGHT.blocks()] {
            let theme = if p.dark { "dark" } else { "light" };
            for (name, bg) in [
                ("page", p.page),
                ("raised", p.raised),
                ("button", p.button),
                ("panel", p.panel),
                ("well", p.well),
            ] {
                let ring = tokens::contrast(p.accent, bg);
                assert!(ring >= 3.0, "{theme} focus ring on {name}: {ring:.2}");
                let refused = tokens::contrast(p.danger, bg);
                assert!(
                    refused >= 3.0,
                    "{theme} refused field on {name}: {refused:.2}"
                );
            }
            let selection = blend(p.accent, p.panel, SELECTION);
            let ratio = tokens::contrast(p.ink, selection);
            assert!(ratio >= 4.5, "{theme} text on a selection: {ratio:.2}");
        }
    }
}
