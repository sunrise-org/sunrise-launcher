//! The launcher's shared controls (one file each under `widgets/`), and the hover tracking they
//! share. See docs/redesign.md: a page builds from these and never draws its own button.
use super::*;

pub(crate) mod button;
pub(crate) mod check;
pub(crate) mod chip;
pub(crate) mod choice;
pub(crate) mod copies;
pub(crate) mod feedback;
pub(crate) mod field;
pub(crate) mod focus;
pub(crate) mod motion_bits;
pub(crate) mod nav;
pub(crate) mod scrolls;
use sunrise_launcher::backend::lock::recover;

/// The element under the pointer, and every element whose hover is easing in or out, each on
/// its own [`motion::Fade`]: a pointer swept across a menu fades each row it leaves from where
/// it had got to, several at once.
pub(crate) struct HoverState {
    lit: Option<SharedString>,
    fades: Vec<(SharedString, motion::Fade)>,
}

pub(crate) static HOVER: Mutex<HoverState> = Mutex::new(HoverState {
    lit: None,
    fades: Vec::new(),
});

impl HoverState {
    fn fade(&self, id: &SharedString) -> Option<motion::Fade> {
        self.fades
            .iter()
            .find(|(f, _)| f == id)
            .map(|(_, fade)| *fade)
    }

    /// `id`'s hover turns `on` or off now, from where its fade was.
    fn turn(&mut self, id: &SharedString, on: bool, now: Instant) {
        let fade = motion::Fade::toward(self.fade(id), on, now);
        self.fades.retain(|(f, fade)| f != id && !fade.rested(now));
        self.fades.push((id.clone(), fade));
    }
}

/// The view an element is drawn in, noted while it is painted: its hover and press redraw that
/// view alone, not the window (which would build every view again, none reused).
#[derive(Clone, Default)]
pub(crate) struct Owner(std::rc::Rc<std::cell::Cell<Option<gpui::EntityId>>>);

impl Owner {
    /// A child of the element, taking no room, that notes the view it is painted in.
    pub(crate) fn noting(&self) -> impl IntoElement {
        let owner = self.0.clone();
        gpui::canvas(
            move |_, window, _| owner.set(Some(window.current_view())),
            |_, _, _, _| {},
        )
        .absolute()
        .size_0()
    }

    /// Redraws the view the element was painted in (the window, before its first paint).
    pub(crate) fn redraw(&self, window: &mut Window, cx: &mut App) {
        match self.0.get() {
            Some(view) => cx.notify(view),
            None => window.refresh(),
        }
    }
}

/// `el` (named `id`) eases its hover layers ([`hover_fade`], [`hover_move`]) in and out under
/// the pointer.
pub(crate) fn track_hover(el: Stateful<Div>, id: SharedString) -> Stateful<Div> {
    let owner = Owner::default();
    el.child(owner.noting())
        .on_hover(move |hovered, window, cx| note_hover(&id, *hovered, &owner, window, cx))
}

/// Records that `id` is now hovered or not, for [`hover_fade`], and redraws the view it is in:
/// an element whose own hover listener does more calls it, with the [`Owner`] it notes.
pub(crate) fn note_hover(
    id: &SharedString,
    hovered: bool,
    owner: &Owner,
    window: &mut Window,
    cx: &mut App,
) {
    let mut state = recover(&HOVER);
    let now = Instant::now();
    let is_lit = state.lit.as_ref() == Some(id);
    if hovered && !is_lit {
        // One element is lit at a time: the one the pointer last entered.
        if let Some(left) = state.lit.replace(id.clone()) {
            state.turn(&left, false, now);
        }
        state.turn(id, true, now);
    } else if !hovered && is_lit {
        state.lit = None;
        state.turn(id, false, now);
    } else {
        return;
    }
    drop(state);
    owner.redraw(window, cx);
}

/// The elements the left button went down on, until it comes up anywhere.
static PRESSED: Mutex<Vec<SharedString>> = Mutex::new(Vec::new());

/// `el` (named `id`) drawn by `pressed` while the left button is held on it, as `.active()`
/// draws it, but redrawing only the view it is in: `.active()` redraws the window on the press
/// and the release, every view built again. `pressed` applies last, over the element's looks.
/// (An element whose `.hover()` sets what the press does keeps `.active()`: a hover style
/// draws over the element's own.)
pub(crate) fn press(
    el: Stateful<Div>,
    id: &SharedString,
    pressed: impl FnOnce(Stateful<Div>) -> Stateful<Div>,
) -> Stateful<Div> {
    let owner = Owner::default();
    let held = recover(&PRESSED).contains(id);
    let (down, up, out) = (id.clone(), owner.clone(), owner.clone());
    let el = el
        .child(owner.noting())
        .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
            recover(&PRESSED).push(down.clone());
            owner.redraw(window, cx);
        })
        .on_mouse_up(gpui::MouseButton::Left, move |_, window, cx| {
            release(&up, window, cx)
        })
        .on_mouse_up_out(gpui::MouseButton::Left, move |_, window, cx| {
            release(&out, window, cx)
        });
    if held {
        pressed(el)
    } else {
        el
    }
}

fn release(owner: &Owner, window: &mut Window, cx: &mut App) {
    let mut pressed = recover(&PRESSED);
    if !pressed.is_empty() {
        pressed.clear();
        drop(pressed);
        owner.redraw(window, cx);
    }
}

/// `id`'s fade as `motion` plays it, while there is something to draw.
fn hover_now(id: &SharedString, motion: sunrise_launcher_theme::Motion) -> Option<motion::Fade> {
    let mut state = recover(&HOVER);
    let (_, fade) = state.fades.iter_mut().find(|(f, _)| f == id)?;
    fade.motion = motion;
    let fade = *fade;
    (!fade.rested(Instant::now())).then_some(fade)
}

/// A layer under an element's content that fades `color` in while it is hovered. `bleed` lets
/// it reach past the element on the left and right, as the hover plate of a list row does.
pub(crate) fn hover_layer(id: &SharedString, color: Hsla, radius: f32, bleed: f32) -> AnyElement {
    let layer = || {
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(-bleed))
            .right(px(-bleed))
            .rounded(px(glass_ui::rad(radius)))
    };
    hover_fade(id, layer, move |d, a| d.bg(color.opacity(color.a * a)))
}

/// The `layer` made while the element `id` (see [`track_hover`]) is hovered, eased in and out:
/// `paint` draws it at a strength from 0 (not shown) to 1. An element not hovered draws nothing,
/// so a long list costs no layers.
pub(crate) fn hover_fade(
    id: &SharedString,
    layer: impl FnOnce() -> Div,
    paint: impl Fn(Div, f32) -> Div + 'static,
) -> AnyElement {
    hover_ease(id, layer, paint, sunrise_launcher_theme::QUICK)
}

/// [`hover_fade`] on any motion: the spring for the site's moving parts, QUICK for colour.
pub(crate) fn hover_ease(
    id: &SharedString,
    layer: impl FnOnce() -> Div,
    paint: impl Fn(Div, f32) -> Div + 'static,
    on: sunrise_launcher_theme::Motion,
) -> AnyElement {
    match hover_now(id, on) {
        Some(fade) => fade.play(id, layer(), paint),
        // Not hovered: nothing to draw (out of the flow, so it takes no gap either).
        None => div().absolute().into_any_element(),
    }
}

/// Sunrise's mark (`docs/media/brand/icon.svg`), cell for cell on its 16 grid: the rays, the sun
/// and the horizon on the dark tile.
pub(crate) fn mark(size: f32) -> Div {
    const CELLS: [(f32, f32, f32, f32, u32); 8] = [
        (7., 3., 2., 2., 0xffe39a),
        (2., 5., 2., 2., 0xffe39a),
        (12., 5., 2., 2., 0xffe39a),
        (6., 7., 4., 1., 0xffcf5c),
        (4., 8., 8., 1., 0xffcf5c),
        (3., 9., 10., 1., 0xffcf5c),
        (3., 10., 10., 1., 0xffb48a),
        (1., 12., 14., 2., 0x6f58d9),
    ];
    let u = size / 16.;
    let mut tile = div()
        .relative()
        .flex_none()
        .size(px(size))
        .rounded(px(3. * u))
        .bg(gpui::rgb(0x100a1f));
    for (x, y, w, h, color) in CELLS {
        tile = tile.child(
            div()
                .absolute()
                .left(px(x * u))
                .top(px(y * u))
                .w(px(w * u))
                .h(px(h * u))
                .bg(gpui::rgb(color)),
        );
    }
    tile
}

/// What the Play button does for the selected version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlayPress {
    Play,
    /// Not prepared, for an account that owns the game: prepare it (from the player's install,
    /// else downloaded from Mojang for that account), then play, as the official launcher does.
    PrepareThenPlay,
    /// Not prepared and no owner (signing in comes first), or busy preparing or playing.
    Nothing,
}

/// Polling must handle each sign-in transition once. Replaying a completed sign-in
/// restarts every account's status worker and leaves the UI perpetually checking.
pub(crate) fn sign_in_change(
    shown: &mut Option<SignInState>,
    next: SignInState,
) -> Option<SignInState> {
    if shown.as_ref() == Some(&next) {
        return None;
    }
    *shown = Some(next.clone());
    Some(next)
}

/// `owner`: whether an account here owns the game, by the keychain (`Launcher::owner`).
pub(crate) fn play_press(prepared: bool, owner: Option<bool>, busy: bool) -> PlayPress {
    match (prepared, owner) {
        _ if busy => PlayPress::Nothing,
        (true, _) => PlayPress::Play,
        (false, Some(true)) => PlayPress::PrepareThenPlay,
        (false, _) => PlayPress::Nothing,
    }
}

/// Whether a shortcut that starts the game at once (a server's Join, a world's Play, a friend's
/// code, a pack's Play) starts it now: only when Play would play. Never while a preparation runs:
/// the game would open on half-written files.
pub(crate) fn starts_now(prepared: bool, preparing: bool, running: bool) -> bool {
    play_press(prepared, None, preparing || running) == PlayPress::Play
}

pub(crate) fn disabled(b: Stateful<Div>) -> Stateful<Div> {
    b.opacity(theme::DISABLED).cursor_default().tab_stop(false)
}

pub(crate) fn short(path: &std::path::Path) -> String {
    let home = dirs::home_dir().unwrap_or_default();
    match path.strip_prefix(&home) {
        Ok(rest) if !home.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A panic while the hover state is held poisons its lock; the next hover must still answer.
    #[test]
    fn a_poisoned_hover_lock_still_answers() {
        let _ = std::thread::spawn(|| {
            let _held = HOVER.lock().unwrap();
            panic!("poison the hover lock on purpose");
        })
        .join();
        assert!(recover(&HOVER).lit.is_none());
    }
}
