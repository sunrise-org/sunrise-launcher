//! The launcher's frame in the night look (docs/design-system.md, "Night"): the sidebar's plate
//! and rows, and the top bar's tabs, icon buttons and account chip. The sky shows between them;
//! only the sidebar and the selected or hovered parts are plates.
use gpui::{rgb, Hsla};

/// A night colour as GPUI's.
#[cfg_attr(not(windows), allow(dead_code))] // the Windows setup window
pub fn c(color: u32) -> Hsla {
    rgb(color).into()
}
