//! The launcher's shared controls (one file each under `widgets/`), and the hover tracking they
//! share. See docs/redesign.md: a page builds from these and never draws its own button.
use super::*;

pub(crate) mod button;
pub(crate) mod check;
pub(crate) mod chip;
pub(crate) mod feedback;
pub(crate) mod field;
pub(crate) mod motion_bits;
pub(crate) mod nav;
use sunrise_launcher::backend::lock::recover;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Kind {
    Primary,
    Secondary,
    Ghost,
}

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

/// Whether `id` is the element [`hover_fade`] shows hovered now.
pub(crate) fn hover_is(id: &SharedString) -> bool {
    recover(&HOVER).lit.as_ref() == Some(id)
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

/// The last thing the player did was press Tab: focused controls show a ring (a click focuses them
/// too, without one).
pub(crate) static KEYBOARD: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Makes a control reachable with Tab, pressed with Enter or Space, with a ring when the keyboard
/// put it in focus.
pub(crate) fn keyboard(el: Stateful<Div>, t: &Theme) -> Stateful<Div> {
    let (ring, gap) = (t.accent, t.page);
    // A ring with a gap of page colour, so it shows around filled buttons too. GPUI draws nothing
    // for a shadow without blur: half a pixel keeps it crisp.
    let shadow = |color, spread: f32| gpui::BoxShadow {
        color,
        offset: gpui::point(px(0.), px(0.)),
        blur_radius: px(0.5),
        spread_radius: px(spread),
        inset: false,
    };
    el.tab_index(0).focus(move |s| {
        if KEYBOARD.load(Ordering::Relaxed) {
            s.shadow(vec![shadow(ring, 4.), shadow(gap, 1.5)])
        } else {
            s
        }
    })
}

/// A button: a block with a light bevel on top and a darker lip below, its label in the pixel
/// face. Primary (the one main action) is the violet face; Secondary a raised block; Ghost only
/// its label until the pointer is over it.
pub(crate) fn button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    kind: Kind,
    t: &Theme,
) -> Stateful<Div> {
    let id: SharedString = id.into();
    let label: SharedString = label.into();
    let label = SharedString::from(tr(&label).to_string());
    if t.glass {
        return glass_ui::button(id, label, kind, t);
    }
    let t = *t;
    let hover = match kind {
        Kind::Primary => t.on_face.opacity(0.10),
        Kind::Secondary => t.bevel.opacity(if t.glass { 0.10 } else { 0.35 }),
        Kind::Ghost => t.raised,
    };
    let base = div()
        .id(id.clone())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(theme::CONTROL))
        .px(px(14.))
        .font_family(theme::PIXEL)
        .text_size(px(15.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .child(hover_layer(&id, hover, 0., 0.));
    let base = match kind {
        Kind::Primary => base
            .border_2()
            .border_color(t.outline)
            .children(blocks::edges(t.face_bevel, t.face_lip, theme::LIP))
            .bg(t.face)
            .text_color(t.on_face),
        Kind::Secondary => base
            .border_2()
            .border_color(t.outline)
            .children(blocks::edges(t.button_bevel, t.lip, theme::LIP))
            .bg(t.button)
            .text_color(t.ink),
        Kind::Ghost => base.text_color(t.muted).hover(move |s| s.text_color(t.ink)),
    };
    // The label sits on the block's face, above its lip; pressing pushes it down.
    let base = base.child(
        div()
            .relative()
            .flex()
            .items_center()
            .gap(px(8.))
            .pb(px(if kind == Kind::Ghost { 0. } else { 2. }))
            .child(label),
    );
    press(keyboard(track_hover(base, id.clone()), &t), &id, |d| {
        d.opacity(theme::PRESSED).top(px(1.))
    })
}

/// Copies to the clipboard, or into `SUNRISE_LAUNCHER_CLIPBOARD` when a test run sets it
/// (`sunrise_launcher::desktop_seams`).
pub(crate) fn copy_to_clipboard(cx: &mut App, item: gpui::ClipboardItem) {
    match item.text().as_deref().map(copied) {
        Some(Copied::Text(shown)) => toast::say_with("Copied {text}", &shown),
        Some(Copied::Lines(n)) => {
            toast::say(trf("Copied {count} lines", &[("count", &n.to_string())]))
        }
        None => toast::say(tr("Copied the picture")),
    }
    copy_quietly(cx, item);
}

/// The longest copy the toast repeats in full; a longer line ends in an ellipsis.
const COPIED_SHOWN: usize = 64;

/// What the toast says about a copy.
#[derive(Debug, PartialEq)]
enum Copied {
    /// One line, as copied or cut short.
    Text(String),
    /// More than one: a build log would fill the window, so only their count.
    Lines(usize),
}

fn copied(text: &str) -> Copied {
    // Counted as copied: blank lines at the edges are lines too. Only the one-line preview is trimmed.
    let lines = text.lines().count();
    if lines > 1 {
        return Copied::Lines(lines);
    }
    let text = text.trim();
    if text.chars().nth(COPIED_SHOWN).is_none() {
        return Copied::Text(text.to_string());
    }
    let end = text
        .char_indices()
        .nth(COPIED_SHOWN - 1)
        .map_or(text.len(), |(i, _)| i);
    Copied::Text(format!("{}…", text[..end].trim_end()))
}

/// Copies like [`copy_to_clipboard`] without its toast, for a button that shows the copy itself.
pub(crate) fn copy_quietly(cx: &mut App, item: gpui::ClipboardItem) {
    if let Some(file) = sunrise_launcher::desktop_seams::clipboard_file() {
        let bytes = match item.entries().first() {
            Some(gpui::ClipboardEntry::Image(image)) => image.bytes.clone(),
            _ => item.text().unwrap_or_default().into_bytes(),
        };
        if let Err(e) = std::fs::write(&file, bytes) {
            eprintln!("{}: {e}", file.display());
        }
        return;
    }
    cx.write_to_clipboard(item);
}

/// An on/off switch: a square knob sliding in a sunk track that turns violet when on.
#[cfg_attr(not(windows), allow(dead_code))] // the Windows setup window
pub(crate) fn switch(id: impl Into<SharedString>, on: bool, t: &Theme) -> Stateful<Div> {
    if t.glass {
        let look = glass_ui::look();
        return glass_ui::toggle(id, on, look.accent, look.on_accent, t);
    }
    let t = *t;
    let id: SharedString = id.into();
    let knob = if on { t.on_face } else { t.muted };
    keyboard(div().id(id.clone()), &t)
        .relative()
        .flex_none()
        .w(px(44.))
        .h(px(24.))
        .border_2()
        .border_color(t.outline)
        .cursor_pointer()
        .bg(if on { t.face } else { t.well })
        .child(
            div()
                .absolute()
                .top(px(2.))
                .size(px(16.))
                .bg(knob)
                .child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .h(px(2.))
                        .bg(if on { t.face_lip } else { t.lip }),
                )
                .with_animation(
                    ElementId::named_usize(id, on as usize),
                    Animation::new(theme::MOVE).with_easing(ease_out_quint()),
                    move |d, p| d.left(px(if on { 2. + 20. * p } else { 22. - 20. * p })),
                ),
        )
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

/// A thin bar. A known fraction eases from its previous value; an unknown one shuttles slowly.
/// Progress: a plate-coloured track filling with the accent; work of unknown length shuttles.
pub(crate) fn progress_bar(fraction: Option<(f32, f32)>, t: &Theme) -> Div {
    // Night: a thin accent line on a plate's colour, square like the art. Glass: the mockup's
    // `Bar`, 4 px and round, in the accent's soft shade on white at 12 %.
    let n = t.night;
    let bar = div()
        .absolute()
        .top_0()
        .bottom_0()
        .bg(plate::alpha(n.accent, 1.))
        .when(t.glass, |d| {
            d.rounded_full()
                .bg(glass_ui::color(glass_ui::look().accent_soft))
        });
    // Full width and never resized by its parent's flex: in a column, a growing track's basis of
    // 0 left it no height, so the bar was never seen while its gap still took room.
    let track = div()
        .relative()
        .flex_none()
        .w_full()
        .h(px(6.))
        .bg(plate::alpha(n.card_hi, n.card_hi_alpha))
        .overflow_hidden()
        .when(t.glass, |d| {
            d.h(px(4.))
                .rounded_full()
                .bg(glass_ui::white(sunrise_launcher_theme::glass::fill::ACTIVE))
        });
    match fraction {
        Some((from, to)) => {
            let (from, to) = (from.clamp(0.02, 1.), to.clamp(0.02, 1.));
            track.child(bar.left_0().with_animation(
                ElementId::named_usize("bar", (to * 1000.) as usize),
                Animation::new(theme::MOVE).with_easing(ease_out_quint()),
                move |d, p| d.w(relative(from + (to - from) * p)),
            ))
        }
        // Reduced motion holds the shuttle still where it can be seen (GPUI freezes a repeating
        // animation at its start, off the track).
        None if motion::still() => track.child(bar.left(relative(0.35)).w(relative(0.3))),
        None => track.child(
            bar.w(relative(0.3)).with_animation(
                "shuttle",
                motion::endless(
                    Animation::new(Duration::from_millis(1600))
                        .repeat()
                        .with_easing(gpui::ease_in_out),
                ),
                |d, p| d.left(relative(-0.3 + 1.3 * p)),
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_shows_in_the_toast_only_while_it_fits_on_a_line() {
        assert_eq!(copied("AB12-CD\n"), Copied::Text("AB12-CD".into()));
        let long = "x".repeat(200);
        assert_eq!(
            copied(&long),
            Copied::Text(format!("{}…", "x".repeat(COPIED_SHOWN - 1)))
        );
        assert_eq!(
            copied("é".repeat(COPIED_SHOWN).as_str()),
            Copied::Text("é".repeat(COPIED_SHOWN))
        );
        assert_eq!(copied("error: one\n  two\n\nthree\n"), Copied::Lines(4));
        assert_eq!(copied("\nerror\n\n"), Copied::Lines(3));
    }

    /// A panic while the hover state is held poisons its lock; the next hover must still answer.
    #[test]
    fn a_poisoned_hover_lock_still_answers() {
        let _ = std::thread::spawn(|| {
            let _held = HOVER.lock().unwrap();
            panic!("poison the hover lock on purpose");
        })
        .join();
        assert!(!hover_is(&SharedString::from("poison-test")));
    }
}
