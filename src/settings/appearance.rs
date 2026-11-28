//! Settings › Appearance: the theme gallery (the built-in themes and the player's saved and
//! imported ones, with their share codes), the current theme's colours as the player sets them,
//! what is behind Home, and the themes folder.
//!
//! Appearance changes use the same settings as the command line. The themes
//! folder is read at startup and on Reload, and its pictures decode off the window's thread.
use super::*;

mod actions;
mod colours;
mod dialogs;
mod edit;
mod edit_dialog;
mod handlers;
mod inputs;
mod photo;
mod preview;
mod rows;
mod state;
#[cfg(test)]
mod tests;
mod theme_library;
mod tiles;

pub(crate) use crate::paint::HomePhoto;
pub(crate) use state::State;
