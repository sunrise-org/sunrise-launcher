//! Window chrome: Windows and Linux client-side decorations are custom drawn; macOS and Linux
//! server-side decorations use the platform's native chrome. The title bar itself is composed in
//! `frame/title_bar.rs`; this file supplies its platform parts: the drag area, the window
//! controls, the resize edges and the appearance.
use crate::theme::Theme;
use gpui::{App, Div, Stateful, TitlebarOptions, Window};
use sunrise_launcher::backend::Appearance;

/// The title bar's height in the launcher's frame.
pub const HEIGHT: f32 = 44.;

/// The setup window's bar: a label and the window controls, nothing else.
#[cfg(target_os = "windows")]
const SETUP_HEIGHT: f32 = 32.;

pub fn options() -> TitlebarOptions {
    TitlebarOptions {
        title: Some(sunrise_launcher::identity::WINDOW_TITLE.into()),
        // macOS and Windows place the GPUI content under the native title bar. Linux uses
        // client-side decorations below, so its title bar is transparent by construction.
        appears_transparent: cfg!(any(target_os = "macos", target_os = "windows")),
        ..Default::default()
    }
}

/// The bar's flexible middle, the window's drag area: no press handling of its own. The
/// platform parts (`drag_area`) add it.
fn plain() -> Stateful<Div> {
    use gpui::{div, prelude::*, px};

    div()
        .id("window-drag")
        .flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .items_center()
        .gap(px(8.))
}

/// The room the bar leaves at its edges for the native resize edges, where the bar is drawn by
/// the launcher (Windows; none when maximized).
#[cfg(target_os = "windows")]
pub fn inset(window: &Window) -> Option<f32> {
    (!window.is_maximized()).then_some(windows::RESIZE_INSET)
}

#[cfg(not(target_os = "windows"))]
pub fn inset(_: &Window) -> Option<f32> {
    None
}

/// The window's drag area: a press on it drags the window, and a double click zooms it.
#[cfg(target_os = "linux")]
pub fn drag_area(window: &mut Window, cx: &mut App) -> Stateful<Div> {
    use gpui::{
        canvas, prelude::*, Decorations, DispatchPhase, MouseButton, MouseMoveEvent, MouseUpEvent,
        Pixels, Point,
    };
    use std::cell::Cell;

    let area = plain();
    // A compositor-drawn frame moves its own window: the area is only a space.
    if matches!(window.window_decorations(), Decorations::Server) {
        return area;
    }
    let maximize = window.window_controls().maximize;
    let pressed_at = window.use_state(cx, |_, _| Cell::new(None::<Point<Pixels>>));
    let pressed_on_bar = pressed_at.clone();
    let pressed_elsewhere = pressed_at.clone();
    area.on_mouse_down(MouseButton::Left, move |ev, window, cx| {
        if maximize && ev.click_count == 2 {
            pressed_on_bar.read(cx).set(None);
            window.zoom_window();
        } else {
            pressed_on_bar.read(cx).set(Some(ev.position));
        }
    })
    .on_mouse_down_out(move |ev, _, cx| {
        if ev.button == MouseButton::Left {
            pressed_elsewhere.read(cx).set(None);
        }
    })
    .child(
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let released = pressed_at.clone();
                window.on_mouse_event(move |ev: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture && ev.button == MouseButton::Left {
                        released.read(cx).set(None);
                    }
                });
                let moved = pressed_at.clone();
                // Starting a native move on the press gives the compositor the pointer before
                // GPUI can see a second click. Wait for GPUI's drag distance (2 px).
                // Listen on the window so a fast drag can already be outside the title bar.
                window.on_mouse_event(move |ev: &MouseMoveEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture && ev.dragging() {
                        let start = moved.read(cx);
                        if start
                            .get()
                            .is_some_and(|p| (ev.position - p).magnitude() > 2.)
                        {
                            start.set(None);
                            window.start_window_move();
                        }
                    }
                });
            },
        )
        .absolute()
        .inset_0(),
    )
}

#[cfg(target_os = "windows")]
pub fn drag_area(_window: &mut Window, _cx: &mut App) -> Stateful<Div> {
    use gpui::{prelude::*, MouseButton, WindowControlArea};

    // GPUI maps this area to the native caption, so Windows drags and snaps it itself.
    plain()
        .window_control_area(WindowControlArea::Drag)
        // A double click maximises or restores. Handled here, the press is not passed on, so
        // the native caption cannot toggle the window a second time.
        .on_mouse_down(MouseButton::Left, |ev, window, cx| {
            if ev.click_count == 2 {
                window.zoom_window();
                cx.stop_propagation();
            }
        })
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn drag_area(_window: &mut Window, _cx: &mut App) -> Stateful<Div> {
    // macOS keeps the drag on its native title bar, under the traffic lights.
    plain()
}

/// The window controls the launcher draws at the bar's right end: Linux with client-side
/// decorations, and Windows. None where the platform draws them (or where no bar is drawn).
#[cfg(target_os = "linux")]
pub fn controls(theme: &Theme, window: &mut Window, _cx: &mut App) -> Option<Div> {
    use gpui::{div, prelude::*, Decorations};

    if matches!(window.window_decorations(), Decorations::Server) {
        return None;
    }
    let controls = window.window_controls();
    let mut row = div().flex().flex_none().items_center().h_full();
    if controls.minimize {
        row = row.child(linux_minimize(theme));
    }
    if controls.maximize {
        row = row.child(linux_maximize(theme, window.is_maximized()));
    }
    Some(row.child(linux_close(theme)))
}

/// A caption button on Linux: 46 px wide, the bar's full height.
#[cfg(target_os = "linux")]
fn linux_caption(id: &'static str) -> gpui::Stateful<Div> {
    use gpui::{div, prelude::*, px};

    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(px(46.))
        .h_full()
        .cursor_pointer()
}

#[cfg(target_os = "linux")]
fn linux_minimize(theme: &Theme) -> gpui::Stateful<Div> {
    use gpui::{div, prelude::*, px};

    let color = theme.ink_soft;
    let hover = theme.raised;
    linux_caption("window-minimize")
        .hover(move |style| style.bg(hover))
        .child(div().w(px(10.)).h(px(1.)).bg(color))
        .on_click(|_, window, _| window.minimize_window())
}

#[cfg(target_os = "linux")]
fn linux_maximize(theme: &Theme, maximized: bool) -> gpui::Stateful<Div> {
    use gpui::{div, prelude::*, px};

    let color = theme.ink_soft;
    let hover = theme.raised;
    let square = || {
        div()
            .absolute()
            .size(px(10.))
            .border_1()
            .border_color(color)
    };
    let icon = div().relative().size(px(14.));
    let icon = if maximized {
        icon.child(
            div()
                .absolute()
                .left(px(4.))
                .top_0()
                .size(px(10.))
                .border_t_1()
                .border_r_1()
                .border_color(color),
        )
        .child(square().left(px(1.)).top(px(3.)))
    } else {
        icon.child(square().left(px(2.)).top(px(2.)))
    };
    linux_caption("window-maximize")
        .hover(move |style| style.bg(hover))
        .child(icon)
        .on_click(|_, window, _| window.zoom_window())
}

#[cfg(target_os = "linux")]
fn linux_close(theme: &Theme) -> gpui::Stateful<Div> {
    use crate::blocks;
    use gpui::prelude::*;

    let close_hover = theme.page;
    let close_bg = theme.danger;
    linux_caption("window-close")
        .group("linux-window-close")
        .hover(move |style| style.bg(close_bg).text_color(close_hover))
        .child(
            blocks::icon("close", 14., theme.ink_soft)
                .group_hover("linux-window-close", move |style| {
                    style.text_color(close_hover)
                }),
        )
        .on_click(|_, window, cx| {
            if !crate::profile_delete::defer_shutdown(cx) {
                window.remove_window();
            }
        })
}

#[cfg(target_os = "windows")]
pub fn controls(_theme: &Theme, window: &mut Window, _cx: &mut App) -> Option<Div> {
    Some(windows::controls(window))
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn controls(_theme: &Theme, _window: &mut Window, _cx: &mut App) -> Option<Div> {
    None
}

/// The label the setup window's bar shows beside its mark.
#[cfg(target_os = "windows")]
fn label() -> Div {
    use gpui::{div, px, FontWeight, ParentElement, Styled};

    div()
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(13.))
        .line_height(px(16.))
        .child(sunrise_launcher::i18n::tr(
            sunrise_launcher::identity::WINDOW_TITLE,
        ))
}

/// The setup window's own bar (`setup_page`): the mark, its name and the window controls. Where
/// the platform draws its controls, there is no bar. The setup window is Windows-only.
#[cfg(target_os = "windows")]
pub fn render(theme: &Theme, window: &mut Window, cx: &mut App) -> Option<Div> {
    use crate::{mark, paint::palette};
    use gpui::{div, prelude::*, px};

    let controls = controls(theme, window, cx)?;
    let bar = div()
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .h(px(SETUP_HEIGHT))
        .font_family(sunrise_launcher_theme::glass::FONT)
        .when_some(inset(window), |bar, inset| bar.pt(px(inset)).px(px(inset)))
        .bg(palette().chrome_plate())
        .text_color(theme.muted)
        .border_b_1()
        .border_color(palette().rule())
        .child(
            drag_area(window, cx)
                .px(px(12.))
                .child(mark(16.))
                .child(label()),
        )
        .child(controls);
    Some(bar)
}

/// The edges a client-decorated Linux window is resized by: none when the compositor draws the
/// frame (it resizes the window itself) or the window is maximized.
#[cfg(target_os = "linux")]
pub fn resize_handles(window: &Window) -> Option<Div> {
    use gpui::Decorations;

    if matches!(window.window_decorations(), Decorations::Server) || window.is_maximized() {
        return None;
    }
    Some(resize_edges())
}

/// A thin grip along every edge and corner of the window, over what the window draws. It is
/// pinned to the window's bounds (`inset_0`): the frame's root lays its children out in a
/// block, so a box that only sized itself to the window sat below the page, outside the
/// window, and no edge took a press (#474).
#[cfg(target_os = "linux")]
fn resize_edges() -> Div {
    use gpui::{div, prelude::*, px, CursorStyle, MouseButton, ResizeEdge};

    let handle = |id: &'static str, edge, cursor| {
        div()
            .id(id)
            .debug_selector(move || id.into())
            .absolute()
            .cursor(cursor)
            .on_mouse_down(MouseButton::Left, move |_, window, _| {
                window.start_window_resize(edge)
            })
    };

    div()
        .absolute()
        .inset_0()
        .child(
            handle("resize-top", ResizeEdge::Top, CursorStyle::ResizeUp)
                .top_0()
                .left(px(8.))
                .right(px(8.))
                .h(px(5.)),
        )
        .child(
            handle("resize-left", ResizeEdge::Left, CursorStyle::ResizeLeft)
                .left_0()
                .top(px(8.))
                .bottom(px(8.))
                .w(px(5.)),
        )
        .child(
            handle("resize-right", ResizeEdge::Right, CursorStyle::ResizeRight)
                .right_0()
                .top(px(8.))
                .bottom(px(8.))
                .w(px(5.)),
        )
        .child(
            handle("resize-bottom", ResizeEdge::Bottom, CursorStyle::ResizeDown)
                .bottom_0()
                .left(px(8.))
                .right(px(8.))
                .h(px(5.)),
        )
        .child(
            handle(
                "resize-top-left",
                ResizeEdge::TopLeft,
                CursorStyle::ResizeUpLeftDownRight,
            )
            .left_0()
            .top_0()
            .size(px(8.)),
        )
        .child(
            handle(
                "resize-top-right",
                ResizeEdge::TopRight,
                CursorStyle::ResizeUpRightDownLeft,
            )
            .right_0()
            .top_0()
            .size(px(8.)),
        )
        .child(
            handle(
                "resize-bottom-left",
                ResizeEdge::BottomLeft,
                CursorStyle::ResizeUpRightDownLeft,
            )
            .left_0()
            .bottom_0()
            .size(px(8.)),
        )
        .child(
            handle(
                "resize-bottom-right",
                ResizeEdge::BottomRight,
                CursorStyle::ResizeUpLeftDownRight,
            )
            .right_0()
            .bottom_0()
            .size(px(8.)),
        )
}

#[cfg(not(target_os = "linux"))]
pub fn resize_handles(_: &Window) -> Option<Div> {
    None
}

/// macOS keeps the window's appearance on the system setting, so a launcher set to Dark still
/// draws a light title bar on a light desktop. Pin the NSWindow's appearance to the choice
/// instead (`theme::pinned_dark`: `System` clears the pin, handing the chrome back to the
/// system). Windows uses custom chrome; Linux draws it only when the compositor requests it.
#[cfg(target_os = "macos")]
pub fn apply_appearance(window: &Window, choice: Appearance) {
    use objc2_app_kit::{
        NSAppearance, NSAppearanceCustomization, NSAppearanceName, NSAppearanceNameAqua,
        NSAppearanceNameDarkAqua, NSView,
    };
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    // AppKit's name constants are extern statics: reading them is `unsafe` to rustc only.
    let name: Option<&'static NSAppearanceName> = match crate::theme::pinned_dark(choice) {
        Some(true) => Some(unsafe { NSAppearanceNameDarkAqua }),
        Some(false) => Some(unsafe { NSAppearanceNameAqua }),
        None => None,
    };
    // The trait, not `Window::window_handle` (which is gpui's own `AnyWindowHandle`).
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return;
    };
    // The handle is the window's content view; its `window` is the NSWindow itself. Both outlive
    // this call.
    let view = unsafe { appkit.ns_view.cast::<NSView>().as_ref() };
    let Some(native) = view.window() else {
        return;
    };
    let appearance = name.and_then(NSAppearance::appearanceNamed);
    native.setAppearance(appearance.as_deref());
}

#[cfg(not(target_os = "macos"))]
pub fn apply_appearance(_: &Window, _: Appearance) {}

#[cfg(target_os = "windows")]
mod windows {
    use super::*;
    use crate::blocks;
    use crate::glass_ui::{color, hover_edge, white};
    use gpui::{div, prelude::*, px, Hsla, SharedString, Stateful, WindowControlArea};

    /// How far the bar sits in from the window's edges, where the native resize edges are.
    pub(super) const RESIZE_INSET: f32 = 5.;
    const BUTTON_WIDTH: f32 = 30.;
    const BUTTON_HEIGHT: f32 = 26.;
    const RADIUS: f32 = 8.;

    #[derive(Clone, Copy)]
    enum Control {
        Minimize,
        Maximize,
        Close,
    }

    impl Control {
        fn area(self) -> WindowControlArea {
            match self {
                Self::Minimize => WindowControlArea::Min,
                Self::Maximize => WindowControlArea::Max,
                Self::Close => WindowControlArea::Close,
            }
        }

        fn name(self) -> &'static str {
            match self {
                Self::Minimize => "window-minimize",
                Self::Maximize => "window-maximize",
                Self::Close => "window-close",
            }
        }
    }

    fn control_icon(control: Control, maximized: bool, rest: Hsla, hover: Hsla) -> Div {
        let name = control.name();
        let square = move || {
            div()
                .absolute()
                .size(px(9.))
                .rounded(px(1.5))
                .border_1()
                .border_color(rest)
                .group_hover(name, move |s| s.border_color(hover))
        };
        let icon = div().relative().size(px(14.));
        match control {
            Control::Minimize => icon.child(
                div()
                    .absolute()
                    .left(px(2.))
                    .top(px(7.))
                    .w(px(10.))
                    .h(px(1.))
                    .bg(rest)
                    .group_hover(name, move |s| s.bg(hover)),
            ),
            Control::Maximize if maximized => icon
                .child(
                    div()
                        .absolute()
                        .left(px(4.))
                        .top(px(1.))
                        .size(px(9.))
                        .rounded(px(1.5))
                        .border_t_1()
                        .border_r_1()
                        .border_color(rest)
                        .group_hover(name, move |s| s.border_color(hover)),
                )
                .child(square().left(px(1.)).top(px(4.))),
            Control::Maximize => icon.child(square().left(px(2.5)).top(px(2.5))),
            Control::Close => icon.child(
                blocks::icon("close", 14., rest).group_hover(name, move |s| s.text_color(hover)),
            ),
        }
    }

    fn control_button(control: Control, maximized: bool) -> Stateful<Div> {
        let id: SharedString = control.name().into();
        let close = matches!(control, Control::Close);
        let rest = white(0.65);
        let hover = if close { gpui::white() } else { white(1.) };
        // The eased hover layer: the close button goes red, the others take the glass fill and edge.
        let layer = if close {
            crate::hover_fade(
                &id,
                || {
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .rounded(crate::tokens::r(RADIUS))
                },
                |d, t| {
                    d.bg(color(0xe5484d).opacity(0.85 * t))
                        .border_1()
                        .border_color(color(0xe5484d).opacity(t))
                },
            )
        } else {
            hover_edge(&id, RADIUS, 0.10, false)
        };
        let el = div()
            .id(id.clone())
            .group(control.name())
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(px(BUTTON_WIDTH))
            .h(px(BUTTON_HEIGHT))
            .rounded(crate::tokens::r(RADIUS))
            // Pressed: darker. Native hit tests may not report the press.
            .active(|s| s.bg(gpui::black().opacity(0.25)))
            // GPUI maps these to native non-client hit tests, retaining Snap Layouts,
            // Alt+Space and close handling. Click callbacks would run the action twice.
            .window_control_area(control.area())
            .child(layer)
            .child(control_icon(control, maximized, rest, hover).relative());
        crate::track_hover(el, id)
    }

    /// The three caption buttons, at the bar's right end.
    pub(super) fn controls(window: &Window) -> Div {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .mr(px(6.))
            .children(
                [Control::Minimize, Control::Maximize, Control::Close]
                    .map(|control| control_button(control, window.is_maximized())),
            )
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests;
