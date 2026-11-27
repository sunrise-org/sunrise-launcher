//! The look's tokens: its type scale, its corner radii and the few sizes the whole
//! window shares. A screen names one of these, never a number of its own, so the scale changes
//! here and nowhere else. Sizes are logical pixels.
//!
//! Spacing (gaps and padding) and a single screen's own layout (a column's width, a picture's
//! height) are not tokens: they say how one screen is laid out, not what the look is.

thread_local! {
    /// The player's corner rounding (Settings › Appearance), as a factor of the radii below.
    static ROUNDNESS: std::cell::Cell<f32> = const { std::cell::Cell::new(1.) };
}

/// Set as each frame starts being built.
pub(crate) fn set_roundness(factor: f32) {
    ROUNDNESS.set(factor);
}

/// The player's corner rounding, as a factor of the look's radii (1 as drawn).
pub(crate) fn roundness() -> f32 {
    ROUNDNESS.get()
}

/// A corner `radius` as the player wants corners: every rounded corner goes through this.
pub(crate) fn r(radius: f32) -> gpui::Pixels {
    gpui::px(radius * ROUNDNESS.get())
}

// ---- type scale

/// Tags, counts and the rail's labels.
pub(crate) const CAPTION: f32 = 11.;
/// A second line under a name; hints.
pub(crate) const SMALL: f32 = 12.;
/// Controls, and the muted sentence under a heading.
pub(crate) const LABEL: f32 = 13.;
/// What the window is set in.
pub(crate) const BODY: f32 = 14.;
/// Play.
pub(crate) const LEAD: f32 = 16.;
/// A section of a page.
pub(crate) const HEADING: f32 = 18.;
/// A sheet's title.
pub(crate) const TITLE: f32 = 24.;
/// A page's title.
pub(crate) const DISPLAY: f32 = 28.;
/// The profile's name in its banner.
pub(crate) const HERO: f32 = 40.;

// ---- corner radii

/// Progress bars and other hairlines.
pub(crate) const R_BAR: f32 = 2.;
/// Checkboxes, tags and small tiles.
pub(crate) const R_SMALL: f32 = 4.;
/// Buttons, fields and chips (chips are not pills).
pub(crate) const R_CONTROL: f32 = 6.;
/// Cards and the rows' pictures.
pub(crate) const R_CARD: f32 = 8.;
/// Sheets, and the page itself.
pub(crate) const R_PANEL: f32 = 12.;

// ---- shared sizes

/// A button's height.
pub(crate) const CONTROL: f32 = 32.;
/// A text field's height, and a tab's.
pub(crate) const FIELD: f32 = 36.;
/// A page's or a sheet's main action.
pub(crate) const ACTION: f32 = 44.;
/// The height of the frame's tab and account row.
pub(crate) const HEADER: f32 = 54.;
/// The equal inset of the sidebar and content from the window edges.
pub(crate) const FRAME_INSET: f32 = 12.;
/// Play, and the build picker beside it.
pub(crate) const PLAY: f32 = 46.;
/// How far a new page rises as it arrives.
pub(crate) const RISE: f32 = sunrise_launcher_theme::RISE;
