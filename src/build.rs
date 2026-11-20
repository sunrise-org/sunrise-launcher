//! A native build under way, as the launcher says it outside its own window: one line, "Building
//! your mods · 2 of 3 · 51%", over one slim bar. The status strip draws it on every tab, Home
//! beside Play; a press on it opens the build window again. When a build ends with its window
//! closed, the same places say how it went for a moment.
use super::*;
use crate::frame::Reach as _;
use crate::paint::palette;
use gpui::rgb;

impl Launcher {
    /// The line while a build runs: how far it is, over its bar (`bar`: a page that shows the
    /// bar itself draws the line without one, so one build never has two bars). A press opens
    /// the build window.
    pub(crate) fn build_line(&self, bar: bool, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let run = self.ui.mods.build.as_ref().filter(|run| run.running())?;
        self.native_step.as_ref()?;
        let fraction = run.fraction();
        Some(
            div()
                .id("alt-build-open")
                .reach()
                .flex()
                .flex_col()
                .gap(px(6.))
                .max_w(px(420.))
                .cursor_pointer()
                .tooltip(crate::glass_ui::tip(tr("Show the build")))
                .child(div().truncate().child(run.line()))
                .when(bar, |d| d.child(progress_bar(fraction)))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.build_all();
                    cx.notify();
                })),
        )
    }

    /// How the last build went, for the moment after it ended with its window closed.
    pub(crate) fn build_said(&self) -> Option<String> {
        self.ui.mods.build.as_ref()?.said(Instant::now())
    }
}

/// A build's one bar: slim, in the accent.
pub(crate) fn progress_bar(fraction: f32) -> Div {
    div()
        .h(px(4.))
        .rounded(crate::tokens::r(crate::tokens::R_BAR))
        .bg(rgb(palette().line))
        .overflow_hidden()
        .child(
            div()
                .h(px(4.))
                .rounded(crate::tokens::r(crate::tokens::R_BAR))
                .bg(rgb(palette().accent))
                .w(relative(fraction.clamp(0.02, 1.))),
        )
}
