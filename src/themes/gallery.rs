//! The gallery of Settings › Appearance: Sunrise, Daybreak and six more built-in themes, each
//! made of the nine colours of a theme file (`theme_files::Roles`) over Sunrise's quiet sky. The
//! older built-ins in `builtin` stay resolvable so a saved choice still paints, but the gallery
//! doesn't list them.

use super::*;

/// Midnight's palette: the nine colours of its tile in Settings, as the launcher's tokens.
pub const MIDNIGHT_NIGHT: Night = Night {
    sky: [0x0f1422, 0x101524, 0x111626, 0x121827, 0x131929, 0x141a2b],
    page: 0x141a2b,
    side: 0x0f1422,
    card: 0x1c2438,
    card_alpha: 0.88,
    card_hi: 0x232b3e,
    card_hi_alpha: 0.92,
    stroke: 0x2c3650,
    stroke_alpha: 0.66,
    rule_alpha: 0.7,
    ink: 0xe8ecf6,
    ink_soft: 0xe8ecf6,
    muted: 0x9aa4bd,
    accent: 0x6c8cff,
    accent_hi: 0x6c8cff,
    on_accent: 0x0b0b0d,
    success: 0x6fd3a0,
    warn: 0xefc56e,
    danger: 0xf08a8a,
    dark: true,
    art: SkyArt {
        sun: [0xe9eeff, 0xc4d1ff, 0x6c8cff],
        far: 0x202a49,
        near: 0x0a0d16,
        cloud: [0x1f2840, 0x273354, 0x324170],
        lit: [0x2a3551, 0x35426c, 0x445795],
        shade: 0x0c101b,
        star: 0xe8ecf6,
        mood: Mood::Plain,
        land: Land::Plain,
    },
};

/// Grove's palette: the nine colours of its tile in Settings, as the launcher's tokens.
pub const GROVE_NIGHT: Night = Night {
    sky: [0x141a16, 0x151c17, 0x161d19, 0x181f1a, 0x19201c, 0x1a221d],
    page: 0x1a221d,
    side: 0x141a16,
    card: 0x212b25,
    card_alpha: 0.88,
    card_hi: 0x28312c,
    card_hi_alpha: 0.92,
    stroke: 0x33423a,
    stroke_alpha: 0.66,
    rule_alpha: 0.7,
    ink: 0xe6efe8,
    ink_soft: 0xe6efe8,
    muted: 0xa3b5a9,
    accent: 0x6fbf73,
    accent_hi: 0x6fbf73,
    on_accent: 0x0b0b0d,
    success: 0x8fd694,
    warn: 0xefc56e,
    danger: 0xe88f84,
    dark: true,
    art: SkyArt {
        sun: [0xe9f5ea, 0xc5e5c7, 0x6fbf73],
        far: 0x263829,
        near: 0x0d110e,
        cloud: [0x243128, 0x2c4030, 0x37543b],
        lit: [0x2f3f33, 0x39533d, 0x48714c],
        shade: 0x101512,
        star: 0xe6efe8,
        mood: Mood::Plain,
        land: Land::Plain,
    },
};

/// Amethyst's palette: the nine colours of its tile in Settings, as the launcher's tokens.
pub const AMETHYST_NIGHT: Night = Night {
    sky: [0x171421, 0x181523, 0x1a1625, 0x1b1827, 0x1d1929, 0x1e1a2b],
    page: 0x1e1a2b,
    side: 0x171421,
    card: 0x262137,
    card_alpha: 0.88,
    card_hi: 0x2d283d,
    card_hi_alpha: 0.92,
    stroke: 0x3a3352,
    stroke_alpha: 0.66,
    rule_alpha: 0.7,
    ink: 0xefeaf8,
    ink_soft: 0xefeaf8,
    muted: 0xada4c4,
    accent: 0xb18cff,
    accent_hi: 0xb18cff,
    on_accent: 0x0b0b0d,
    success: 0x8fd6a8,
    warn: 0xefc56e,
    danger: 0xf39ab0,
    dark: true,
    art: SkyArt {
        sun: [0xf3eeff, 0xe0d1ff, 0xb18cff],
        far: 0x332a49,
        near: 0x0f0d15,
        cloud: [0x2c253f, 0x393053, 0x4d3f6f],
        lit: [0x3a3250, 0x4d406c, 0x685594],
        shade: 0x12101a,
        star: 0xefeaf8,
        mood: Mood::Plain,
        land: Land::Plain,
    },
};

/// Ember's palette: the nine colours of its tile in Settings, as the launcher's tokens.
pub const EMBER_NIGHT: Night = Night {
    sky: [0x18110f, 0x191210, 0x1b1311, 0x1c1513, 0x1e1614, 0x1f1715],
    page: 0x1f1715,
    side: 0x18110f,
    card: 0x2a1e1b,
    card_alpha: 0.88,
    card_hi: 0x302522,
    card_hi_alpha: 0.92,
    stroke: 0x44302a,
    stroke_alpha: 0.66,
    rule_alpha: 0.7,
    ink: 0xf5e9e3,
    ink_soft: 0xf5e9e3,
    muted: 0xbfa79f,
    accent: 0xff7a3d,
    accent_hi: 0xff7a3d,
    on_accent: 0x0b0b0d,
    success: 0x8fcf8a,
    warn: 0xefc56e,
    danger: 0xff8f8f,
    dark: true,
    art: SkyArt {
        sun: [0xffebe2, 0xffcab1, 0xff7a3d],
        far: 0x3e251b,
        near: 0x100b0a,
        cloud: [0x33221c, 0x482b20, 0x663825],
        lit: [0x452e25, 0x623928, 0x8d4b2e],
        shade: 0x130e0c,
        star: 0xf5e9e3,
        mood: Mood::Plain,
        land: Land::Plain,
    },
};

/// Graphite's palette: the nine colours of its tile in Settings, as the launcher's tokens.
pub const GRAPHITE_NIGHT: Night = Night {
    sky: [0x141416, 0x161618, 0x171719, 0x19191b, 0x1a1a1c, 0x1c1c1e],
    page: 0x1c1c1e,
    side: 0x141416,
    card: 0x252527,
    card_alpha: 0.88,
    card_hi: 0x2c2c2d,
    card_hi_alpha: 0.92,
    stroke: 0x38383b,
    stroke_alpha: 0.66,
    rule_alpha: 0.7,
    ink: 0xededee,
    ink_soft: 0xededee,
    muted: 0x9a9a9e,
    accent: 0xd4d4d8,
    accent_hi: 0xd4d4d8,
    on_accent: 0x0b0b0d,
    success: 0x86d69a,
    warn: 0xefc56e,
    danger: 0xf08f8f,
    dark: true,
    art: SkyArt {
        sun: [0xf9f9f9, 0xeeeeef, 0xd4d4d8],
        far: 0x363638,
        near: 0x0d0d0e,
        cloud: [0x2c2c2e, 0x3e3e40, 0x565659],
        lit: [0x3d3d3e, 0x545456, 0x78787a],
        shade: 0x101012,
        star: 0xededee,
        mood: Mood::Plain,
        land: Land::Plain,
    },
};

/// Parchment's palette: the nine colours of its tile in Settings, as the launcher's tokens.
pub const PARCHMENT_NIGHT: Night = Night {
    sky: [0xf1ebe0, 0xf3ede3, 0xf5f0e6, 0xf7f2ea, 0xf9f5ed, 0xfbf7f0],
    page: 0xfbf7f0,
    side: 0xf1ebe0,
    card: 0xffffff,
    card_alpha: 0.9,
    card_hi: 0xf7f7f7,
    card_hi_alpha: 0.94,
    stroke: 0xe2d9cb,
    stroke_alpha: 0.66,
    rule_alpha: 0.7,
    ink: 0x2b2824,
    ink_soft: 0x2b2824,
    muted: 0x716a60,
    accent: 0x8a5a2b,
    accent_hi: 0x8a5a2b,
    on_accent: 0xffffff,
    success: 0x387b46,
    warn: 0x8a5e00,
    danger: 0xb9443a,
    dark: false,
    art: SkyArt {
        sun: [0xede6df, 0xd0bdaa, 0x8a5a2b],
        far: 0xf0e7dc,
        near: 0xe7dbcd,
        cloud: [0xffffff, 0xfdfbf8, 0xf0e7dc],
        lit: [0xffffff, 0xf0eeeb, 0xe8e1da],
        shade: 0xe7dcce,
        star: 0x2b2824,
        mood: Mood::Plain,
        land: Land::Plain,
    },
};

/// Midnight: deep blue and periwinkle.
pub const MIDNIGHT: Theme = Theme {
    id: "midnight",
    name: crate::i18n::tr_key("Midnight"),
    description: crate::i18n::tr_key("Deep blue and periwinkle"),
    sky: MIDNIGHT_NIGHT.sky,
    accent: ThemeAccent::Colour(0x6c8cff),
    palette: Some(&MIDNIGHT_NIGHT),
    light_palette: None,
};

/// Grove: moss green and leaf.
pub const GROVE: Theme = Theme {
    id: "grove",
    name: crate::i18n::tr_key("Grove"),
    description: crate::i18n::tr_key("Moss green and leaf"),
    sky: GROVE_NIGHT.sky,
    accent: ThemeAccent::Colour(0x6fbf73),
    palette: Some(&GROVE_NIGHT),
    light_palette: None,
};

/// Amethyst: violet dusk and lilac.
pub const AMETHYST: Theme = Theme {
    id: "amethyst",
    name: crate::i18n::tr_key("Amethyst"),
    description: crate::i18n::tr_key("Violet dusk and lilac"),
    sky: AMETHYST_NIGHT.sky,
    accent: ThemeAccent::Colour(0xb18cff),
    palette: Some(&AMETHYST_NIGHT),
    light_palette: None,
};

/// Ember: warm brown and flame.
pub const EMBER: Theme = Theme {
    id: "ember",
    name: crate::i18n::tr_key("Ember"),
    description: crate::i18n::tr_key("Warm brown and flame"),
    sky: EMBER_NIGHT.sky,
    accent: ThemeAccent::Colour(0xff7a3d),
    palette: Some(&EMBER_NIGHT),
    light_palette: None,
};

/// Graphite: neutral grey and silver.
pub const GRAPHITE: Theme = Theme {
    id: "graphite",
    name: crate::i18n::tr_key("Graphite"),
    description: crate::i18n::tr_key("Neutral grey and silver"),
    sky: GRAPHITE_NIGHT.sky,
    accent: ThemeAccent::Colour(0xd4d4d8),
    palette: Some(&GRAPHITE_NIGHT),
    light_palette: None,
};

/// Parchment: cream paper and ink.
pub const PARCHMENT: Theme = Theme {
    id: "parchment",
    name: crate::i18n::tr_key("Parchment"),
    description: crate::i18n::tr_key("Cream paper and ink"),
    sky: PARCHMENT_NIGHT.sky,
    accent: ThemeAccent::Colour(0x8a5a2b),
    palette: Some(&PARCHMENT_NIGHT),
    light_palette: Some(&PARCHMENT_NIGHT),
};

/// The gallery, in the order it is shown: eight themes, two rows of four. Every other theme in
/// [`THEMES`] is kept for a saved choice and left out here.
pub const GALLERY: [&Theme; 8] = [
    &DAWN_THEME,
    &MIDNIGHT,
    &GROVE,
    &AMETHYST,
    &EMBER,
    &GRAPHITE,
    &PARCHMENT,
    &DAWN_LIGHT_THEME,
];
