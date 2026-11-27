//! Hosting in the new look: the servers this computer hosts, and one of them at a time with its
//! address, players, settings, backups and log. Every action is the Servers page's own
//! (`servers_page.rs`); only the drawing is new.
use super::*;
use crate::paint::palette;
use crate::settings::{group, hint, off, row};
use crate::widgets::check::switch;
use crate::widgets::choice::choices;
use gpui::rgb;

mod create;
mod page;
mod settings;
mod status;
mod widgets;
