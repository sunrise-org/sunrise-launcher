//! A native build under way, shown once: the play bar's line, which is on every page, draws the
//! build's three stages and how far the current one is when the build says so, beside Stop. The
//! build gives no time left, so none is shown. The strip above the Mods lists says how it ended.
use super::*;
use crate::frame::Reach as _;
use crate::frame::{CARD, LINE, MUTED, SOFT, TEXT};
use gpui::{rgb, Animation, AnimationExt as _};
use native_mods::Stage;

impl Launcher {
    /// Whether the build that runs, or last ran, is the selected setup's.
    fn build_mine(&self) -> bool {
        self.selected_instance()
            .is_some_and(|i| self.native_for.as_deref() == Some(i.id.as_str()))
    }

    /// The step a running build is at, in words.
    fn build_step(&self) -> Option<String> {
        self.native_step.clone()
    }

    fn build_progress(&self) -> Option<native_mods::Progress> {
        self.native_progress
    }

    /// The three stages as one bar: done ones full, the current one as far as it reports, or
    /// breathing when it reports nothing.
    pub(crate) fn build_bar(&self) -> Div {
        let now = self.build_progress();
        let mut bar = div().flex().gap(px(4.)).h(px(4.));
        for (i, stage) in Stage::ALL.into_iter().enumerate() {
            let track = div()
                .flex_1()
                .h(px(4.))
                .rounded(px(tokens::R_BAR))
                .bg(rgb(LINE))
                .overflow_hidden();
            let fill = div().h(px(4.)).rounded(px(tokens::R_BAR)).bg(rgb(TEXT));
            bar = bar.child(match now {
                Some(p) if stage < p.stage => track.child(fill.w_full()),
                Some(p) if stage == p.stage => match p.fraction {
                    Some(f) => track.child(fill.w(relative(f.clamp(0.03, 1.)))),
                    None if crate::motion::still() => track.child(fill.w_full().opacity(0.5)),
                    None => track.child(
                        fill.w_full().with_animation(
                            ElementId::named_usize("alt-build-stage", i),
                            Animation::new(Duration::from_millis(1400))
                                .repeat()
                                .with_easing(gpui::ease_in_out),
                            |d, t| d.opacity(0.25 + 0.5 * (1. - (2. * t - 1.).abs())),
                        ),
                    ),
                },
                // Before the first step is in, the first stage is the one under way.
                None if i == 0 => track.child(fill.w_full().opacity(0.4)),
                _ => track,
            });
        }
        bar
    }

    /// "Stage 2 of 3 · Build".
    fn build_stage(&self) -> String {
        let stage = self.build_progress().map_or(Stage::Prepare, |p| p.stage);
        let n = Stage::ALL.iter().position(|s| *s == stage).unwrap_or(0) + 1;
        trf(
            "Stage {n} of 3 · {stage}",
            &[("n", &n.to_string()), ("stage", stage.label())],
        )
    }

    /// The play bar's line while a build runs: what it is doing, over the bar.
    pub(crate) fn build_line(&self) -> Option<Div> {
        let step = self.build_step()?;
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .max_w(px(420.))
                .child(div().truncate().child(trf(
                    "{stage} · {step}",
                    &[("stage", &self.build_stage()), ("step", &step)],
                )))
                .child(self.build_bar()),
        )
    }

    /// Above the Mods lists: how the selected setup's build ended, and during a first build what
    /// it downloads.
    pub(crate) fn build_strip(&self, cx: &mut Context<Self>) -> Option<Div> {
        if !self.build_mine() {
            return None;
        }
        let said = |text: String, cx: &mut Context<Self>| {
            div()
                .flex()
                .items_start()
                .gap(px(12.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(tokens::LABEL))
                        .child(text),
                )
                .child(
                    div()
                        .id("alt-build-dismiss")
                        .reach()
                        .flex_none()
                        .text_size(px(tokens::SMALL))
                        .text_color(rgb(MUTED))
                        .cursor_pointer()
                        .child(tr("Dismiss"))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.native_done = None;
                            cx.notify();
                        })),
                )
        };
        if self.build_step().is_none() {
            // How it went. A failure is the play bar's to say, in one place.
            return self
                .native_done
                .clone()
                .filter(|_| self.native_error.is_none())
                .map(|done| said(done, cx));
        }
        // How far it is, and Stop, are the play bar's, on every page. Only what a first build
        // brings with it is said here.
        self.first_build.map(|bytes| {
            div()
                .p(px(14.))
                .rounded(px(tokens::R_CARD))
                .border_1()
                .border_color(rgb(SOFT))
                .bg(rgb(CARD))
                .text_size(px(tokens::LABEL))
                .text_color(rgb(MUTED))
                .child(crate::selectable::selectable(trf(
                    "The first build downloads the compiler ({size} MB), once. You can keep using the launcher.",
                    &[("size", &bytes.div_ceil(1_000_000).to_string())],
                )))
        })
    }
}
