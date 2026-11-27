//! Mods › Discover: the page's own list and its controls, in parts: the catalogue and the
//! page's shape (`catalog`), filters and the summary (`filters`), results and collections
//! (`results`), and the cards' facts (`cards`).
use super::model::discover_v2::{self, V2Sort};
use super::state::Menu;
use super::ui::*;
use super::*;
use crate::paint::palette;
use crate::widgets::button::{primary, secondary};
use crate::widgets::check::switch;
use crate::widgets::chip::{self, tag};
use crate::widgets::feedback::empty_state;
use gpui::{deferred, MouseButton};
use sunrise_launcher::browser::details::category_label;
use sunrise_launcher::browser::images;

mod cards;
mod catalog;
mod filters;
mod results;

pub(super) use cards::install_fact;
use cards::{fit_pill, unfit_pill};
use filters::summary_count_line;
pub(super) use filters::KindTab;

/// How many cards are drawn before "Show more".
const PAGE: usize = 36;

/// What choosing a menu entry does.
pub(super) type Pick = Box<dyn Fn(&mut Launcher)>;

pub(super) type MenuOption = (SharedString, bool, Pick);
