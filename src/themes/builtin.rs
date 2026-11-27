//! The built-in themes: each one's palette (`Night`) and its entry in Settings.

use super::*;

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
    light_palette: None,
};

/// Nord: the Nord palette's polar night, snow storm and frost.
pub const NORD: Theme = Theme {
    id: "nord",
    name: "Nord",
    description: "Polar night and frost",
    sky: NORD_DARK.sky,
    accent: ThemeAccent::Colour(0x88c0d0),
    palette: Some(&NORD_DARK),
    light_palette: None,
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

/// Dawn's day: the same warm palette as [`DAWN_DARK`] turned light, a cream room with white
/// cards, the burnt-orange brand on its buttons and the sky at dawn, pale peach to cream. Sunrise
/// draws it in the light look (`theme::from_settings`).
pub const DAWN_LIGHT: Night = Night {
    sky: [0xf9e1d3, 0xfae6d8, 0xfbebdd, 0xfcefe2, 0xfdf3e6, 0xfcf6ea],
    page: 0xf7f1ee,
    side: 0xefe7e3,
    card: 0xffffff,
    card_alpha: 0.9,
    card_hi: 0xf3ebe7,
    card_hi_alpha: 0.94,
    stroke: 0xd9cdc8,
    stroke_alpha: 0.67,
    rule_alpha: 0.7,
    ink: 0x2b2324,
    ink_soft: 0x4a3e3e,
    muted: 0x6f625e,
    accent: 0xc2552d,
    accent_hi: 0x9c3f1e,
    on_accent: 0xffffff,
    success: 0x23703a,
    warn: 0x8a5e00,
    danger: 0xb93a3a,
    dark: false,
    art: SkyArt {
        sun: [0xfff6e6, 0xffe2c4, 0xffc99a],
        far: 0xead8cc,
        near: 0xdcc3b6,
        cloud: [0xfff8f2, 0xfbeadf, 0xf7d9c8],
        lit: [0xffffff, 0xfff4ea, 0xfbe0cd],
        shade: 0xefd9cc,
        star: 0xffffff,
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
    light_palette: Some(&DAWN_LIGHT),
};

/// Hypr Neon: OLED black with neon cyan and magenta, on Sunrise's pages with solid plates.
pub const HYPR_NEON: Theme = Theme {
    id: "hypr_neon",
    name: "Hypr Neon",
    description: "Neon on pure black",
    sky: HYPR_DARK.sky,
    accent: ThemeAccent::Colour(0x3fe0ff),
    palette: Some(&HYPR_DARK),
    light_palette: None,
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
    light_palette: None,
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
    light_palette: None,
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
    light_palette: None,
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
    light_palette: None,
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
    light_palette: None,
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
    light_palette: None,
};
