//! The sidebar's profiles: the heading with its + button, the list (each profile's mark and
//! name, its actions button, its context menu), and the switch between them.
use super::*;
use crate::widgets::button::{icon_button, Style};
use crate::widgets::focus::Reach;
use gpui::{rgb, AnimationExt, AnyElement, App, MouseButton, MouseDownEvent, Window};
use instances::{Icon, Instance, Kind};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeSet, HashMap},
    rc::Rc,
    time::Instant,
};

/// What the profiles are drawn with: how open the sidebar is, why profiles are locked (if they
/// are), and whether the page is Home, where the selected profile is shown as selected.
#[derive(Clone, Copy)]
pub(super) struct Look {
    pub open: f32,
    pub lock: Option<&'static str>,
    pub on_home: bool,
}

/// A profile row's height open (its name over its channel), and closed, where it is a square
/// highlight as wide as the folded sidebar's rows, around the mark.
const PROFILE_ROW: f32 = 48.;
const PROFILE_ROW_CLOSED: f32 = 30.;

/// The row's height `open` of the way: it grows from the closed highlight to the open row with
/// the sidebar, so the fill never jumps.
fn row_height(open: f32) -> f32 {
    PROFILE_ROW_CLOSED + (PROFILE_ROW - PROFILE_ROW_CLOSED) * open.clamp(0., 1.)
}

/// A profile's mark when the sidebar is open, and folded. Folded, the mark sits in the nav
/// square 30 px highlight with 4 px of it showing at each side.
const MARK_OPEN: f32 = 28.;
const MARK_CLOSED: f32 = 22.;

/// The mark's size `open` of the way, easing with the sidebar so it never jumps.
fn mark_size(open: f32) -> f32 {
    MARK_CLOSED + (MARK_OPEN - MARK_CLOSED) * open.clamp(0., 1.)
}

/// The height of the dots that grow under a folded profile's mark when the pointer is on it.
const DOTS_HEIGHT: f32 = 22.;

/// Whether a profile's fill and dots are the entry's: the sidebar is folded and profiles can be
/// picked.
fn folds_open(look: Look) -> bool {
    look.open <= 0. && look.lock.is_none()
}

/// The actions button of a folded profile, which grows under its mark while the pointer is on the
/// entry `hover_id` (see `track_hover`) and fades in with it, then eases away again. It is built
/// only while the hover eases, so it is no Tab stop at rest. It opens the same menu as the open
/// sidebar's button.
/// A click handler that several closures share.
type OnClick = dyn Fn(&gpui::ClickEvent, &mut Window, &mut App);

fn folded_dots(
    i: &Instance,
    hover_id: &SharedString,
    busy: bool,
    cx: &mut Context<Launcher>,
) -> AnyElement {
    let p = palette();
    let button = Rc::new(Cell::new(gpui::Bounds::<gpui::Pixels>::default()));
    let menu_of = i.clone();
    let read_button = button.clone();
    let pick: Rc<OnClick> = Rc::new(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
        this.open_profile_menu_under(&menu_of, read_button.get());
        cx.notify();
    }));
    let key = SharedString::from(format!("profile-dots-{}", i.id));
    let (ink, hover) = (rgb(p.chrome_text), rgb(p.hover));
    crate::widgets::hover_fade(
        hover_id,
        || {
            div()
                .w_full()
                .flex_none()
                .overflow_hidden()
                .flex()
                .justify_center()
        },
        move |row, v| {
            let v = v.clamp(0., 1.);
            let write_button = button.clone();
            let pick = pick.clone();
            row.h(px(DOTS_HEIGHT * v)).child(
                div()
                    .id(key.clone())
                    .w(px(28.))
                    .h(px(DOTS_HEIGHT))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .opacity(v)
                    .cursor_pointer()
                    .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                    .hover(move |d| d.bg(hover))
                    .tooltip(crate::glass_ui::tip_unless(busy, tr("Profile actions")))
                    .child(
                        div()
                            .flex()
                            .gap(px(2.))
                            .children((0..3).map(|_| div().size(px(3.)).rounded_full().bg(ink))),
                    )
                    .child(
                        gpui::canvas(
                            move |bounds, _, _| write_button.set(bounds),
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    )
                    // A press on the dots is the dots' own: it never reaches the profile's row,
                    // which would take it as a press to pick the profile.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |e, w, cx| pick(e, w, cx)),
            )
        },
    )
}

/// A profile's name, fading in as the sidebar opens.
fn label_text(open: f32, name: String, under: Option<&'static str>) -> Div {
    let line = |text: String| {
        div()
            .overflow_hidden()
            .whitespace_nowrap()
            // An ellipsis would flicker along a label still being uncovered.
            .when(open >= 1., |d| d.text_ellipsis())
            .child(text)
    };
    div()
        .min_w_0()
        .flex_1()
        .flex()
        .flex_col()
        .opacity(label_opacity(open))
        .child(line(name).text_size(px(tokens::LABEL)))
        // The profile's channel under its name (Figma: "Nightly · 3 mods"; the count is known
        // only for the open profile, so the line names the channel).
        .children(under.map(|under| {
            line(under.to_owned())
                .text_size(px(tokens::CAPTION))
                .text_color(rgb(palette().muted))
        }))
}

/// A profile's row: its selected fill (no accent edge), its hover while it can be picked, and
/// its tooltip when its name is hidden or it is locked (then with the reason).
fn profile_button(id: ElementId, name: String, on: bool, look: Look) -> Stateful<Div> {
    let p = palette();
    let labelled = look.open > 0.;
    // Folded, the entry draws the fill, so it covers the dots that grow under the mark too.
    let entry_fills = folds_open(look);
    div()
        .id(id)
        .reach_edge()
        .h(px(row_height(look.open)))
        .flex_none()
        .w_full()
        // Open, the selected fill reaches a little past the mark on its left.
        .pl(px(6. * look.open))
        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
        .flex()
        .items_center()
        .gap(px(4.))
        .pr(px(8. * look.open))
        .when(on && !entry_fills, |d| d.bg(rgb(p.selected)))
        .when(look.lock.is_none(), |d| {
            d.cursor_pointer().when(!on && !entry_fills, |d| {
                d.hover(|d| d.bg(rgb(p.chrome_raised)))
            })
        })
        .when(look.lock.is_some(), |d| d.opacity(0.45).tab_stop(false))
        .when(!labelled || look.lock.is_some(), |d| {
            // Above and right, so the dots that grow under a folded profile stay in view.
            d.tooltip(crate::glass_ui::tip_above(
                look.lock.map_or(name.clone(), str::to_owned),
            ))
        })
}

/// How far the open sidebar's dots slide in from the right, in px.
const DOTS_SLIDE: f32 = 8.;

/// The three dots of an actions button, in `ink`.
fn dots_mark(ink: gpui::Rgba) -> Div {
    div()
        .flex()
        .gap(px(2.))
        .children((0..3).map(move |_| div().size(px(3.)).rounded_full().bg(ink)))
}

/// The actions button of a profile, beside its name once the sidebar is open enough to show it,
/// fading out as it folds. Folded, the row is the mark alone and the menu is on a right-click.
/// It opens the profile's menu under it.
///
/// Two parts: the dots that slide in from the right and fade in while the pointer is on the entry
/// `hover_id` (see `track_hover`), and the button itself over them, which is always there so the
/// keyboard can reach it. It is clear until it has the focus, when it shows. Its press stops at
/// it, so it never picks the profile under it.
fn profile_actions(
    i: &Instance,
    open: f32,
    hover_id: &SharedString,
    busy: bool,
    cx: &mut Context<Launcher>,
) -> Vec<AnyElement> {
    if !actions_shown(open) {
        return Vec::new();
    }
    let p = palette();
    let shown = label_opacity(open);
    let top = (row_height(open) - 28.) / 2.;
    let (ink, hover) = (rgb(p.chrome_text), rgb(p.hover));
    let plate = SharedString::from(format!("profile-actions-plate-{}", i.id));
    let dots = crate::widgets::hover_fade(
        hover_id,
        move || {
            div()
                .absolute()
                .top(px(top))
                .w(px(28.))
                .h(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
        },
        move |d, v| {
            let v = v.clamp(0., 1.);
            d.right(px(6. - DOTS_SLIDE * (1. - v)))
                .opacity(v * shown)
                .child(
                    div()
                        .id(plate.clone())
                        .absolute()
                        .inset_0()
                        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                        .hover(move |d| d.bg(hover)),
                )
                .child(dots_mark(ink))
        },
    );
    let button = Rc::new(Cell::new(gpui::Bounds::<gpui::Pixels>::default()));
    let read_button = button.clone();
    let i = i.clone();
    let press = div()
        .id(SharedString::from(format!("profile-actions-{}", i.id)))
        .debug_selector(|| "selected-profile-actions".into())
        .reach()
        .absolute()
        .top(px(top))
        .right(px(6.))
        .w(px(28.))
        .h(px(28.))
        .opacity(0.)
        // `reach` set the focus style (the mark `seen` reads); this one keeps it and shows the
        // button when the keyboard is on it.
        .focus(|s| s.text_bg(crate::widgets::focus::focus_mark()).opacity(1.))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
        .tooltip(crate::glass_ui::tip_unless(busy, tr("Profile actions")))
        .child(dots_mark(ink))
        .child(
            gpui::canvas(move |bounds, _, _| button.set(bounds), |_, _, _, _| {})
                .absolute()
                .inset_0(),
        )
        // The press is the button's alone: the row under it picks its profile on a click it
        // sees go down.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        // The profile's menu (settings, duplicate, folder, order, Delete) under the button;
        // right-clicking the row opens the same one.
        .on_click(cx.listener(move |this, _, _, cx| {
            this.open_profile_menu_under(&i, read_button.get());
            cx.notify();
        }));
    vec![dots, press.into_any_element()]
}

/// A profile's right-click target: where it was last drawn (for the menu that opens over it),
/// and the menu it opens. Only the part of the row the list shows counts.
fn menu_target(
    button: Stateful<Div>,
    i: &Instance,
    rows: ProfileRows,
    cx: &mut Context<Launcher>,
) -> Stateful<Div> {
    let (drawn, menu) = (i.clone(), i.clone());
    button
        .child(
            gpui::canvas(
                move |bounds, window, _| {
                    let visible = bounds.intersect(&window.content_mask().bounds);
                    if !visible.is_empty() {
                        rows.borrow_mut().push((visible, drawn.clone()));
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                this.open_profile_menu(&menu, f32::from(ev.position.y));
                cx.notify();
            }),
        )
}

/// The gap between the list's rows.
const ROW_GAP: f32 = 4.;

/// A group's heading extent in the list, with the gap after it: its 22 px and 6 px above, or the
/// 1 px rule and 4 px above of the folded sidebar.
const HEADING_EXTENT: f32 = 22. + 6. + ROW_GAP;
const RULE_EXTENT: f32 = 1. + 4. + ROW_GAP;

/// A group's folds: which are folded (read from `instances.json` once, and again after a change
/// to the groups), what each was when last drawn, and the ease of each that is turning over, so
/// the rows under a heading slide shut or open once and then rest.
#[derive(Default)]
struct Folds {
    folded: Option<BTreeSet<String>>,
    drawn: HashMap<String, bool>,
    /// Per group still turning over: how open it is going to be (1 open, 0 folded).
    fades: HashMap<String, crate::motion::Fade>,
}

impl Folds {
    fn set(&mut self) -> &mut BTreeSet<String> {
        self.folded
            .get_or_insert_with(|| instances::group_choices().collapsed)
    }

    /// Notes that the group is drawn folded or not. Returns how open it is now (1 open, 0 folded)
    /// and, while it turns over, the ease that plays it. A turn made while another is still
    /// playing reverses from where that one is; under reduced motion and in benchmarks it snaps.
    fn note(&mut self, group: &str, folded: bool) -> (f32, Option<crate::motion::Fade>) {
        let now = Instant::now();
        if self.drawn.insert(group.to_owned(), folded) == Some(!folded) {
            if crate::motion::arrivals() {
                let mut fade =
                    crate::motion::Fade::toward(self.fades.get(group).copied(), !folded, now);
                fade.motion = sunrise_launcher_theme::MOVE;
                self.fades.insert(group.to_owned(), fade);
            } else {
                self.fades.remove(group);
            }
        }
        match self.fades.get(group).copied() {
            Some(fade) if crate::motion::arrivals() && fade.moving(now) => {
                (fade.value(now).clamp(0., 1.), Some(fade))
            }
            _ => {
                self.fades.remove(group);
                (if folded { 0. } else { 1. }, None)
            }
        }
    }
}

thread_local! {
    static FOLDS: RefCell<Folds> = RefCell::new(Folds::default());
}

/// Whether the group `group` is folded in the sidebar.
pub(crate) fn group_collapsed(group: &str) -> bool {
    FOLDS.with_borrow_mut(|folds| folds.set().contains(group))
}

/// Reads which groups are folded again, after a group was renamed or removed.
pub(crate) fn reload_group_folds() {
    FOLDS.with_borrow_mut(|folds| folds.folded = None);
}

/// One small button on a group's heading: `mark` slides in from the right and fades in while the
/// pointer is on the heading `hover_id` (see `track_hover`), as a profile's dots do, and the
/// button itself is over it, clear until the keyboard has it. `right` is how far its right edge
/// is from the heading's, `tip` its tooltip (kept away while `busy`: what it opened is up), and
/// `press` runs on a click, which never reaches the heading (that folds the group).
#[allow(clippy::too_many_arguments)]
fn group_button(
    key: SharedString,
    hover_id: &SharedString,
    right: f32,
    busy: bool,
    tip: SharedString,
    mark: impl Fn() -> AnyElement + Clone + 'static,
    press: impl Fn(gpui::Bounds<gpui::Pixels>, &mut Launcher, &mut Context<Launcher>) + 'static,
    cx: &mut Context<Launcher>,
) -> Vec<AnyElement> {
    const WIDE: f32 = 24.;
    const HIGH: f32 = 22.;
    let p = palette();
    let hover = rgb(p.hover);
    let plate = SharedString::from(format!("{key}-plate"));
    let drawn = mark.clone();
    let fade = crate::widgets::hover_fade(
        hover_id,
        || {
            div()
                .absolute()
                .top_0()
                .w(px(WIDE))
                .h(px(HIGH))
                .flex()
                .items_center()
                .justify_center()
                .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
        },
        move |d, v| {
            let v = v.clamp(0., 1.);
            d.right(px(right - DOTS_SLIDE * (1. - v)))
                .opacity(v)
                .child(
                    div()
                        .id(plate.clone())
                        .absolute()
                        .inset_0()
                        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                        .hover(move |d| d.bg(hover)),
                )
                .child(drawn())
        },
    );
    // Each button's fade sits in its own keyed box, so two on one heading play under their own
    // animation ids.
    let fade = div()
        .id(SharedString::from(format!("{key}-fade")))
        .absolute()
        .inset_0()
        .child(fade)
        .into_any_element();
    let button = Rc::new(Cell::new(gpui::Bounds::<gpui::Pixels>::default()));
    let read_button = button.clone();
    let pressed = div()
        .id(key)
        .reach()
        .absolute()
        .top_0()
        .right(px(right))
        .w(px(WIDE))
        .h(px(HIGH))
        .opacity(0.)
        .focus(|s| s.text_bg(crate::widgets::focus::focus_mark()).opacity(1.))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
        .tooltip(crate::glass_ui::tip_unless(busy, tip))
        .child(mark())
        .child(
            gpui::canvas(move |bounds, _, _| button.set(bounds), |_, _, _, _| {})
                .absolute()
                .inset_0(),
        )
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, _, _, cx| {
            press(read_button.get(), this, cx);
            cx.notify();
        }));
    vec![fade, pressed.into_any_element()]
}

/// The buttons of a group's heading, which slide in from the right: a plus (a new profile in the
/// group, which the "…" menu no longer lists) and the dots, which open the group's menu under
/// them.
fn group_actions(
    group: &str,
    hover_id: &SharedString,
    busy: bool,
    locked: Option<&'static str>,
    cx: &mut Context<Launcher>,
) -> Vec<AnyElement> {
    let ink = rgb(crate::paint::palette().chrome_text);
    let named = group.to_owned();
    let mut buttons = group_button(
        SharedString::from(format!("group-actions-{group}")),
        hover_id,
        4.,
        busy,
        tr("Group actions").into(),
        move || dots_mark(ink).into_any_element(),
        move |button, this, _| this.open_group_menu_under(&named, button),
        cx,
    );
    let added = group.to_owned();
    let plus = group_button(
        SharedString::from(format!("group-add-{group}")),
        hover_id,
        4. + 24. + 2.,
        busy,
        locked
            .unwrap_or_else(|| tr("New profile in this group"))
            .into(),
        move || crate::glass_ui::icon("plus-sign", 14., ink.into()).into_any_element(),
        move |_, this, cx| {
            if this.profile_lock().is_none() {
                this.close_profile_action();
                this.new_profile_in_group(&added, cx);
            }
        },
        cx,
    );
    buttons.extend(plus);
    buttons
}

/// Where each profile was last listed, and the glide the last move gave the rows it swapped.
#[derive(Default)]
struct Glides {
    /// The ids in the order the list was last drawn.
    order: Vec<String>,
    /// Counts the moves: a row's glide plays once under its own animation id.
    moves: usize,
    /// The groups in the order the list was last drawn.
    groups: Vec<String>,
    /// Per swapped row (and group heading, by `group:<name>`), the move that last glided it and
    /// how far above (negative) or below (positive) its new place it started, in px. Rows never
    /// moved are absent.
    started: HashMap<String, (usize, f32)>,
}

thread_local! {
    static GLIDES: RefCell<Glides> = RefCell::new(Glides::default());
    /// The profile the list last scrolled to show: it scrolls again only when another is selected.
    static SHOWN: RefCell<Option<String>> = const { RefCell::new(None) };
}

impl Glides {
    /// Notes the list drawn now as `ids`. When it is the last one with two neighbours swapped
    /// (a profile moved up or down), those two rows start a glide from where they were, `step` px
    /// per place. Any other change (a profile made, renamed, deleted) snaps, as before.
    fn note(&mut self, ids: &[String], step: f32) {
        let changed: Vec<usize> = (0..ids.len().min(self.order.len()))
            .filter(|n| ids[*n] != self.order[*n])
            .collect();
        let swapped = ids.len() == self.order.len()
            && changed.len() == 2
            && changed[1] == changed[0] + 1
            && ids[changed[0]] == self.order[changed[1]]
            && ids[changed[1]] == self.order[changed[0]];
        if swapped {
            self.moves += 1;
            // The one now above was below, so it starts `step` lower; the other the reverse.
            self.started
                .insert(ids[changed[0]].clone(), (self.moves, step));
            self.started
                .insert(ids[changed[1]].clone(), (self.moves, -step));
        }
        if self.order != ids {
            self.order = ids.to_vec();
        }
    }

    /// Notes the groups drawn now as `order`, each with the rows it shows and their ids in
    /// `shown`. When it is the last order with two neighbouring groups swapped (a group moved up
    /// or down), both groups, their headings and rows start a glide from where they were: the
    /// other group's rows (`step` px each) and heading (`head`) away. True for such a move.
    fn note_groups(
        &mut self,
        order: &[String],
        shown: &HashMap<String, (usize, Vec<String>)>,
        step: f32,
        head: f32,
    ) -> bool {
        let changed: Vec<usize> = (0..order.len().min(self.groups.len()))
            .filter(|n| order[*n] != self.groups[*n])
            .collect();
        let swapped = order.len() == self.groups.len()
            && changed.len() == 2
            && changed[1] == changed[0] + 1
            && order[changed[0]] == self.groups[changed[1]]
            && order[changed[1]] == self.groups[changed[0]];
        if swapped {
            self.moves += 1;
            let (upper, lower) = (&order[changed[0]], &order[changed[1]]);
            let extent =
                |group: &String| shown.get(group).map_or(0, |(rows, _)| *rows) as f32 * step + head;
            // The one now above was below, so it starts lower by the other's extent.
            for (group, from) in [(upper, extent(lower)), (lower, -extent(upper))] {
                self.started
                    .insert(format!("group:{group}"), (self.moves, from));
                for id in shown.get(group).into_iter().flat_map(|(_, ids)| ids) {
                    self.started.insert(id.clone(), (self.moves, from));
                }
            }
        }
        if self.groups != order {
            self.groups = order.to_vec();
        }
        swapped
    }
}

/// `entry` held `from` px off its place at the start of a glide, easing to it; at once under
/// reduced motion. Layout moves with it, so the row stays clickable and its hover goes along.
fn glide(entry: Stateful<Div>, id: &str, (moved, from): (usize, f32)) -> AnyElement {
    let row = div().flex_none().relative().child(entry);
    // A row that never moved is simply there: an animation on every row would keep the window
    // drawing frames (and a benchmark counting them) for nothing.
    if moved == 0 || from == 0. || !crate::motion::arrivals() {
        return row.into_any_element();
    }
    row.with_animation(
        ElementId::named_usize(SharedString::from(format!("profile-glide-{id}")), moved),
        crate::motion::eased(sunrise_launcher_theme::MOVE),
        move |d, progress| d.top(px(from * (1. - progress))),
    )
    .into_any_element()
}

/// A group's rows while the list is built, until its last one is known.
struct Run {
    group: String,
    /// Where its heading is among the list's children.
    heading_at: usize,
    rows: Vec<AnyElement>,
    /// Which of `rows` is the selected profile's.
    selected: Option<usize>,
    /// Folded and at rest, so its rows aren't built.
    hidden: bool,
    /// Its fold is turning over: the rows sit in a box whose height follows this ease.
    fade: Option<crate::motion::Fade>,
    /// The height of all its rows, in px.
    height: f32,
}

/// Puts a finished group's rows in the list: straight in, in one box whose height eases between
/// shut and full while its fold turns over, or not at all when it is folded. The box holds the
/// space of the list's gap above it with it as it shuts, so folded it takes none, and what comes
/// after follows its height with nothing overlapping.
fn finish_run(run: Option<Run>, items: &mut Vec<AnyElement>, selected_at: &mut Option<usize>) {
    let Some(run) = run else {
        return;
    };
    let at = items.len();
    if let Some(fade) = run.fade {
        if run.selected.is_some() {
            *selected_at = Some(at);
        }
        let height = run.height;
        let rows = div()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(ROW_GAP))
            .overflow_hidden()
            .children(run.rows);
        items.push(fade.play(
            &SharedString::from(format!("group-fold-{}", run.group)),
            rows,
            move |d, open| {
                let open = open.clamp(0., 1.);
                d.h(px(height * open)).mt(px(-ROW_GAP * (1. - open)))
            },
        ));
        return;
    }
    if let Some(row) = run.selected {
        *selected_at = Some(at + row);
    }
    items.extend(run.rows);
}

impl Launcher {
    pub(crate) fn profile_lock(&self) -> Option<&'static str> {
        profile_lock_reason(
            &self.launch_activity(),
            !self.location_unavailable,
            self.ui.after_sign_in.is_some() || self.ui.after_install.is_some(),
        )
    }

    /// Folds the group `group` under its heading, or unfolds it, and keeps the choice for the
    /// next time the launcher opens.
    pub(crate) fn toggle_group(&mut self, group: &str, cx: &mut Context<Self>) {
        let fold = !group_collapsed(group);
        if let Err(error) = instances::collapse_group(group, fold) {
            crate::toast::say(error);
            return;
        }
        FOLDS.with_borrow_mut(|folds| {
            if fold {
                folds.set().insert(group.to_owned());
            } else {
                folds.set().remove(group);
            }
        });
        cx.notify();
    }

    pub(crate) fn switch_profile(&mut self, instance: &Instance, cx: &mut Context<Self>) {
        if self.profile_lock().is_some() {
            return;
        }
        if !self
            .selected_instance()
            .is_some_and(|i| i.id == instance.id)
        {
            self.select_instance(instance);
            self.clear_profile_notices();
        }
        self.navigate(Page::Home);
        cx.notify();
    }

    pub(crate) fn clear_profile_notices(&mut self) {
        self.client_error = None;
        self.prepare_error = None;
        self.install_error = None;
        self.crash = None;
    }

    pub(crate) fn profile_mark(&mut self, i: &Instance, size: f32) -> Div {
        self.profile_mark_from(i, size, size)
    }

    /// A profile's mark drawn `size` px wide from the picture made for `source` px, so a mark
    /// that eases between two sizes keeps one decoded picture instead of asking for a new one
    /// every frame.
    fn profile_mark_from(&mut self, i: &Instance, size: f32, source: f32) -> Div {
        let p = palette();
        let picture = self.icon_image(format!("picture:{}", i.id), source, || {
            Some(i.home.join(instances::PICTURE))
        });
        let texture = match &i.icon {
            Some(icon @ Icon::Texture(name)) if picture.is_none() => {
                self.version_of(i).cloned().and_then(|tag| {
                    self.icon_image(format!("texture:{tag}:{name}"), source, || {
                        instances::icon_texture(icon, &backend::resources(&tag))
                    })
                })
            }
            _ => None,
        };
        let tile = div()
            .size(px(size))
            .flex_none()
            .rounded(px(size * 0.22))
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center();
        if let Some(image) = picture.or(texture) {
            return tile.child(
                gpui::img(image)
                    .size_full()
                    .object_fit(gpui::ObjectFit::Contain),
            );
        }
        match &i.icon {
            Some(Icon::Colour(hex)) => {
                let colour =
                    u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(p.selected);
                let ink = if sunrise_launcher_theme::contrast(colour, p.text)
                    > sunrise_launcher_theme::contrast(colour, p.background)
                {
                    p.text
                } else {
                    p.background
                };
                tile.bg(rgb(colour))
                    .text_color(rgb(ink))
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(size * 0.54))
                    .child(
                        i.name
                            .chars()
                            .next()
                            .unwrap_or('S')
                            .to_uppercase()
                            .to_string(),
                    )
            }
            _ => crate::mark(size),
        }
    }

    /// The profiles section: the heading, then the list, which takes the room the sidebar has.
    pub(super) fn profiles_section(&mut self, look: Look, cx: &mut Context<Self>) -> Div {
        let heading = self.profiles_heading(look, cx);
        let list = self.profile_list(look, cx);
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(px(4.))
            .child(heading)
            .child(list)
    }

    /// "Profiles", and the + button that makes a new one. Locked, the button says why.
    fn profiles_heading(&mut self, look: Look, cx: &mut Context<Self>) -> Div {
        let p = palette();
        let add = match look.lock {
            None => icon_button("profile-add", "plus-sign", tr("New profile"), Style::Ghost)
                .on_click(cx.listener(|this, _, _, cx| {
                    if this.profile_lock().is_none() {
                        this.new_profile(cx);
                    }
                })),
            Some(reason) => crate::widgets::button::icon_only(
                "profile-add",
                crate::widgets::button::Look::new(
                    Style::Ghost,
                    crate::widgets::button::Size::Medium,
                )
                .icon("plus-sign")
                .disabled(true),
            )
            .tooltip(crate::glass_ui::tip(reason)),
        };
        let title = div()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_size(px(tokens::CAPTION))
            .text_color(rgb(p.chrome_muted))
            .opacity(label_opacity(look.open))
            .child(tr("Profiles"));
        // Closed, the + alone is centred in the column.
        let pad = if look.open > 0. {
            ICON_PAD
        } else {
            (CLOSED_ROW - tokens::CONTROL) / 2.
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .px(px(pad))
            .when(look.open > 0., |d| d.child(title))
            .child(add)
    }

    /// A group's heading: its name (and how many profiles it holds when it is folded), over its
    /// first profile; pressing it folds the group. With the sidebar folded, a short rule that
    /// still marks where a group begins. The pointer on it slides in the button that opens the
    /// group's menu, as the pointer on a profile does.
    fn group_heading(
        &self,
        look: Look,
        group: &str,
        members: usize,
        open: f32,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = palette();
        let labelled = look.open > 0.;
        let hover_id = SharedString::from(format!("group-entry-{group}"));
        let busy = self.group_busy_with(group);
        let dots = labelled && actions_shown(look.open) && !self.location_unavailable;
        let mut heading = div()
            .id(hover_id.clone())
            .relative()
            .flex_none()
            .h(px(22.))
            .mt(px(6.))
            .px(px(9.))
            .flex()
            .items_end()
            .truncate()
            .text_size(px(crate::tokens::SMALL))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(p.chrome_muted))
            .opacity(label_opacity(look.open))
            .when(!labelled, |d| {
                d.h(px(1.))
                    .mt(px(4.))
                    .mx(px(8.))
                    .bg(rgb(p.chrome_muted))
                    .opacity(0.5)
            });
        if !labelled {
            return heading;
        }
        let toggled = group.to_owned();
        heading = heading
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_group(&toggled, cx)))
            .child(div().min_w_0().truncate().child(group.to_owned()))
            // How many it holds fades in as the group folds and out as it opens.
            .when(open < 1., |d| {
                d.child(
                    div()
                        .flex_none()
                        .ml(px(6.))
                        .opacity(0.7 * (1. - open))
                        .child(members.to_string()),
                )
            });
        if dots {
            heading = crate::widgets::track_hover(heading, hover_id.clone())
                .pr(px(56.))
                .children(group_actions(
                    group,
                    &hover_id,
                    busy,
                    self.profile_lock(),
                    cx,
                ));
            // The dots stay out while the menu they opened, or a dialog it led to, is open.
            crate::widgets::pin_hover(&hover_id, busy);
        }
        heading
    }

    /// The profiles, each group under its heading, in a list that scrolls. A folded group's
    /// profiles are left out (they slide shut and open as it turns over), and a group moved up
    /// or down glides to its new place with its profiles.
    fn profile_list(&mut self, look: Look, cx: &mut Context<Self>) -> Stateful<Div> {
        let selected = self.selected_instance().map(|i| i.id.clone());
        let rows = self.ui.profile_rows.clone();
        rows.borrow_mut().clear();
        let scroll = crate::frame::scrolls("sidebar");
        let list = div()
            .id("sidebar-profiles")
            .flex()
            .flex_col()
            .min_h_0()
            .flex_1()
            .gap(px(ROW_GAP))
            .overflow_y_scroll()
            .track_scroll(&scroll);
        let all = self.instances.list.clone();
        let headings = group_headings(all.iter().map(|i| i.group.as_deref()));
        let named: Vec<Option<&str>> = all
            .iter()
            .map(|i| i.group.as_deref().filter(|g| !g.trim().is_empty()))
            .collect();
        let ids: Vec<String> = all.iter().map(|i| i.id.clone()).collect();
        // Each group's rows shown, and their ids, for the glide of a group that moved.
        let order = instances::group_names(&all);
        let shown: HashMap<String, (usize, Vec<String>)> = order
            .iter()
            .map(|group| {
                let inside: Vec<String> = all
                    .iter()
                    .filter(|i| i.group.as_deref() == Some(group.as_str()))
                    .map(|i| i.id.clone())
                    .collect();
                let hidden = look.open > 0. && group_collapsed(group);
                (
                    group.clone(),
                    (if hidden { 0 } else { inside.len() }, inside),
                )
            })
            .collect();
        let step = row_height(look.open) + ROW_GAP;
        let head = if look.open > 0. {
            HEADING_EXTENT
        } else {
            RULE_EXTENT
        };
        let glides = GLIDES.with_borrow_mut(|glides| {
            let before = glides.moves;
            glides.note(&ids, step);
            // A group moved changes the profiles' order too: only the group's glide plays.
            if glides.note_groups(&order, &shown, step, head) && glides.moves > before + 1 {
                glides.started.retain(|_, (moved, _)| *moved != before + 1);
            }
            glides.started.clone()
        });
        let mut items: Vec<AnyElement> = Vec::new();
        let mut selected_at = None;
        let mut run: Option<Run> = None;
        for (n, i) in all.iter().cloned().enumerate() {
            if let Some(group) = headings[n].clone() {
                finish_run(run.take(), &mut items, &mut selected_at);
                let members = named[n..].iter().take_while(|g| **g == named[n]).count();
                // A folded group's profiles aren't built, once it has finished sliding shut.
                let folded = group_collapsed(&group);
                let (opened, fade) = FOLDS.with_borrow_mut(|folds| folds.note(&group, folded));
                let fade = fade.filter(|_| look.open > 0.);
                let hidden = look.open > 0. && folded && fade.is_none();
                let heading = self.group_heading(look, &group, members, opened, cx);
                let started = glides.get(&format!("group:{group}")).copied();
                items.push(match started {
                    Some(started) => glide(heading, &format!("group:{group}"), started),
                    None => heading.into_any_element(),
                });
                run = Some(Run {
                    group,
                    heading_at: items.len() - 1,
                    rows: Vec::new(),
                    selected: None,
                    hidden,
                    fade,
                    height: members as f32 * row_height(look.open)
                        + members.saturating_sub(1) as f32 * ROW_GAP,
                });
            } else if run
                .as_ref()
                .is_some_and(|r| named[n] != Some(r.group.as_str()))
            {
                finish_run(run.take(), &mut items, &mut selected_at);
            }
            let id = i.id.clone();
            let current = selected.as_deref() == Some(id.as_str());
            if let Some(run) = run.as_mut().filter(|r| r.hidden) {
                if current {
                    selected_at = Some(run.heading_at);
                }
                continue;
            }
            let entry = self.profile_entry(n, i, selected.as_deref(), look, cx);
            let row = glide(entry, &id, glides.get(&id).copied().unwrap_or((0, 0.)));
            match run.as_mut() {
                Some(run) => {
                    if current {
                        run.selected = Some(run.rows.len());
                    }
                    run.rows.push(row);
                }
                None => {
                    if current {
                        selected_at = Some(items.len());
                    }
                    items.push(row);
                }
            }
        }
        finish_run(run.take(), &mut items, &mut selected_at);
        // A profile selected from elsewhere (or at startup) scrolls into view, once.
        SHOWN.with_borrow_mut(|shown| {
            if *shown != selected {
                if let Some(at) = selected_at {
                    scroll.scroll_to_item(at);
                }
                *shown = selected.clone();
            }
        });
        list.children(items)
    }

    /// One profile: its face, and its actions button while it is the selected one. The
    /// selected profile looks selected only on the Home page. Folded, the entry is the highlight
    /// (selected or hovered), and the pointer on it grows the actions dots under the mark.
    fn profile_entry(
        &mut self,
        n: usize,
        i: Instance,
        selected: Option<&str>,
        look: Look,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = palette();
        let current = selected == Some(i.id.as_str());
        let on = current && look.on_home;
        // Every profile has its actions button (the default one too) while the sidebar is open.
        // Folded, a profile is its mark alone until the pointer is on it.
        let more = (current || look.open > 0.) && !self.location_unavailable;
        let face = self.profile_face(n, &i, current, more, look, cx);
        let hover_id = SharedString::from(format!("profile-entry-{}", i.id));
        let folded = folds_open(look);
        // Open, the dots slide in under the pointer, so the entry tracks it as the folded one does.
        let dots_out = more && actions_shown(look.open);
        // The menu or dialog the dots opened is up: their tooltip stays away.
        let busy = self.profile_busy_with(&i.id);
        let actions = if dots_out {
            profile_actions(&i, look.open, &hover_id, busy, cx)
        } else {
            Vec::new()
        };
        let mut entry = div()
            .id(hover_id.clone())
            .relative()
            .flex()
            .flex_col()
            .flex_none();
        if folded {
            entry = crate::widgets::track_hover(entry, hover_id.clone())
                .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                .when(on, |d| d.bg(rgb(p.selected)))
                .when(!on, |d| {
                    d.child(crate::widgets::hover_layer(
                        &hover_id,
                        rgb(p.chrome_raised).into(),
                        crate::tokens::R_CONTROL,
                        0.,
                    ))
                });
        } else if dots_out {
            entry = crate::widgets::track_hover(entry, hover_id.clone());
        }
        entry = entry.child(face).children(actions);
        if (folded || dots_out) && !self.location_unavailable {
            // The dots stay out while the menu they opened, or a dialog it led to, is open.
            crate::widgets::pin_hover(&hover_id, busy);
        }
        if folded && !self.location_unavailable {
            entry = entry.child(folded_dots(&i, &hover_id, busy, cx));
        }
        entry
    }

    /// A profile's button: its mark and its name (the selected one's more button sits over its
    /// right end). Right-clicking it opens its menu, and a press picks it unless the profiles
    /// are locked.
    fn profile_face(
        &mut self,
        n: usize,
        i: &Instance,
        current: bool,
        more: bool,
        look: Look,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let on = current && look.on_home;
        let labelled = look.open > 0.;
        let mut button = profile_button(
            ElementId::named_usize("sidebar-profile", n),
            i.name.clone(),
            on,
            look,
        )
        .child(
            icon_slot(self.profile_mark_from(i, mark_size(look.open), MARK_OPEN))
                .when(current, |d| {
                    d.debug_selector(|| "selected-profile-mark".into())
                }),
        );
        if labelled {
            // The channel of the build it plays (a Stable profile plays Nightly until Stable has
            // a build), as the banner's build line names it.
            let under = self
                .profile_target(i)
                .as_deref()
                .and_then(crate::sheets::dated_channel)
                .unwrap_or_else(|| crate::sheets::channel_name(i.channel));
            button = button.child(label_text(look.open, i.name.clone(), Some(under)));
        }
        if labelled && more {
            button = button.pr(px(36.));
        }
        if (i.kind == Kind::Own || more) && !self.location_unavailable {
            button = menu_target(button, i, self.ui.profile_rows.clone(), cx);
        }
        if look.lock.is_none() {
            let pick = i.clone();
            button = button.on_click(cx.listener(move |this, _, _, cx| {
                this.switch_profile(&pick, cx);
            }));
        }
        button
    }
}
