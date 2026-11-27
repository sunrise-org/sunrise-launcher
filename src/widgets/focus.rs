//! What every control shares: the keyboard's focus ring, [`Reach`] for things that are not
//! buttons, and the colour helpers.

use super::*;
use crate::paint::palette;
use gpui::rgb;

/// The keyboard's ring, in `color`, around a filled control that has no edge of its own to turn
/// gold. Behind a see-through control it would show as a fill: give that one an edge instead.
pub(crate) fn focus_ring(color: u32) -> Vec<gpui::BoxShadow> {
    vec![gpui::BoxShadow {
        color: rgb(color).into(),
        offset: gpui::point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(2.),
        inset: false,
    }]
}

/// What makes a clickable thing that is not a button reachable by keyboard: a Tab stop that shows
/// the ring. Enter and Space press it, as a click does.
pub(crate) trait Reach {
    /// For a thing with no edge of its own: a gold line just inside it, which takes no room and
    /// is never cut off by what it sits in.
    fn reach(self) -> Self;
    /// For a thing that has an edge: that edge turns gold.
    fn reach_edge(self) -> Self;
    /// For every Tab stop: the page it is on scrolls to show it when the keyboard lands on it.
    fn seen(self) -> Self;
}

impl<T: InteractiveElement + ParentElement + Styled> Reach for T {
    fn reach(self) -> Self {
        self.tab_index(0)
            .rounded(crate::tokens::r(crate::tokens::R_SMALL))
            .focus_visible(|s| {
                s.shadow(vec![gpui::BoxShadow {
                    inset: true,
                    spread_radius: px(1.),
                    ..focus_ring(palette().focus).remove(0)
                }])
            })
            .seen()
    }

    fn reach_edge(self) -> Self {
        self.tab_index(0)
            .focus_visible(|s| s.border_color(rgb(palette().focus)))
            .seen()
    }

    fn seen(self) -> Self {
        // The toolkit tells nobody where the focused control is. So a focused control marks the
        // text style it hands its children (a background nothing can see), and a child that
        // covers it reads the mark while the frame is laid out, when it knows its place.
        self.focus(|s| s.text_bg(focus_mark())).child(
            gpui::canvas(super::scrolls::scroll_to_show, |_, _, _, _| {})
                .absolute()
                .inset_0(),
        )
    }
}

pub(crate) fn c(x: u32) -> gpui::Hsla {
    gpui::rgb(x).into()
}

pub(crate) fn mix(a: gpui::Hsla, b: gpui::Hsla, v: f32) -> gpui::Hsla {
    let (a, b) = (gpui::Rgba::from(a), gpui::Rgba::from(b));
    let m = |x: f32, y: f32| x + (y - x) * v;
    gpui::Hsla::from(gpui::Rgba {
        r: m(a.r, b.r),
        g: m(a.g, b.g),
        b: m(a.b, b.b),
        a: m(a.a, b.a),
    })
}

pub(crate) fn focus_mark() -> gpui::Hsla {
    gpui::hsla(0.125, 0.5, 0.25, 0.)
}
