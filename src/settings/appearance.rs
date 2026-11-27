//! Settings › Appearance: the theme (the built-in ones, then the player's own files), the
//! accent under it, what is behind Home, and the themes folder.
//!
//! Appearance changes use the same settings as the command line. The opacity slider only
//! previews during a drag; a failed save on release uses the ordinary Retry notice. The themes
//! folder is read at startup and on Reload, and its pictures decode off the window's thread.
use super::*;

mod handlers;
mod photo;
mod rows;
mod state;
#[cfg(test)]
mod tests;

pub(crate) use crate::paint::HomePhoto;
pub(crate) use state::State;
