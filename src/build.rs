//! A native build under way, as the launcher says it outside its own window: one line, "Building
//! your mods · 2 of 3 · 51%", over one slim bar. The status strip draws it on every tab, Home
//! beside Play; a press on it opens the build window again. When a build ends with its window
//! closed, the same places say how it went for a moment.
use super::*;
use crate::frame::Reach as _;
use crate::mods::build_run::Run;

impl Launcher {
    /// The build window's build, when it is the selected profile's.
    pub(crate) fn build_here(&self) -> Option<&Run> {
        let selected = self.selected_instance()?;
        self.mods_state
            .build
            .as_ref()
            .filter(|run| run.instance == selected.id)
    }

    /// The running build's progress. Pressing it opens the build window.
    pub(crate) fn build_line(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let run = self.mods_state.build.as_ref().filter(|run| run.running())?;
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
                .child(crate::widgets::feedback::progress_bar(Some(fraction)))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.build_all();
                    cx.notify();
                })),
        )
    }

    /// How the last build went, for the moment after it ended with its window closed.
    pub(crate) fn build_said(&self) -> Option<String> {
        self.mods_state.build.as_ref()?.said(Instant::now())
    }
}
