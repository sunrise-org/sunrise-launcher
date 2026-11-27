//! The player's own look, on top of Dark, Light or System: an accent colour and a picture behind
//! Home.
//!
//! An accent is a hue, and the player may pick any of them; seven hues have names, as shortcuts.
//! Sunrise's violet is the default. Another hue turns the violet tokens (`sunrise_launcher_theme::Blocks`'
//! accent and the main action's face, bevel and lip) to it, keeping their saturation and
//! lightness, then darkens or lightens each just enough to read as well as the violet does: white
//! on the face at 4.5:1, the accent at 3:1 on every background.
use std::path::{Path, PathBuf};
use sunrise_launcher_theme::night::{Land, Mood, Night, SkyArt};
use sunrise_launcher_theme::{contrast, Blocks};

/// An accent: a hue in degrees, or none for Sunrise's violet, whose tokens are the palette's own
/// and stay untouched; and for a colour typed as hex, that colour (the glass look wears it as
/// is, the light look takes its hue).
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Accent {
    hue: Option<f32>,
    rgb: Option<u32>,
}

/// The accent a launcher theme brings: a hue in degrees (First Light) or one exact colour (Nord's
/// frost).
#[derive(Clone, Copy, Debug)]
pub enum ThemeAccent {
    Hue(f32),
    Colour(u32),
}

impl ThemeAccent {
    pub fn accent(self) -> Accent {
        match self {
            Self::Hue(hue) => Accent::custom(hue),
            Self::Colour(rgb) => Accent::from_rgb(rgb),
        }
    }
}

/// A launcher theme: free for everyone in Settings › Appearance, never sold or claimed. Each
/// has a sky (six stops, top to bottom) and the accent it brings. A theme that brings a whole
/// palette carries its one [`Night`] (there is no light and dark of a theme) and paints Sunrise's
/// own pages with it; the other themes only change the sky.
pub struct Theme {
    /// Its name in `Settings::theme`.
    pub id: &'static str,
    /// Its name in Settings, in English (through `tr`).
    pub name: &'static str,
    /// A short line under its name in Settings, in English (through `tr`; kept short, as it is
    /// translated into every language).
    pub description: &'static str,
    pub sky: [u32; 6],
    pub accent: ThemeAccent,
    /// The theme's own palette, for a theme that is more than a sky: every theme is one look.
    pub palette: Option<&'static Night>,
}

/// Nord's polar night and snow storm, with the frost accent.
pub const NORD_DARK: Night = Night {
    sky: [0x2e3440, 0x2e3440, 0x313845, 0x353d4b, 0x3b4252, 0x434c5e],
    page: 0x2e3440,
    side: 0x292e39,
    card: 0x3b4252,
    card_alpha: 0.88,
    card_hi: 0x434c5e,
    card_hi_alpha: 0.92,
    stroke: 0xd8dee9,
    stroke_alpha: 0.10,
    rule_alpha: 0.10,
    ink: 0xeceff4,
    ink_soft: 0xd8dee9,
    muted: 0xb6bfca,
    accent: 0x88c0d0,
    accent_hi: 0x9fd3d3,
    on_accent: 0x2e3440,
    success: 0xafc99a,
    warn: 0xebcb8b,
    danger: 0xeeb3b8,
    dark: true,
    art: SkyArt {
        sun: [0x8fbcbb, 0x88c0d0, 0x5e81ac],
        far: 0x3b4252,
        near: 0x272c36,
        cloud: [0x3b4252, 0x434c5e, 0x4c566a],
        lit: [0x4c566a, 0x5e6a7d, 0x81a1c1],
        shade: 0x272c36,
        star: 0xeceff4,
        mood: Mood::Plain,
        land: Land::Plain,
    },
};

/// Old School's sky: the night tokens under the sky just before sunrise.
pub const OLD_SCHOOL_NIGHT: Night = Night {
    sky: [0x0b1030, 0x18225a, 0x232c66, 0x6f6ab4, 0xe2a0b0, 0xffd6a0],
    ..sunrise_launcher_theme::night::NIGHT
};

/// Old School (once First Light, the launcher's first theme; its id is kept for saved settings):
/// the sky just before sunrise, with a harbour skyline, painted as it was rather than the sunset.
pub const FIRST_LIGHT: Theme = Theme {
    id: "first_light",
    name: "Old School",
    description: "The launcher's first sky",
    sky: OLD_SCHOOL_NIGHT.sky,
    accent: ThemeAccent::Hue(246.),
    palette: Some(&OLD_SCHOOL_NIGHT),
};

/// Nord: the Nord palette's polar night, snow storm and frost.
pub const NORD: Theme = Theme {
    id: "nord",
    name: "Nord",
    description: "Polar night and frost",
    sky: NORD_DARK.sky,
    accent: ThemeAccent::Colour(0x88c0d0),
    palette: Some(&NORD_DARK),
};

/// Dawn's night: the Sunrise website's Morning palette (sunny `frontend/site/styles/globals.css`): a
/// warm charcoal room, cream light, peach and amber from the sun, the burnt-orange brand on its
/// buttons. A warm dusk sky, never magenta or saturated red.
pub const DAWN_DARK: Night = Night {
    sky: [0x1f1a1c, 0x2c2427, 0x3b2f2e, 0x5a4034, 0x8f5e3e, 0xd9935a],
    page: 0x2b2324,
    side: 0x1e1b1a,
    card: 0x322829,
    card_alpha: 0.88,
    card_hi: 0x3a2f30,
    card_hi_alpha: 0.92,
    stroke: 0xf7c9a2,
    stroke_alpha: 0.12,
    rule_alpha: 0.10,
    ink: 0xf3eae6,
    ink_soft: 0xd8cac4,
    muted: 0xb9aba6,
    accent: 0xc2552d,
    accent_hi: 0xf2bc96,
    on_accent: 0xffffff,
    success: 0x7dc98c,
    warn: 0xefc56e,
    danger: 0xec8d8d,
    dark: true,
    art: SkyArt {
        sun: [0xfff1d6, 0xf7c9a2, 0xf2a65e],
        far: 0x43373a,
        near: 0x221c1e,
        cloud: [0x3b3134, 0x6b4a3b, 0xc98a5e],
        lit: [0x57484b, 0x9a6646, 0xf4b98c],
        shade: 0x2c2427,
        star: 0xfbf1e6,
        mood: Mood::Plain,
        land: Land::Plain,
    },
};

/// Hypr Neon's night: true black over a magenta sunrise, a cyan accent and magenta edges, neon
/// lines along the skylines and a grid floor (after the player's Hypr OLED NEON Discord theme;
/// its colours only, nothing of its CSS).
pub const HYPR_DARK: Night = Night {
    sky: [0x000000, 0x010007, 0x04020f, 0x0b041c, 0x1c0630, 0x3a0a48],
    page: 0x000000,
    side: 0x000000,
    card: 0x07060d,
    card_alpha: 0.96,
    card_hi: 0x110f1c,
    card_hi_alpha: 0.97,
    stroke: 0xff4fd8,
    stroke_alpha: 0.24,
    rule_alpha: 0.12,
    ink: 0xe9f6ff,
    ink_soft: 0xc6d6ea,
    muted: 0x97a6c2,
    accent: 0x3fe0ff,
    accent_hi: 0x7cecff,
    on_accent: 0x02141c,
    success: 0xa6e3a1,
    warn: 0xf9e2af,
    danger: 0xff8fb0,
    dark: true,
    art: SkyArt {
        sun: [0xff4fd8, 0xff7aa8, 0xffb36b],
        far: 0x0b0618,
        near: 0x030209,
        cloud: [0x0b0618, 0x120926, 0x1a0c34],
        lit: [0x2a0f46, 0x3a125c, 0x1c4a66],
        shade: 0x06040d,
        star: 0xbff6ff,
        mood: Mood::Neon([0xff4fd8, 0x3fe0ff]),
        land: Land::Neon([0xb04cff, 0x3fe0ff]),
    },
};

/// Sunrise: the look of playsunrise.net, its warm charcoal and brand orange, and the theme a
/// launcher with none saved wears. Its id is `dawn`, as it was saved before it took the name.
pub const DAWN_THEME: Theme = Theme {
    id: "dawn",
    name: "Sunrise",
    description: "Warm charcoal and orange",
    sky: DAWN_DARK.sky,
    accent: ThemeAccent::Colour(0xc2552d),
    palette: Some(&DAWN_DARK),
};

/// Hypr Neon: OLED black with neon cyan and magenta, on Sunrise's pages with solid plates.
pub const HYPR_NEON: Theme = Theme {
    id: "hypr_neon",
    name: "Hypr Neon",
    description: "Neon on pure black",
    sky: HYPR_DARK.sky,
    accent: ThemeAccent::Colour(0x3fe0ff),
    palette: Some(&HYPR_DARK),
};

/// Deep Ocean's night: teal water under light shafts and rising bubbles.
pub const OCEAN_DARK: Night = Night {
    sky: [0x031a24, 0x052a38, 0x083c4e, 0x0b5468, 0x10708a, 0x2a9bb0],
    page: 0x041c26,
    side: 0x03141c,
    card: 0x07242f,
    card_alpha: 0.88,
    card_hi: 0x0d3342,
    card_hi_alpha: 0.92,
    stroke: 0x9fe8f2,
    stroke_alpha: 0.12,
    rule_alpha: 0.10,
    ink: 0xeafcff,
    ink_soft: 0xc4e6ec,
    muted: 0x93bcc6,
    accent: 0x2cc4d6,
    accent_hi: 0x7fe3ee,
    on_accent: 0x03212b,
    success: 0x6fe3a8,
    warn: 0xffd37a,
    danger: 0xff9a9a,
    dark: true,
    art: SkyArt {
        sun: [0xd8fbff, 0xa6eef7, 0x62d4e6],
        far: 0x0a4458,
        near: 0x04202b,
        cloud: [0x0b4a5c, 0x11677c, 0x2a8fa3],
        lit: [0x13607a, 0x2288a0, 0x5cc0d0],
        shade: 0x06303f,
        star: 0xdffaff,
        mood: Mood::Water([0x8feeff, 0xbff6ff, 0xffffff]),
        land: Land::Plain,
    },
};

/// Deep Ocean: teal water, light shafts and bubbles, on Sunrise's glass pages.
pub const DEEP_OCEAN: Theme = Theme {
    id: "deep_ocean",
    name: "Deep Ocean",
    description: "Light through deep water",
    sky: OCEAN_DARK.sky,
    accent: ThemeAccent::Colour(0x2cc4d6),
    palette: Some(&OCEAN_DARK),
};

/// Autumn Forest's night: embers of amber and rust, leaves falling.
pub const AUTUMN_DARK: Night = Night {
    sky: [0x140b07, 0x24120a, 0x3a1b0d, 0x5e2a10, 0x9a4a14, 0xe08a2e],
    page: 0x160c07,
    side: 0x120907,
    card: 0x2a1509,
    card_alpha: 0.88,
    card_hi: 0x3b1f10,
    card_hi_alpha: 0.92,
    stroke: 0xffc58a,
    stroke_alpha: 0.12,
    rule_alpha: 0.10,
    ink: 0xfff4e6,
    ink_soft: 0xf0d8bd,
    muted: 0xc8a78a,
    accent: 0xe0782a,
    accent_hi: 0xffb26b,
    on_accent: 0x1e0e05,
    success: 0xb5d96b,
    warn: 0xffcf6b,
    danger: 0xff9a8a,
    dark: true,
    art: SkyArt {
        sun: [0xffe3a0, 0xffbf5c, 0xf0892a],
        far: 0x3a1c0e,
        near: 0x160a05,
        cloud: [0x3d1d0f, 0x6e3414, 0xb8621f],
        lit: [0x5a2c14, 0x9a4d1a, 0xe89a4a],
        shade: 0x2a1409,
        star: 0xffe9c8,
        mood: Mood::Leaves([0xd9531e, 0xf0a02a, 0x9c3a14]),
        land: Land::Plain,
    },
};

/// Autumn Forest: amber and rust under falling leaves, on Sunrise's glass pages.
pub const AUTUMN_FOREST: Theme = Theme {
    id: "autumn_forest",
    name: "Autumn Forest",
    description: "Falling leaves at dusk",
    sky: AUTUMN_DARK.sky,
    accent: ThemeAccent::Colour(0xe0782a),
    palette: Some(&AUTUMN_DARK),
};

/// Cherry Blossom's night: a lavender twilight with pink petals drifting.
pub const CHERRY_DARK: Night = Night {
    sky: [0x120d1f, 0x1f1633, 0x32214d, 0x4f3166, 0x7d4a80, 0xc98bb0],
    page: 0x140e22,
    side: 0x0f0a1b,
    card: 0x251a3a,
    card_alpha: 0.88,
    card_hi: 0x32244d,
    card_hi_alpha: 0.92,
    stroke: 0xffd0e4,
    stroke_alpha: 0.12,
    rule_alpha: 0.10,
    ink: 0xfff4fa,
    ink_soft: 0xead8ef,
    muted: 0xb9a6cc,
    accent: 0xf59ac0,
    accent_hi: 0xffc2da,
    on_accent: 0x2a0f1f,
    success: 0x8fe3b0,
    warn: 0xffd59a,
    danger: 0xff9aa8,
    dark: true,
    art: SkyArt {
        sun: [0xfff0f6, 0xffd0e2, 0xf5a0c0],
        far: 0x3a2655,
        near: 0x160f28,
        cloud: [0x3a2a5c, 0x6a4a86, 0xb87ca8],
        lit: [0x4f3a74, 0x8b64a0, 0xe8a8c8],
        shade: 0x2a1d44,
        star: 0xfff0fa,
        mood: Mood::Petals([0xffc2d9, 0xff8fb8]),
        land: Land::Plain,
    },
};

/// Cherry Blossom: a pink twilight with drifting petals, on Sunrise's glass pages.
pub const CHERRY_BLOSSOM: Theme = Theme {
    id: "cherry_blossom",
    name: "Cherry Blossom",
    description: "Petals on a pink evening",
    sky: CHERRY_DARK.sky,
    accent: ThemeAccent::Colour(0xf59ac0),
    palette: Some(&CHERRY_DARK),
};

/// Midnight Aurora's night: deep blue with curtains of green and violet light.
pub const AURORA_DARK: Night = Night {
    sky: [0x02060f, 0x040c1c, 0x071428, 0x0b1e34, 0x10304a, 0x1a4a5c],
    page: 0x030812,
    side: 0x02050d,
    card: 0x0a1426,
    card_alpha: 0.88,
    card_hi: 0x122238,
    card_hi_alpha: 0.92,
    stroke: 0xa8f0d0,
    stroke_alpha: 0.12,
    rule_alpha: 0.10,
    ink: 0xeefff8,
    ink_soft: 0xcce8e0,
    muted: 0x92b8b0,
    accent: 0x4fe0a0,
    accent_hi: 0x9af5cc,
    on_accent: 0x03140e,
    success: 0x7ff0a8,
    warn: 0xffd98a,
    danger: 0xff9aa0,
    dark: true,
    art: SkyArt {
        sun: [0xeafff5, 0xbdf5dd, 0x7fe0b8],
        far: 0x0d2236,
        near: 0x040a14,
        cloud: [0x0a1a2e, 0x10304a, 0x1d4a60],
        lit: [0x14304a, 0x1f5068, 0x3c8a90],
        shade: 0x07121f,
        star: 0xeefaff,
        mood: Mood::Aurora([0x3dffa0, 0x9b6bff, 0x35d6ff]),
        land: Land::Plain,
    },
};

/// Midnight Aurora: green and violet curtains over a deep blue night, on Sunrise's glass pages.
pub const MIDNIGHT_AURORA: Theme = Theme {
    id: "midnight_aurora",
    name: "Midnight Aurora",
    description: "Northern lights",
    sky: AURORA_DARK.sky,
    accent: ThemeAccent::Colour(0x4fe0a0),
    palette: Some(&AURORA_DARK),
};

/// Overworld's night: a deep blue sky, stars, a square moon over stepped grassy land.
pub const OVERWORLD_DARK: Night = Night {
    sky: [0x060b1e, 0x0a1230, 0x0f1a40, 0x16244f, 0x1f3060, 0x2c4070],
    page: 0x070c1c,
    side: 0x050916,
    card: 0x101a2e,
    card_alpha: 0.88,
    card_hi: 0x18243c,
    card_hi_alpha: 0.92,
    stroke: 0xb8e08a,
    stroke_alpha: 0.12,
    rule_alpha: 0.10,
    ink: 0xf1f6ea,
    ink_soft: 0xd3dcc8,
    muted: 0xa8b4a0,
    accent: 0x5fae3c,
    accent_hi: 0x9fdc6e,
    on_accent: 0x0c1a06,
    success: 0x9fdc6e,
    warn: 0xffd27a,
    danger: 0xff9a8a,
    dark: true,
    art: SkyArt {
        sun: [0xe8ecf4, 0xd6dcea, 0xc2cadc],
        far: 0x1a2a3c,
        near: 0x2a1d14,
        cloud: [0x1a2448, 0x223058, 0x2c3a66],
        lit: [0x2c3a66, 0x384878, 0x46568a],
        shade: 0x121a36,
        star: 0xf1f6ff,
        mood: Mood::Plain,
        land: Land::Blocks {
            grass: [0x24402c, 0x2f5a24],
            leaves: 0x1f4a1c,
        },
    },
};

/// Overworld: an original blocky day and night over stepped grassy land, square clouds drifting.
pub const OVERWORLD: Theme = Theme {
    id: "overworld",
    name: "Overworld",
    description: "Blocky hills under square clouds",
    sky: OVERWORLD_DARK.sky,
    accent: ThemeAccent::Colour(0x5fae3c),
    palette: Some(&OVERWORLD_DARK),
};

/// Taiga: what was the launcher's light appearance, now a theme of its own: a snowy blue hour
/// over spruce woods, deep slate plates and frost-teal accent, the snow the brightest thing.
pub const TAIGA_NIGHT: Night = Night {
    sky: [0x111923, 0x17212d, 0x1e2a38, 0x273647, 0x334559, 0x46596e],
    page: 0x101820,
    side: 0x0d141b,
    card: 0x18222d,
    card_alpha: 0.84,
    card_hi: 0x223040,
    card_hi_alpha: 0.90,
    stroke: 0xc4d8e8,
    stroke_alpha: 0.12,
    rule_alpha: 0.10,
    ink: 0xeef4f8,
    ink_soft: 0xc6d3de,
    muted: 0x97a9b9,
    accent: 0x2b7690,
    accent_hi: 0x8ccde2,
    on_accent: 0xffffff,
    success: 0x6fd3a0,
    warn: 0xf0b860,
    danger: 0xff8f86,
    dark: true,
    art: SkyArt {
        sun: [0xe6edf3, 0xd2dce5, 0xbcc8d4],
        far: 0x23313f,
        near: 0x0c1218,
        cloud: [0x1c2836, 0x243244, 0x2e3e52],
        lit: [0x3a4c62, 0x46596f, 0x54687e],
        shade: 0x151e29,
        star: 0xeef4ff,
        mood: Mood::Snow([0xf4f8fb, 0x0c1218]),
        land: Land::Taiga {
            snow: [0x9fb0c0, 0xd6e1ea],
        },
    },
};

/// Taiga: snow on the spruce woods. Where an old light appearance opens (`Settings::parse`).
pub const TAIGA: Theme = Theme {
    id: "taiga",
    name: "Taiga",
    description: "Snow on the spruce woods",
    sky: TAIGA_NIGHT.sky,
    accent: ThemeAccent::Colour(0x2b7690),
    palette: Some(&TAIGA_NIGHT),
};

/// Sunrise's own theme's line in Settings.
pub const TWILIGHT_NAME: &str = crate::i18n::tr_key("Twilight");
pub const TWILIGHT_DESCRIPTION: &str = crate::i18n::tr_key("Indigo night and violet");

/// The launcher themes past Sunrise's own sky: the featured families first, then the older ones.
pub const THEMES: [Theme; 10] = [
    DAWN_THEME,
    HYPR_NEON,
    DEEP_OCEAN,
    AUTUMN_FOREST,
    CHERRY_BLOSSOM,
    MIDNIGHT_AURORA,
    OVERWORLD,
    TAIGA,
    FIRST_LIGHT,
    NORD,
];

/// Sunrise's own theme's key in `Settings::theme_accents`.
pub const TWILIGHT: &str = "twilight";

/// What Twilight was saved as while it carried the name Sunrise. Settings that say so are read
/// as Twilight (`backend::look_settings`), so the indigo look a player chose stays theirs now
/// that Sunrise names the orange one.
pub const TWILIGHT_BEFORE: &str = "sunrise";

/// A theme's key in `Settings::theme_accents`: its id (a player's theme keeps its own
/// `local:<slug>`, there or not), or [`TWILIGHT`]. No saved theme and `dawn` share Dawn's.
pub fn accent_key(id: Option<&str>) -> &str {
    match id {
        Some(local) if local.starts_with(crate::theme_files::PREFIX) => local,
        _ => theme(id).map_or(TWILIGHT, |t| t.id),
    }
}

/// The accent a theme brings, before any the player picked under it.
pub fn own_accent(id: Option<&str>) -> Accent {
    match resolve(id) {
        Resolved::Twilight => Accent::VIOLET,
        Resolved::Builtin(t) => t.accent.accent(),
        Resolved::Local(local) => Accent::from_rgb(local.roles.accent),
        Resolved::Missing => DAWN_THEME.accent.accent(),
    }
}

/// The accent a theme starts with: the one last picked under it, else its own (violet for
/// Sunrise's).
pub fn starting_accent(
    id: Option<&str>,
    kept: &std::collections::BTreeMap<String, Accent>,
) -> Accent {
    kept.get(accent_key(id))
        .copied()
        .unwrap_or_else(|| own_accent(id))
}

/// What a saved theme id means right now.
#[derive(Clone)]
pub enum Resolved {
    /// The indigo and violet look: `twilight`, or an id this launcher doesn't know.
    Twilight,
    Builtin(&'static Theme),
    /// A theme file of the player's (`theme_files`).
    Local(crate::theme_files::Local),
    /// A `local:` id whose file is gone or broken: Dawn is painted until it is back, and the
    /// saved choice stays as it is.
    Missing,
}

/// Resolves `id` (`Settings::theme`) against the built-in themes and the player's, as last
/// read from disk.
pub fn resolve(id: Option<&str>) -> Resolved {
    match id {
        Some(local) if local.starts_with(crate::theme_files::PREFIX) => {
            crate::theme_files::find(local).map_or(Resolved::Missing, Resolved::Local)
        }
        _ => theme(id).map_or(Resolved::Twilight, Resolved::Builtin),
    }
}

/// The theme named `id` (`Settings::theme`): Dawn while none is saved, None for Sunrise's own sky
/// ([`TWILIGHT`]) or a name this launcher doesn't know.
pub fn theme(id: Option<&str>) -> Option<&'static Theme> {
    let id = id.unwrap_or(DAWN_THEME.id);
    THEMES.iter().find(|t| t.id == id)
}

/// The glass look's colour for each of [`Accent::ALL`]: the mockup's swatches.
pub const SWATCHES: [u32; 7] = [
    0x4f60e6, 0x2563eb, 0x0f766e, 0x15803d, 0xc2410c, 0xdc2626, 0xbe185d,
];

/// [`Accent::ALL`]'s names, in its order.
const NAMES: [&str; 7] = ["violet", "blue", "teal", "green", "amber", "red", "pink"];

impl Accent {
    pub const VIOLET: Accent = Accent {
        hue: None,
        rgb: None,
    };
    pub const BLUE: Accent = Accent {
        hue: Some(212.),
        rgb: None,
    };
    pub const TEAL: Accent = Accent {
        hue: Some(176.),
        rgb: None,
    };
    pub const GREEN: Accent = Accent {
        hue: Some(140.),
        rgb: None,
    };
    pub const AMBER: Accent = Accent {
        hue: Some(36.),
        rgb: None,
    };
    pub const RED: Accent = Accent {
        hue: Some(356.),
        rgb: None,
    };
    pub const PINK: Accent = Accent {
        hue: Some(322.),
        rgb: None,
    };

    pub const ALL: [Accent; 7] = [
        Self::VIOLET,
        Self::BLUE,
        Self::TEAL,
        Self::GREEN,
        Self::AMBER,
        Self::RED,
        Self::PINK,
    ];

    /// Any hue the player picks.
    pub fn custom(hue: f32) -> Self {
        Self {
            hue: Some(hue.rem_euclid(360.)),
            rgb: None,
        }
    }

    /// A colour (`0xrrggbb`): one of the swatches is its named accent.
    pub fn from_rgb(rgb: u32) -> Self {
        let rgb = rgb & 0xff_ffff;
        match SWATCHES.iter().position(|&c| c == rgb) {
            Some(i) => Self::ALL[i],
            None => Self {
                hue: Some(hsl(rgb).0),
                rgb: Some(rgb),
            },
        }
    }

    /// The colour the glass look wears: the one typed, a named accent's swatch, else the hue at
    /// the violet's saturation and lightness.
    pub fn rgb(self) -> u32 {
        if let Some(rgb) = self.rgb {
            return rgb;
        }
        match (Self::ALL.iter().position(|a| *a == self), self.hue) {
            (Some(i), _) => SWATCHES[i],
            (None, Some(hue)) => {
                let (_, s, l) = hsl(SWATCHES[0]);
                from_hsl(hue, s, l)
            }
            (None, None) => SWATCHES[0],
        }
    }

    /// [`rgb`](Self::rgb) as `#rrggbb`.
    pub fn hex(self) -> String {
        format!("#{:06x}", self.rgb())
    }

    /// Its name in `settings.json` and on the command line: a named hue's, a typed colour's
    /// `#rrggbb`, else its degrees.
    pub fn name(self) -> String {
        if self.rgb.is_some() {
            return self.hex();
        }
        let named = Self::ALL
            .iter()
            .position(|a| *a == self)
            .map(|i| NAMES[i].to_owned());
        named.unwrap_or_else(|| match self.hue {
            None => "violet".to_owned(),
            Some(hue) => format!("{hue}"),
        })
    }

    /// A name, `#rrggbb`, or a hue in degrees.
    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        if let Some(hex) = name.strip_prefix('#') {
            return (hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| u32::from_str_radix(hex, 16).ok())
                .flatten()
                .map(Self::from_rgb);
        }
        let name = if name == "purple" || name == "default" {
            "violet"
        } else {
            &name
        };
        if let Some(i) = NAMES.iter().position(|n| *n == name) {
            return Some(Self::ALL[i]);
        }
        name.parse::<f32>()
            .ok()
            .filter(|hue| hue.is_finite())
            .map(Self::custom)
    }

    /// Its hue in degrees; None for the violet, whose tokens stay as they are.
    pub fn hue(self) -> Option<f32> {
        self.hue
    }

    /// `palette` with this accent.
    pub fn apply(self, mut p: Blocks) -> Blocks {
        let Some(hue) = self.hue else {
            return p;
        };
        let turn = |c: u32| {
            let (_, s, l) = hsl(c);
            (hue, s, l)
        };
        // The face holds its label: white on a dark face, Nord's ink on a light one. Move the
        // face away from the label until the two read.
        let step = if contrast(p.on_face, 0x000000) < contrast(p.on_face, 0xffffff) {
            1.
        } else {
            -1.
        };
        let face = fit(turn(p.face), |c| contrast(p.on_face, c) >= 4.5, step);
        let (_, _, face_l) = hsl(face);
        let (h, s, l) = turn(p.face_bevel);
        p.face_bevel = from_hsl(h, s, l.max(face_l + 0.08).min(0.92));
        let (h, s, l) = turn(p.face_lip);
        p.face_lip = from_hsl(h, s, l.min(face_l - 0.12).max(0.05));
        p.face = face;
        // The accent marks things on every background: lighter on dark, darker on light.
        let grounds = [p.page, p.raised, p.button, p.panel, p.well];
        let reads = |c: u32| grounds.iter().all(|&g| contrast(c, g) >= 3.2);
        p.accent = fit(turn(p.accent), reads, if p.dark { 1. } else { -1. });
        p
    }
}

impl TryFrom<String> for Accent {
    type Error = String;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        Self::parse(&name).ok_or_else(|| {
            crate::i18n::trf(
                "{name}: a colour name or a hue in degrees",
                &[("name", &name)],
            )
        })
    }
}

impl From<Accent> for String {
    fn from(accent: Accent) -> Self {
        accent.name()
    }
}

/// The colour the hue picker shows for `hue`: a strip of them, at one saturation and lightness.
pub fn hue_colour(hue: f32) -> u32 {
    from_hsl(hue.rem_euclid(360.), 0.62, 0.55)
}

/// The colour's hue (degrees), saturation and lightness (0 to 1).
fn hsl(c: u32) -> (f32, f32, f32) {
    let [r, g, b] = [16, 8, 0].map(|s| ((c >> s) & 255) as f32 / 255.);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.;
    if max == min {
        return (0., 0., l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2. - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6. } else { 0. }
    } else if max == g {
        (b - r) / d + 2.
    } else {
        (r - g) / d + 4.
    };
    (h * 60., s, l)
}

fn from_hsl(h: f32, s: f32, l: f32) -> u32 {
    let c = (1. - (2. * l - 1.).abs()) * s;
    let x = c * (1. - ((h / 60.) % 2. - 1.).abs());
    let m = l - c / 2.;
    let (r, g, b) = match (h.rem_euclid(360.) / 60.) as u32 {
        0 => (c, x, 0.),
        1 => (x, c, 0.),
        2 => (0., c, x),
        3 => (0., x, c),
        4 => (x, 0., c),
        _ => (c, 0., x),
    };
    let byte = |v: f32| ((v + m).clamp(0., 1.) * 255.).round() as u32;
    byte(r) << 16 | byte(g) << 8 | byte(b)
}

/// The colour, made lighter (`step` 1) or darker (-1) a little at a time until `good` holds.
fn fit((h, s, l): (f32, f32, f32), good: impl Fn(u32) -> bool, step: f32) -> u32 {
    let mut l = l;
    for _ in 0..100 {
        let c = from_hsl(h, s, l);
        if good(c) || !(0.0..=1.0).contains(&l) {
            return c;
        }
        l += step * 0.01;
    }
    from_hsl(h, s, l.clamp(0., 1.))
}

use crate::theme_files;
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::Dir;

/// Keeps `file` as Home's background in `dir` (`background-<hash>.<ext>`: a new picture gets a
/// new name, so the window doesn't show the one it cached), and returns the kept file's name.
/// Nothing is kept unless it is a picture Home can show: at most 40 MB are read, what kind it
/// is comes from what it holds and never from its name, its size is within the limits, and it
/// decodes to one still picture. Slow: call it off the window's thread. An older background
/// stays: nothing in the data folder is deleted because a setting changed, since a name alone
/// doesn't say whose a file is.
pub fn keep_background(dir: &Path, file: &Path) -> Result<String, String> {
    use std::io::Read as _;
    let shown = file.display().to_string();
    let mut bytes = Vec::new();
    std::fs::File::open(file)
        .and_then(|f| f.take(theme_files::MAX_PICTURE + 1).read_to_end(&mut bytes))
        .map_err(|e| format!("{shown}: {e}"))?;
    if bytes.len() as u64 > theme_files::MAX_PICTURE {
        return Err(theme_files::Reason::PictureTooBig(shown).text());
    }
    let ext = match theme_files::kind(&bytes, &shown).map_err(|r| r.text())? {
        image::ImageFormat::Png => "png",
        image::ImageFormat::Jpeg => "jpg",
        _ => "webp",
    };
    theme_files::still(&bytes, &shown).map_err(|r| r.text())?;
    let hash = crate::sources::sha1_hex(&bytes);
    let name = format!("background-{}.{ext}", &hash[..12]);
    publish(dir, &name, &bytes).map_err(|e| format!("{name}: {e}"))?;
    Ok(name)
}

/// Whether `name` in `dir` is already this picture: a regular file, itself and not a link,
/// holding exactly `bytes`. `None` when nothing has that name.
fn same_copy(dir: &Dir, name: &str, bytes: &[u8]) -> std::io::Result<Option<bool>> {
    use std::io::Read as _;
    match dir.symlink_metadata(name) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
        Ok(meta) if !meta.is_file() => return Ok(Some(false)),
        Ok(_) => {}
    }
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = dir.open_with(name, &options)?;
    if !file.metadata()?.is_file() {
        return Ok(Some(false));
    }
    let mut held = Vec::new();
    file.take(bytes.len() as u64 + 1).read_to_end(&mut held)?;
    Ok(Some(held == bytes))
}

/// Puts `bytes` in `dir` as `name` without ever writing through what is already there. The
/// bytes go to a new file of the launcher's own, made exclusively, which then takes the name
/// in one step only if the name is free. A copy already there is kept as it is when it is
/// this picture, and anything else under the name (a link, a folder, other bytes) is refused
/// and left alone.
fn publish(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let taken = || {
        std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "something else already has this name in the launcher's folder; it was left alone",
        )
    };
    std::fs::create_dir_all(dir)?;
    let dir = Dir::open_ambient_dir(dir, cap_std::ambient_authority())?;
    match same_copy(&dir, name, bytes)? {
        Some(true) => return Ok(()),
        Some(false) => return Err(taken()),
        None => {}
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let temp = format!(".background-{}-{nanos}.tmp", std::process::id());
    let mut options = cap_std::fs::OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    let written = dir
        .open_with(&temp, &options)
        .and_then(|mut file| file.write_all(bytes).and_then(|()| file.sync_all()));
    // A hard link takes the name only if it is free, and never follows what is there.
    let published = written.and_then(|()| {
        dir.hard_link(&temp, &dir, name).or_else(|e| {
            // A disk with no hard links (FAT): the name is taken by a move, if it is free.
            let free = matches!(dir.symlink_metadata(name), Err(e) if e.kind() == std::io::ErrorKind::NotFound);
            if e.kind() != std::io::ErrorKind::AlreadyExists && free {
                dir.rename(&temp, &dir, name)
            } else {
                Err(e)
            }
        })
    });
    let _ = dir.remove_file(&temp);
    match published {
        Ok(()) => Ok(()),
        // Another launcher got there first: fine if it put the same picture.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            match same_copy(&dir, name, bytes)? {
                Some(true) => Ok(()),
                _ => Err(taken()),
            }
        }
        Err(e) => Err(e),
    }
}

/// The kept background `name`, decoded for Home: the copy in `dir` itself, never through a
/// link, read with the same bounds a new picture is held to and decoded to one still picture.
/// A copy kept by an older launcher that isn't such a picture is an error, never shown. Slow:
/// call it off the window's thread.
pub fn background_picture(dir: &Path, name: &str) -> Result<image::RgbaImage, String> {
    use std::io::Read as _;
    let said = |e: std::io::Error| format!("{name}: {e}");
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = Dir::open_ambient_dir(dir, cap_std::ambient_authority())
        .and_then(|dir| dir.open_with(name, &options))
        .map_err(said)?;
    if !file.metadata().map_err(said)?.is_file() {
        return Err(theme_files::Reason::Linked(crate::i18n::tr_key("picture")).text());
    }
    let mut bytes = Vec::new();
    file.take(theme_files::MAX_PICTURE + 1)
        .read_to_end(&mut bytes)
        .map_err(said)?;
    if bytes.len() as u64 > theme_files::MAX_PICTURE {
        return Err(theme_files::Reason::PictureTooBig(name.to_string()).text());
    }
    theme_files::still(&bytes, name).map_err(|r| r.text())
}

/// Home's background, if the player keeps one and it is still there.
pub fn background(dir: &Path, name: Option<&str>) -> Option<PathBuf> {
    let name = name.filter(|n| n.starts_with("background-") && !n.contains(['/', '\\']))?;
    Some(dir.join(name)).filter(|p| p.is_file())
}

#[cfg(test)]
mod tests;
