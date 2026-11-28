//! "What to copy": the options of a copy, as a two-column grid of checkboxes.
use super::*;
use crate::widgets::button::link;
use crate::widgets::check::{checkbox, Tick};
use crate::widgets::focus::Reach;
use gpui::AnimationExt as _;
use instances::CopyOptions;

/// The options in the grid's order, each with its label.
const LABELS: [&str; 8] = [
    i18n::tr_key("Keep play time"),
    i18n::tr_key("Copy worlds"),
    i18n::tr_key("Copy game options"),
    i18n::tr_key("Copy resource packs"),
    i18n::tr_key("Copy screenshots"),
    i18n::tr_key("Copy shader packs"),
    i18n::tr_key("Copy servers"),
    i18n::tr_key("Copy mods"),
];

/// Option `n` of [`LABELS`].
pub(super) fn slot(options: &mut CopyOptions, n: usize) -> &mut bool {
    match n {
        0 => &mut options.play_time,
        1 => &mut options.worlds,
        2 => &mut options.options,
        3 => &mut options.resource_packs,
        4 => &mut options.screenshots,
        5 => &mut options.shader_packs,
        6 => &mut options.servers,
        _ => &mut options.mods,
    }
}

pub(super) fn is_on(options: CopyOptions, n: usize) -> bool {
    let mut options = options;
    *slot(&mut options, n)
}

impl Launcher {
    /// The box under Start from while a copy is chosen; it eases open.
    pub(super) fn copy_box(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let copy = self.instances.dialog.as_ref()?.copy.as_ref()?;
        let options = copy.options();
        let p = palette();
        let shown: Vec<usize> = (0..LABELS.len()).collect();
        let all_on = shown.iter().all(|n| is_on(options, *n));
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(p.muted))
                    .child(tr("What to copy")),
            )
            .child(
                link(
                    "new-copy-all",
                    if all_on {
                        tr("Select none")
                    } else {
                        tr("Select all")
                    },
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(copy) = this
                        .instances
                        .dialog
                        .as_mut()
                        .filter(|d| !d.busy)
                        .and_then(|d| d.copy.as_mut())
                    {
                        let now = copy.options();
                        let all_on = (0..LABELS.len()).all(|n| is_on(now, n));
                        copy.set_options(if all_on {
                            CopyOptions::nothing()
                        } else {
                            CopyOptions::everything()
                        });
                    }
                    cx.notify();
                })),
            );
        let mut grid = div().flex().flex_col().gap(px(2.));
        for pair in shown.chunks(2) {
            let mut line = div().flex().gap(px(12.));
            for &n in pair {
                let on = is_on(options, n);
                line = line.child(
                    div()
                        .id(ElementId::named_usize("new-copy-option", n))
                        .reach()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .h(px(30.))
                        .px(px(4.))
                        .rounded(tokens::r(tokens::R_SMALL))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(p.hover)))
                        .child(
                            checkbox(
                                ElementId::named_usize("new-copy-check", n),
                                if on { Tick::On } else { Tick::Off },
                            )
                            .tab_stop(false),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(px(tokens::LABEL))
                                .child(tr(LABELS[n])),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(copy) = this
                                .instances
                                .dialog
                                .as_mut()
                                .filter(|d| !d.busy)
                                .and_then(|d| d.copy.as_mut())
                            {
                                let mut now = copy.options();
                                let slot = slot(&mut now, n);
                                *slot = !*slot;
                                copy.set_options(now);
                            }
                            cx.notify();
                        })),
                );
            }
            // A last odd option keeps its column's width.
            if pair.len() == 1 {
                line = line.child(div().flex_1());
            }
            grid = grid.child(line);
        }
        let panel = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            // No box: a rule above, then the options on the dialog's own fill.
            .pt(px(16.))
            .border_t_1()
            .border_color(rgb(p.divider))
            .overflow_hidden()
            .child(header)
            .child(grid);
        // It opens a little at a time, and is simply there under reduced motion.
        Some(
            panel
                .with_animation(
                    "new-copy-box",
                    crate::motion::eased(sunrise_launcher_theme::QUICK),
                    |d, t| d.max_h(px(40. + 220. * t)).opacity(t),
                )
                .into_any_element(),
        )
    }
}
