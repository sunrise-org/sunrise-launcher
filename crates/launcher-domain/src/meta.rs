//! The player's saved choices about one instance (`instances.json`), and the tolerant readers that
//! keep one bad field from invalidating the rest.
use crate::icon::{is_texture_name, Icon};
use crate::{Channel, Launch, Profile};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The player's choices about one instance.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(
        default,
        deserialize_with = "saved_icon",
        skip_serializing_if = "Option::is_none"
    )]
    pub icon: Option<Icon>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Where the player put it among its group's profiles in the sidebar (smaller is higher).
    /// Missing, or unreadable: after the ordered ones, by name.
    #[serde(
        default,
        deserialize_with = "saved_order",
        skip_serializing_if = "Option::is_none"
    )]
    pub order: Option<u32>,
    /// The Sunrise version this instance plays, kept when newer ones are installed. None: the
    /// launcher's own version, which follows updates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The builds "latest" means for it; missing is [`Channel::Nightly`].
    #[serde(
        default,
        deserialize_with = "saved_channel",
        skip_serializing_if = "Option::is_none"
    )]
    pub channel: Option<Channel>,
    /// The last committed switch build choice already reflected (or superseded) here.
    #[serde(
        default,
        deserialize_with = "saved_choice_id",
        skip_serializing_if = "Option::is_none"
    )]
    build_choice_applied: Option<[u8; 16]>,
    /// This instance's saved native build choice; missing or unknown starts at Optimized.
    #[serde(
        default,
        deserialize_with = "saved_profile",
        skip_serializing_if = "Option::is_none"
    )]
    pub native_profile: Option<Profile>,
    /// Seconds played in it, every run started by the launcher (the window or `play`) added up.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub played_secs: u64,
    /// When its last run ended, in seconds since 1970.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_played: Option<u64>,
    /// What runs around the game: a pre-launch command, a wrapper, variables (`launch`).
    #[serde(default, skip_serializing_if = "Launch::is_empty")]
    pub launch: Launch,
    /// Set by an earlier launcher for a Minecraft: Java Edition instance. This launcher no
    /// longer starts those: such an instance is not listed, and its record is kept exactly as it
    /// was, whatever it holds, whenever `instances.json` is saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java: Option<serde_json::Value>,
    /// The same earlier launcher's put-aside Java settings of an instance that played Sunrise
    /// meanwhile: kept as it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java_kept: Option<serde_json::Value>,
    #[serde(flatten)]
    other: BTreeMap<String, serde_json::Value>,
}

impl Meta {
    /// Whether the committed build choice `id` is already reflected here.
    pub fn reflects_build_choice(&self, id: [u8; 16]) -> bool {
        self.build_choice_applied == Some(id)
    }

    /// Records that the committed build choice `id` is now reflected here.
    pub fn note_build_choice(&mut self, id: [u8; 16]) {
        self.build_choice_applied = Some(id);
    }
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

// An unknown channel must not invalidate unrelated instance metadata.
fn saved_channel<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Channel>, D::Error> {
    Ok(serde_json::from_value(serde_json::Value::deserialize(deserializer)?).ok())
}

fn saved_order<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<u32>, D::Error> {
    Ok(serde_json::from_value(serde_json::Value::deserialize(deserializer)?).ok())
}

fn saved_choice_id<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<[u8; 16]>, D::Error> {
    Ok(serde_json::from_value(serde_json::Value::deserialize(deserializer)?).ok())
}

// A malformed preference must not invalidate unrelated instance metadata.
fn saved_profile<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Profile>, D::Error> {
    Ok(serde_json::from_value(serde_json::Value::deserialize(deserializer)?).ok())
}

// A saved icon is held to what the dialog and the command line take; one that isn't is no icon,
// and never a reason to lose the instance's other choices.
fn saved_icon<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Icon>, D::Error> {
    let saved: Option<Icon> =
        serde_json::from_value(serde_json::Value::deserialize(deserializer)?).ok();
    Ok(saved.filter(|icon| match icon {
        Icon::Texture(name) => is_texture_name(name),
        Icon::Colour(hex) => Icon::parse(hex).as_ref() == Ok(icon),
    }))
}
