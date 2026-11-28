//! Shared helpers of the installed mods' rows: their labels and the "not on this build" words.
//! Mods › Installed itself is gone; Home's Mods tab lists them.
use super::model::{self, Row};
use super::ui::*;
use super::*;
use crate::paint::palette;
use std::collections::BTreeMap;

mod items;
mod page;
mod unfit;

pub(super) use items::remove_names;
