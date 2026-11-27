//! Small pieces the three Mods screens share. Colours are the launcher's (`crate::frame`).
use super::*;
use crate::paint::palette;
use std::path::PathBuf;
use std::rc::Rc;

pub(super) use crate::frame::c;

pub(super) fn muted(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(tokens::SMALL))
        .text_color(c(palette().muted))
        .child(text.into())
}

/// `muted` text a reader may want to copy: an error, a reason, a note.
pub(super) fn note(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(tokens::SMALL))
        .text_color(c(palette().muted))
        .child(crate::selectable::selectable(text))
}

/// A control that can't be used now: drawn dim, with no handler.
pub(super) fn dim(b: Stateful<Div>) -> Stateful<Div> {
    b.opacity(0.45).cursor_default().tab_stop(false)
}

pub(super) use crate::mod_icons::tile;

/// How many cards fit side by side in a Discover list `width` wide: three on a roomy window,
/// fewer as it narrows, so a card never gets narrower than its own text needs.
pub(super) fn columns(width: f32) -> u16 {
    (((width + 12.) / 332.).floor() as u16).clamp(1, 3)
}

/// Prefer the catalogue picture, then the exact installed package's validated artwork.
pub(super) fn mod_artwork(
    catalogue: Option<PathBuf>,
    package: Option<std::sync::Arc<gpui::Image>>,
) -> Option<gpui::ImageSource> {
    catalogue
        .map(Into::into)
        .or_else(|| package.map(Into::into))
}

/// One thing to add in a Discover list, for mods and packs alike: artwork, name and author, one
/// line of what it is, then its tags, with the numbers (`stats`) and the control in a column on
/// the right, and under the tags a sentence when there is something to say (why it can't be
/// added, how well it runs). Every text shortens or wraps inside the card, whatever its width.
#[allow(clippy::too_many_arguments)]
pub(super) fn browse_card(
    id: impl Into<ElementId>,
    picture: Option<impl Into<gpui::ImageSource>>,
    project: &str,
    name: &str,
    author: &str,
    summary: &str,
    facts: Vec<Div>,
    stats: Option<Div>,
    control: Option<AnyElement>,
    note: Option<String>,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_start()
        .gap(px(14.))
        .p(px(12.))
        .min_w_0()
        .overflow_hidden()
        .rounded(crate::tokens::r(crate::tokens::R_CARD))
        .border_1()
        .border_color(palette().input_edge())
        .bg(palette().card())
        .cursor_pointer()
        .hover(|s| s.border_color(c(palette().muted)))
        .tab_index(0)
        .seen()
        .focus_visible(|s| s.border_color(c(palette().focus)))
        .child(tile(picture, project, name, 64.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(8.))
                        .min_w_0()
                        .child(
                            div()
                                .min_w_0()
                                .font_weight(FontWeight::SEMIBOLD)
                                .truncate()
                                .child(name.to_owned()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(tokens::SMALL))
                                .text_color(c(palette().muted))
                                .truncate()
                                .child(author.to_owned()),
                        ),
                )
                .child(
                    div()
                        .text_size(px(tokens::LABEL))
                        .text_color(c(palette().muted))
                        .line_clamp(2)
                        .text_ellipsis()
                        .child(summary.to_owned()),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_x(px(8.))
                        .gap_y(px(4.))
                        .children(facts),
                )
                // Two lines at most: the whole sentence is the tooltip.
                .children(note.map(|note| {
                    muted(note.clone())
                        .id("note")
                        .line_clamp(2)
                        .text_ellipsis()
                        .tooltip(crate::glass_ui::tip(note))
                })),
        )
        .child(
            div()
                .flex_none()
                .w(px(136.))
                .flex()
                .flex_col()
                .items_end()
                .gap(px(8.))
                .children(stats.map(|stats| {
                    stats
                        .text_size(px(tokens::SMALL))
                        .text_color(c(palette().muted))
                }))
                .children(control.map(|c| div().flex_none().child(c))),
        )
}

/// "Installed", where a card's control would be.
pub(super) fn added_mark() -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(5.))
        .text_size(px(tokens::LABEL))
        .text_color(c(palette().success))
        .child(blocks::icon("check", 12., c(palette().success)))
        .child(tr("Installed"))
        .into_any_element()
}

/// A row's quiet status: a small glyph in `color` and a few words, never a sentence.
pub(super) fn status(icon: &'static str, label: impl Into<SharedString>, color: u32) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(tokens::LABEL))
        .text_color(c(palette().muted))
        .child(crate::glass_ui::icon(icon, 14., c(color)))
        .child(label.into())
}

pub(super) fn card() -> Div {
    div()
        .rounded(crate::tokens::r(crate::tokens::R_CARD))
        .border_1()
        .border_color(palette().rule())
        .bg(palette().card())
}

/// The theme the shared text field draws with, in this look's colours.
pub(crate) fn field_theme(mut t: crate::theme::Theme) -> crate::theme::Theme {
    t.ink = c(palette().text);
    t.muted = c(palette().muted);
    t.glass = false;
    t
}

/// For the row or control a page was opened at: moves `scroll` until the row is in view, puts the
/// keyboard on `focus` when it has one, and then marks `done`. Laid over the row, it takes no
/// room.
pub(crate) fn reveal(
    scroll: gpui::ScrollHandle,
    done: Rc<std::cell::Cell<bool>>,
    focus: Option<FocusHandle>,
) -> impl IntoElement {
    reveal_in(scroll, done, focus, None)
}

/// Like [`reveal`], for a row inside a culled list whose unmeasured heights can shrink or grow
/// while scrolling to it: adjusts the scroll offset until the row is visible and `cull` has
/// settled, and only then marks `done`.
pub(super) fn reveal_in(
    scroll: gpui::ScrollHandle,
    done: Rc<std::cell::Cell<bool>>,
    focus: Option<FocusHandle>,
    cull: Option<crate::cull::Cull>,
) -> impl IntoElement {
    const ROOM: f32 = 24.;
    gpui::canvas(
        move |row: Bounds<gpui::Pixels>, window, cx| {
            if done.get() {
                return;
            }
            // A frame being drawn can't ask for the next one itself: that is asked after it.
            let again = |window: &mut Window, cx: &mut App| {
                let window = window.window_handle();
                cx.defer(move |cx| {
                    window.update(cx, |_, window, _| window.refresh()).ok();
                });
            };
            let view = scroll.bounds();
            // The page is not laid out before its first frame: wait for the next.
            if view.size.height <= px(0.) {
                return again(window, cx);
            }
            let settled = cull.as_ref().is_none_or(crate::cull::Cull::settled);
            let lift = if row.top() < view.top() {
                if row.bottom() > view.bottom() {
                    px(0.)
                } else {
                    row.top() - view.top() - px(ROOM)
                }
            } else if row.bottom() > view.bottom() {
                if row.size.height > view.size.height {
                    row.top() - view.top()
                } else {
                    row.bottom() - view.bottom() + px(ROOM)
                }
            } else {
                px(0.)
            };
            if lift != px(0.) {
                let at = scroll.offset();
                let max_y = scroll.max_offset().y;
                let target_y = (at.y - lift).clamp(-max_y, px(0.));
                if (target_y - at.y).abs() > px(0.5) {
                    scroll.set_offset(gpui::point(at.x, target_y));
                    return again(window, cx);
                }
                if !settled {
                    return again(window, cx);
                }
            } else if !settled {
                return again(window, cx);
            }
            done.set(true);
            if let Some(focus) = &focus {
                focus.focus(window, cx);
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_artwork_prefers_catalogue_then_package_then_placeholder() {
        let listed = PathBuf::from("source-icon.png");
        let package = std::sync::Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, vec![]));
        assert!(matches!(
            mod_artwork(Some(listed), Some(package.clone())),
            Some(gpui::ImageSource::Resource(_))
        ));
        assert!(
            matches!(mod_artwork(None, Some(package.clone())), Some(gpui::ImageSource::Image(image)) if std::sync::Arc::ptr_eq(&image, &package))
        );
        assert!(mod_artwork(None, None).is_none());
    }
}
