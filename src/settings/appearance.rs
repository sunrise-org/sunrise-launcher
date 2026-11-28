//! Settings › Appearance: the look (Sunrise or Light), every colour of the theme as the player
//! sets it, the library of saved and imported themes with their share codes, what is behind
//! Home, and the themes folder.
//!
//! Appearance changes use the same settings as the command line. The themes
//! folder is read at startup and on Reload, and its pictures decode off the window's thread.
use super::*;

mod actions;
mod colours;
mod handlers;
mod inputs;
mod photo;
mod rows;
mod state;
#[cfg(test)]
mod tests;
mod theme_library;

pub(crate) use crate::paint::HomePhoto;
pub(crate) use state::State;
