//! The launcher look's part of `settings.json`: malformed choices dropped alone, an old light
//! appearance moved to the Sunrise Light theme, and the indigo theme once saved as "sunrise" read
//! as Twilight.
use super::{Appearance, Settings};
use serde_json::{Map, Value};

/// The settling that files saved by this launcher carry (`Settings::look_version`). Files without
/// one predate the appearance choice; version 1 files may carry a light or system appearance,
/// which version 2 has folded into the theme.
pub(super) const LOOK_VERSION: u8 = 2;

/// Drops the look's fields that aren't what they say, each alone, before the rest is read.
pub(super) fn tidy(fields: &mut Map<String, Value>) {
    // A version that isn't a small whole number is as if never saved.
    if fields
        .get("look_version")
        .is_some_and(|v| serde_json::from_value::<u8>(v.clone()).is_err())
    {
        fields.remove("look_version");
    }
    // An accent per theme that isn't one is dropped alone; a map that isn't one, whole.
    match fields.get_mut("theme_accents") {
        Some(Value::Object(accents)) => accents
            .retain(|_, a| serde_json::from_value::<crate::themes::Accent>(a.clone()).is_ok()),
        Some(_) => {
            fields.remove("theme_accents");
        }
        None => {}
    }
    // Twilight was saved as "sunrise" while it carried that name: its choice and the accent
    // kept under it are read under its own id. Nothing saves the old one any more, so this
    // can't catch a newer choice.
    use crate::themes::{TWILIGHT, TWILIGHT_BEFORE};
    if fields.get("theme").and_then(Value::as_str) == Some(TWILIGHT_BEFORE) {
        fields.insert("theme".into(), TWILIGHT.into());
    }
    if let Some(Value::Object(accents)) = fields.get_mut("theme_accents") {
        if let Some(kept) = accents.remove(TWILIGHT_BEFORE) {
            accents.entry(TWILIGHT).or_insert(kept);
        }
    }
    // A switch that isn't on or off is as if never set, not a reason to lose the rest.
    for key in ["performance", "reduce_motion", "cat"] {
        if fields
            .get(key)
            .is_some_and(|v| !v.is_boolean() && !v.is_null())
        {
            fields.remove(key);
        }
    }
}

/// Settles the look of a file as it was read. A file from before the appearance choice (no
/// `look_version`) was always dark: a saved light moves to the Taiga theme, as it always did.
/// Light is a theme now (Sunrise Light) and no longer a mode: a version 1 file whose appearance
/// was light on Sunrise (or on no saved theme, which is Sunrise) moves to that theme, and any
/// other appearance (the system's too) opens dark, which is how every other theme painted. The
/// appearance kept in memory follows the theme from then on.
pub(super) fn settle(settings: &mut Settings) {
    let from = settings.look_version;
    if from < 1 && settings.appearance == Appearance::Light {
        settings.theme = Some(crate::themes::TAIGA.id.to_string());
        settings.accent =
            crate::themes::starting_accent(settings.theme.as_deref(), &settings.theme_accents);
        settings.appearance = Appearance::Dark;
    }
    settings.look_version = LOOK_VERSION;

    // Old Dawn saved its pink default like a picked accent. If it wasn't saved under
    // the theme as a choice, follow the updated default instead of keeping the old one.
    use crate::themes::DAWN_THEME;
    if settings
        .theme
        .as_deref()
        .is_none_or(|id| id == DAWN_THEME.id)
        && settings.accent.rgb() == 0xc2366f
        && !settings.theme_accents.contains_key(DAWN_THEME.id)
    {
        settings.accent = DAWN_THEME.accent.accent();
    }

    use crate::themes::DAWN_LIGHT_THEME;
    if from == 1
        && settings.appearance == Appearance::Light
        && settings
            .theme
            .as_deref()
            .is_none_or(|id| id == DAWN_THEME.id)
    {
        // The accent picked under Sunrise comes along to its light card.
        if let Some(kept) = settings.theme_accents.get(DAWN_THEME.id).copied() {
            settings
                .theme_accents
                .entry(DAWN_LIGHT_THEME.id.to_string())
                .or_insert(kept);
        }
        settings.theme = Some(DAWN_LIGHT_THEME.id.to_string());
    }
    // The theme alone says whether the launcher is light.
    settings.appearance = if crate::themes::is_light(settings.theme.as_deref()) {
        Appearance::Light
    } else {
        Appearance::Dark
    };
}

#[cfg(test)]
mod tests {
    use super::super::Appearance;
    use super::Settings;

    /// A file saved before the appearance choice: a light opens on Taiga, dark or the system's
    /// on the saved theme, all of them dark, as they always were.
    #[test]
    fn a_file_from_before_the_choice_settles_to_dark_as_it_always_did() {
        let light = Settings::parse(r#"{"appearance": "light", "game_args": "--x"}"#);
        assert_eq!(light.appearance, Appearance::Dark);
        assert_eq!(light.theme.as_deref(), Some("taiga"));
        assert_eq!(light.game_args, "--x");
        for appearance in ["dark", "system"] {
            let kept = Settings::parse(&format!(
                r#"{{"appearance": "{appearance}", "theme": "nord"}}"#
            ));
            assert_eq!(kept.appearance, Appearance::Dark, "{appearance}");
            assert_eq!(kept.theme.as_deref(), Some("nord"), "{appearance}");
        }
        assert_eq!(Settings::parse("{}").appearance, Appearance::Dark);
    }

    /// A light chosen while the mode existed (version 1) on Sunrise, or on no saved theme, is the
    /// Sunrise Light theme now, with the accent picked under Sunrise. It stays there through
    /// saves, and the system's choice opens dark.
    #[test]
    fn a_saved_light_appearance_becomes_the_light_theme() {
        for text in [
            r##"{"appearance": "light", "look_version": 1, "theme_accents": {"dawn": "teal"}}"##,
            r##"{"appearance": "light", "theme": "dawn", "look_version": 1, "theme_accents": {"dawn": "teal"}}"##,
        ] {
            let read = Settings::parse(text);
            assert_eq!(read.theme.as_deref(), Some("dawn_light"), "{text}");
            assert_eq!(read.appearance, Appearance::Light, "{text}");
            assert_eq!(read.look_version, super::LOOK_VERSION);
            assert_eq!(
                read.theme_accents["dawn_light"],
                crate::themes::Accent::TEAL
            );
            let again = Settings::parse(&serde_json::to_string(&read).unwrap());
            assert_eq!(again.theme.as_deref(), Some("dawn_light"), "{text}");
            assert_eq!(again.appearance, Appearance::Light, "{text}");
        }
        let system = Settings::parse(r#"{"appearance": "system", "look_version": 1}"#);
        assert_eq!(
            (system.appearance, system.theme.as_deref()),
            (Appearance::Dark, None)
        );
    }

    /// A light under another theme painted dark (only Sunrise had a light look), so it stays
    /// that theme, dark. Choosing the light theme is light, and nothing else is.
    #[test]
    fn light_is_only_the_light_theme() {
        let other =
            Settings::parse(r#"{"appearance": "light", "theme": "nord", "look_version": 1}"#);
        assert_eq!(other.appearance, Appearance::Dark);
        assert_eq!(other.theme.as_deref(), Some("nord"));
        let chosen = Settings::parse(r#"{"theme": "dawn_light", "look_version": 2}"#);
        assert_eq!(chosen.appearance, Appearance::Light);
        let stale =
            Settings::parse(r#"{"appearance": "light", "theme": "nord", "look_version": 2}"#);
        assert_eq!(stale.appearance, Appearance::Dark);
    }

    /// An appearance this launcher doesn't know is dark; a look version that isn't a number is
    /// as if never saved, and the rest of the file is kept.
    #[test]
    fn unknown_appearance_and_bad_look_version_lose_only_themselves() {
        let read = Settings::parse(
            r#"{"appearance": "sepia", "look_version": "new", "game_args": "--x"}"#,
        );
        assert_eq!(read.appearance, Appearance::Dark);
        assert_eq!(read.game_args, "--x");
        assert_eq!(read.look_version, super::LOOK_VERSION);
    }

    #[test]
    fn dawn_follows_the_new_default_but_keeps_accent_choices() {
        for theme in ["null", "\"dawn\""] {
            let legacy = format!(r##"{{"theme":{theme}, "accent":"#C2366F"}}"##);
            let settled = Settings::parse(&legacy);
            assert_eq!(settled.accent.rgb(), 0xc2552d);
            let saved = serde_json::to_string(&settled).unwrap();
            assert_eq!(Settings::parse(&saved).accent, settled.accent);
            let picked = format!(
                r##"{{"theme":{theme}, "accent":"#C2366F", "theme_accents":{{"dawn":"#C2366F"}}}}"##
            );
            assert_eq!(Settings::parse(&picked).accent.rgb(), 0xc2366f);
        }
        for (json, expected) in [
            (r##"{"theme":"twilight", "accent":"#C2366F"}"##, 0xc2366f),
            (r##"{"theme":"dawn", "accent":"#C23670"}"##, 0xc23670),
            (r##"{"theme":"dawn", "accent":"pink"}"##, 0xbe185d),
        ] {
            assert_eq!(Settings::parse(json).accent.rgb(), expected);
        }
    }

    /// A wrong accent under one theme, or a motion choice that isn't one, loses only itself.
    #[test]
    fn theme_accents_and_motion_survive_bad_entries() {
        let s = Settings::parse(
            r##"{"theme_accents": {"dawn": "#c2366f", "nord": 12, "sunrise": "teal"},
                "reduce_motion": "yes", "game_args": "--x"}"##,
        );
        assert_eq!(s.game_args, "--x");
        assert_eq!(s.reduce_motion, None);
        assert_eq!(s.theme_accents.len(), 2);
        assert_eq!(s.theme_accents["twilight"], crate::themes::Accent::TEAL);
        let s = Settings::parse(r#"{"theme_accents": [1], "reduce_motion": true}"#);
        assert!(s.theme_accents.is_empty());
        assert_eq!(s.reduce_motion, Some(true));
        let saved = serde_json::to_string(&Settings::default()).unwrap();
        assert!(!saved.contains("theme_accents") && !saved.contains("reduce_motion"));
    }

    #[test]
    fn reduced_graphics_round_trips_the_old_performance_key() {
        for (text, expected) in [
            (r#"{"performance":true,"game_args":"--x"}"#, Some(true)),
            (r#"{"performance":false,"game_args":"--x"}"#, Some(false)),
            (r#"{"performance":null,"game_args":"--x"}"#, None),
            (r#"{"performance":"fast","game_args":"--x"}"#, None),
            (r#"{"game_args":"--x"}"#, None),
        ] {
            let s = Settings::parse(text);
            assert_eq!(s.reduced_graphics, expected, "{text}");
            assert_eq!(s.reduced_graphics(), expected.unwrap_or(false));
            assert_eq!(s.game_args, "--x");
            let saved = serde_json::to_value(&s).unwrap();
            assert_eq!(saved.get("performance").and_then(|v| v.as_bool()), expected);
            assert!(!saved.as_object().unwrap().contains_key("reduced_graphics"));
        }
    }

    /// Sunrise now names the orange theme. A player who chose the indigo one while it was
    /// called Sunrise keeps it, with the accent they picked under it; one on the orange theme,
    /// saved as `dawn` or not at all, stays there.
    #[test]
    fn the_indigo_theme_saved_as_sunrise_is_read_as_twilight() {
        use crate::themes::{self, Accent};
        let old = Settings::parse(
            r##"{"theme": "sunrise", "theme_accents": {"sunrise": "pink", "dawn": "teal"}}"##,
        );
        assert_eq!(old.theme.as_deref(), Some("twilight"));
        assert_eq!(
            old.accent,
            Accent::PINK,
            "the accent kept under it comes along"
        );
        assert_eq!(old.theme_accents["twilight"], Accent::PINK);
        assert!(!old.theme_accents.contains_key("sunrise"));
        assert_eq!(old.theme_accents["dawn"], Accent::TEAL);
        assert!(matches!(
            themes::resolve(old.theme.as_deref()),
            themes::Resolved::Twilight
        ));
        // Without an accent of the player's it is the violet it always was.
        assert_eq!(
            Settings::parse(r#"{"theme": "sunrise"}"#).accent,
            Accent::VIOLET
        );
        // Saved again it says twilight, and reads the same from then on.
        let saved = serde_json::to_string(&old).unwrap();
        assert!(
            saved.contains(r#""theme":"twilight""#) && !saved.contains("sunrise"),
            "{saved}"
        );
        let again = Settings::parse(&saved);
        assert_eq!((again.theme, again.accent), (old.theme, old.accent));
        // The orange one is untouched: saved as dawn it stays dawn, and none saved is the same.
        for text in [r#"{"theme": "dawn"}"#, "{}"] {
            let orange = Settings::parse(text);
            assert_eq!(orange.accent.rgb(), 0xc2552d, "{text}");
            assert!(matches!(
                themes::resolve(orange.theme.as_deref()),
                themes::Resolved::Builtin(t) if t.id == "dawn" && t.name == "Sunrise"
            ));
        }
        assert_eq!(
            Settings::parse(r#"{"theme": "dawn"}"#).theme.as_deref(),
            Some("dawn")
        );
        // A newer accent already kept under twilight wins over the old key.
        let both =
            Settings::parse(r##"{"theme_accents": {"sunrise": "pink", "twilight": "green"}}"##);
        assert_eq!(both.theme_accents["twilight"], Accent::GREEN);
    }

    /// A cat value that isn't on or off is dropped alone: the folder, the arguments and every
    /// other choice are read, and are still there after the next save.
    #[test]
    fn a_cat_that_is_not_a_switch_loses_only_itself_through_a_load_and_a_save() {
        let saved = r##"{"cat": "off", "minecraft_dir": "/games/minecraft", "game_args": "--x",
            "theme": "nord", "accent": "teal", "keep_versions": 5}"##;
        let mut s = Settings::parse(saved);
        assert_eq!(s.cat, None);
        assert!(s.show_cat(), "unset is the default: the cat shows");
        assert_eq!(
            s.minecraft_dir.as_deref(),
            Some(std::path::Path::new("/games/minecraft"))
        );
        // The next edit writes everything that was read.
        s.skip_opening = true;
        let written = serde_json::to_string(&s).unwrap();
        let again = Settings::parse(&written);
        assert_eq!(
            again.minecraft_dir.as_deref(),
            Some(std::path::Path::new("/games/minecraft"))
        );
        assert_eq!(again.game_args, "--x");
        assert_eq!(again.theme.as_deref(), Some("nord"));
        assert_eq!(again.accent, crate::themes::Accent::parse("teal").unwrap());
        assert_eq!(again.keep_versions, Some(5));
        assert!(again.skip_opening);
        // A real choice is kept as it is.
        assert_eq!(Settings::parse(r#"{"cat": false}"#).cat, Some(false));
        assert_eq!(
            Settings::parse(r#"{"cat": null, "game_args": "--y"}"#).game_args,
            "--y"
        );
    }

    /// Panel opacity, glass and blur are gone: a file that still carries them is read as before,
    /// whatever their values, and the next save leaves them out.
    #[test]
    fn a_file_with_the_removed_look_settings_still_loads() {
        for old in [
            r#"{"panel_opacity":40,"glass":12,"blur":8.5,"theme":"nord","game_args":"--x"}"#,
            r#"{"panel_opacity":"x","glass":"frosty","blur":null,"theme":"nord","game_args":"--x"}"#,
        ] {
            let read = Settings::parse(old);
            assert_eq!(read.theme.as_deref(), Some("nord"), "{old}");
            assert_eq!(read.game_args, "--x", "{old}");
            let saved = serde_json::to_value(&read).unwrap();
            for key in ["panel_opacity", "glass", "blur"] {
                assert!(saved.get(key).is_none(), "{key} is not written again");
            }
        }
    }
}
