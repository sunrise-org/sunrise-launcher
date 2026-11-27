//! The Skins tab: how the account that plays looks. A 3D preview of the player (drag to turn
//! it) beside the choices: the skin (the one worn now, the saved ones, the game's default
//! skins, or a new PNG), wide or slim arms, and the cape (none, or one of the Minecraft capes
//! the account owns). Choices show on the preview at once; Save sends them to the Minecraft
//! services, which every client and server then shows.
//!
//! The skin, its arms and the cape are separate edits: one not touched is never sent, and a
//! cape the page doesn't know yet (the profile still being read) is never taken for "no cape".
//! A skin chosen is the picture clicked ([`SkinPick`]): the one an account wore at a URL, a
//! saved skin, a default skin; never "whatever is worn" resolved later. What the account wears
//! comes from one reading of its profile, made each time the page opens, so a change made in
//! another launcher shows here.
//!
//! Each account has its own page ([`SkinsPages`]): switching to another account and back finds
//! the first one as it was left, a Save still under way included, so an account never has two
//! Saves at once, and a Save that ends while another account is shown is reconciled with its
//! own account's page. Choices made while a Save runs are kept as they are, and compared with
//! what the account wears only once the Save has said what that is.
//!
//! Every request runs off the UI thread and comes back as a message (`Msg::SkinsProfile`,
//! `Picture`, `LookSaved`, `SkinsAdded`, `SkinFile`, `SkinKept`) carrying the account it was
//! for and, for a reading or a Save, its own number, unique for the whole run: an answer for
//! another account, or one a later request replaced, changes nothing on the page. Pictures from
//! Mojang and files from disk go through two [`Queue`]s, one thread each with a bounded queue;
//! the Save itself is [`account::look::save`].
//!
//! In an offline developer session the page only shows a player in the account's default skin
//! (the game's offline look): no requests, no edits. The mode is the one in force now, read at
//! every drawing, choice and answer, not the one the page was opened in: an answer to a
//! request made online is still taken in (a Save that went through is recorded), but starts
//! nothing.
use super::*;
use crate::frame::Reach as _;
use crate::paint::palette;
use account::look::{self, Change, Kept};
use gpui::{img, rgb, ExternalPaths, MouseButton, MouseDownEvent, MouseMoveEvent, RenderImage};
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::Arc;
use sunrise_launcher::account::capes::{self, Profile};
use sunrise_launcher::player_model::{self, Pose, Texture};
use sunrise_launcher::skin_library::{self, Saved};

mod edits;
mod jobs;
mod library;
mod pages;
mod profile;
mod view;
use edits::*;
use jobs::*;
use library::*;
pub(crate) use pages::SkinsPages;
use pages::*;

/// The preview's size, in points.
const PREVIEW_W: f32 = 280.;
const PREVIEW_H: f32 = 420.;
/// A tile's picture: a skin's model or a cape's face (10 by 16 texels at 6 points).
const PICTURE_W: f32 = 60.;
const PICTURE_H: f32 = 96.;
const TILE_W: f32 = 88.;
/// The game's default skins, each with wide and slim arms.
const DEFAULT_SKINS: [&str; 9] = [
    "steve", "alex", "ari", "efe", "kai", "makena", "noor", "sunny", "zuri",
];

#[cfg(test)]
mod tests;
