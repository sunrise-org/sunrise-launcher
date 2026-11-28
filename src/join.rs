//! The servers of the selected profile as cards, for Home's Saved servers tab (`home::tabs::servers`
//! lays the tab out): each with Join and a "…" menu.
//!
//! Everything shown is read from the player's real launcher data, and nothing is written until
//! the player acts (Add, Edit, Move, Remove, Join). Join passes the destination to
//! `play_game`, which owns the sign-in, install, prepare, build and trust gates:
//! - servers: the game's own `servers.dat` (`server_list`) with the status ping the Multiplayer
//!   screen sends; an invite code joins through `join_friend`. The dialog "Add server" opens
//!   saves an address, or joins at once without saving (that is where quick join went);
//! - featured servers: a bundled pair, Horizon Minecraft (joinable once its address is set)
//!   and Sunrise's own (inert, coming soon); no backend service.
//!
//! - `cards`: a saved server's card and a featured one's, and the selection bar.
//! - `dialog`: the "…" menu, the form and the question before a removal.
use super::*;
use gpui::{rgb, rgba};

mod cards;
mod dialog;

/// Horizon Minecraft's address, passed to the game as written. Empty: the row has no Join.
pub(crate) const HORIZON_ADDRESS: &str = "play.playhorizon.dev";

/// A server Sunrise features, bundled: there is no service to ask.
#[derive(Debug, PartialEq)]
pub(crate) struct Featured {
    pub name: &'static str,
    /// None: it can't be joined (yet).
    pub address: Option<&'static str>,
    pub note: &'static str,
    /// What an entry without an address says.
    pub inert: &'static str,
    /// The picture it wears when the server sends none of its own.
    pub badge: Badge,
}

/// A featured server's bundled picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Badge {
    /// Horizon's site icon.
    Horizon,
    /// The launcher's own mark.
    Sunrise,
}

/// Horizon's icon as its site draws it: an H on dark purple, 16 cells a side, painted as
/// rectangles so it stays sharp at any size.
fn horizon_mark(size: f32) -> Div {
    const CELLS: [(f32, f32, f32, f32, u32); 5] = [
        (4., 3., 2., 4., 0xffffffff),
        (10., 3., 2., 4., 0xffffffff),
        (4., 8., 2., 4., 0xffffff66),
        (10., 8., 2., 4., 0xffffff66),
        (1., 7., 14., 1., 0xa98fffff),
    ];
    let u = size / 16.;
    let mut tile = div()
        .relative()
        .flex_none()
        .size(px(size))
        .rounded(px(3. * u))
        .bg(rgb(0x100a1f));
    for (x, y, w, h, color) in CELLS {
        tile = tile.child(
            div()
                .absolute()
                .left(px(x * u))
                .top(px(y * u))
                .w(px(w * u))
                .h(px(h * u))
                .bg(rgba(color)),
        );
    }
    tile
}

pub(crate) fn featured() -> Vec<Featured> {
    featured_with(HORIZON_ADDRESS)
}

fn featured_with(horizon: &'static str) -> Vec<Featured> {
    vec![
        Featured {
            name: crate::i18n::tr_key("Horizon Minecraft"),
            address: Some(horizon).filter(|a| !a.is_empty()),
            note: crate::i18n::tr_key("A Horizon community server"),
            inert: crate::i18n::tr_key("Address coming soon"),
            badge: Badge::Horizon,
        },
        Featured {
            name: crate::i18n::tr_key("Sunrise server"),
            address: None,
            note: crate::i18n::tr_key("Sunrise's own server"),
            inert: crate::i18n::tr_key("Coming soon"),
            badge: Badge::Sunrise,
        },
    ]
}

/// What the address box holds.
#[derive(Debug, PartialEq)]
pub(crate) enum Entry {
    Empty,
    Invite,
    Address(String),
    /// Spaces or the like inside: neither.
    Unusable,
}

pub(crate) fn entry(text: &str) -> Entry {
    let text = text.trim();
    if text.is_empty() {
        Entry::Empty
    } else if sunrise_launcher_friends::invite::parse(text).is_some() {
        Entry::Invite
    } else if text.chars().any(|c| c.is_whitespace() || c.is_control()) {
        Entry::Unusable
    } else {
        Entry::Address(text.to_string())
    }
}

/// The first letter of `name`, upper case, for a plain tile.
pub(crate) fn initial(name: &str) -> String {
    name.trim()
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "?".into())
}

/// The host an address names, without its port, in lower case.
fn host(address: &str) -> Option<String> {
    sunrise_launcher_telemetry::server_status::split_address(address)
        .map(|(host, _)| host.to_ascii_lowercase())
}

/// The featured server at `address`'s host, whatever port or letter case it was saved with.
pub(crate) fn featured_at(address: &str) -> Option<Featured> {
    let host = host(address)?;
    featured()
        .into_iter()
        .find(|f| f.address.and_then(self::host).as_ref() == Some(&host))
}

/// A server's picture: its own, else the bundled one of a featured server, else a tile with its
/// first letter, `size` square.
fn server_picture(
    icon: Option<std::sync::Arc<gpui::Image>>,
    badge: Option<Badge>,
    (address, name): (&str, &str),
    size: f32,
) -> AnyElement {
    match (icon, badge) {
        (None, Some(Badge::Horizon)) => horizon_mark(size).into_any_element(),
        (None, Some(Badge::Sunrise)) => crate::mark(size).into_any_element(),
        (icon, _) => crate::mod_icons::tile(icon, address, name, size).into_any_element(),
    }
}

#[cfg(test)]
mod tests;
