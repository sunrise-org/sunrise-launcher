//! The window's root: the launcher, drawn from its last frame (GPUI Fast retains views).
use crate::Launcher;
use gpui::{div, prelude::*, Context, Entity, IntoElement, Render, Window};

pub struct Shell {
    launcher: Entity<Launcher>,
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_startup_checks(window, cx, true)
    }

    pub(crate) fn with_startup_checks(
        window: &mut Window,
        cx: &mut Context<Self>,
        startup_checks: bool,
    ) -> Self {
        let launcher = cx.new(|cx| Launcher::new_with_startup_checks(window, cx, startup_checks));
        Self { launcher }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_startup_checks(window, cx, false)
    }

    /// The launcher it draws, for tests that drive the window as it opens.
    #[cfg(test)]
    pub(crate) fn launcher(&self) -> Entity<Launcher> {
        self.launcher.clone()
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            // Cached: the launcher is drawn again only when it is notified.
            .child(
                self.launcher
                    .clone()
                    .cached(gpui::StyleRefinement::default().size_full()),
            )
    }
}
