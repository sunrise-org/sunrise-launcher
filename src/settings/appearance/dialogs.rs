//! Settings › Appearance: the theme library's dialogs, sheets over the window: naming the
//! colours being edited (Save as theme), renaming a theme, importing a code (with a look at the
//! themes in it before they are added) and sharing themes as a code.
use super::inputs::{CODE, NAME};
use super::preview::theme_card;
use super::state::Dialog;
use super::*;
use crate::sheets::{frame, title};
use crate::widgets::button::{primary, secondary};
use gpui::SharedString;
use sunrise_launcher::themes::library;
use sunrise_launcher::themes::share;

/// A dialog's width.
const WIDTH: f32 = 520.;
/// How tall a list of themes in a dialog grows before it scrolls.
const LIST_MAX: f32 = 280.;

impl Launcher {
    /// The sheet for the open dialog, or None when none is open (the sheet then closes). Its
    /// text box has the keyboard from the first frame it is drawn.
    pub(crate) fn theme_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        let dialog = self.ui.settings.appearance.dialog.clone()?;
        let state = &mut self.ui.settings.appearance;
        if std::mem::take(&mut state.dialog_focus) {
            let ready = || cx.focus_handle().tab_index(0).tab_stop(true);
            let handle = match &dialog {
                Dialog::Save | Dialog::Rename(_) => {
                    Some(state.name_focus.get_or_insert_with(ready).clone())
                }
                Dialog::Import => Some(state.code_focus.get_or_insert_with(ready).clone()),
                Dialog::Export { .. } => None,
            };
            if let Some(handle) = handle {
                window.focus(&handle, cx);
            }
        }
        let body = match dialog {
            Dialog::Save => self.name_dialog(false, window, cx),
            Dialog::Rename(_) => self.name_dialog(true, window, cx),
            Dialog::Import => self.import_dialog(window, cx),
            Dialog::Export { code, themes } => self.export_dialog(code, &themes, cx),
        };
        Some(frame("alt-theme-dialog", WIDTH, body, cx))
    }

    /// The Cancel (or Close) button of a dialog.
    fn dialog_cancel(&self, label: &'static str, cx: &mut Context<Self>) -> Stateful<Div> {
        secondary("alt-dialog-cancel", tr(label)).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.close_sheet();
                cx.notify();
            },
        ))
    }

    /// A dialog's main button: it does what the dialog is for when `ready`, and is dimmed and
    /// dead until then.
    fn dialog_ok(
        &self,
        label: impl Into<SharedString>,
        ready: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let ok = primary("alt-dialog-ok", label);
        if ready {
            ok.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.dialog_confirm(window, cx);
                cx.notify();
            }))
        } else {
            off(ok)
        }
    }

    /// Why the last try failed, in the dialog.
    fn dialog_problem(&self, text: impl Into<SharedString>) -> Div {
        div()
            .text_size(px(tokens::SMALL))
            .text_color(rgb(palette().danger))
            .child(text.into())
    }

    /// Save as theme and Rename: a name, and the button that takes it.
    fn name_dialog(&mut self, renaming: bool, window: &Window, cx: &mut Context<Self>) -> Div {
        let heading = if renaming {
            tr("Rename theme")
        } else {
            tr("Save as theme")
        };
        let field = self
            .text_input(NAME, Self::dialog_confirm, window, cx)
            .w_full();
        let state = &self.ui.settings.appearance;
        let ready = !state.name.text().trim().is_empty();
        let problem = state
            .dialog_error
            .clone()
            .map(|why| self.dialog_problem(why));
        let ok = self.dialog_ok(if renaming { tr("Rename") } else { tr("Save") }, ready, cx);
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(title(heading.into()))
            .child(field)
            .children(problem)
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(self.dialog_cancel("Cancel", cx))
                    .child(ok),
            )
    }

    /// A pasted code, and a look at the themes it holds, in their colours, before they are
    /// added.
    fn import_dialog(&mut self, window: &Window, cx: &mut Context<Self>) -> Div {
        let field = self
            .text_input(CODE, Self::dialog_confirm, window, cx)
            .w_full();
        let state = &self.ui.settings.appearance;
        let typed = state.code.text().to_string();
        let decoded = (!typed.trim().is_empty()).then(|| share::decode(&typed));
        let problem = state.dialog_error.clone();
        let (found, count) = match &decoded {
            Some(Ok(themes)) => (Some(theme_list("alt-import", themes)), themes.len()),
            Some(Err(why)) => (Some(self.dialog_problem(why.text())), 0),
            None => (
                Some(hint(tr("Paste a theme code to see the themes in it."))),
                0,
            ),
        };
        let label = if count > 0 {
            trf("Import {count}", &[("count", &count.to_string())])
        } else {
            tr("Import").to_string()
        };
        let ok = self.dialog_ok(label, count > 0, cx);
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(title(tr("Import themes").into()))
            .child(field)
            .children(found)
            .children(problem.map(|why| self.dialog_problem(why)))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(self.dialog_cancel("Cancel", cx))
                    .child(ok),
            )
    }

    /// The code for `themes`, to read and to copy, and the themes in it.
    fn export_dialog(
        &mut self,
        code: String,
        themes: &[library::Custom],
        cx: &mut Context<Self>,
    ) -> Div {
        let heading = match themes {
            [one] => trf("Share {name}", &[("name", one.name.as_str())]),
            _ => trf(
                "Share {count} themes",
                &[("count", &themes.len().to_string())],
            ),
        };
        let shown = div()
            .id("alt-export-code")
            .max_h(px(120.))
            .overflow_y_scroll()
            .p(px(10.))
            // Room on the right for the copy icon in the corner.
            .pr(px(44.))
            .rounded(tokens::r(tokens::R_SMALL))
            .bg(rgb(palette().background))
            .border_1()
            .border_color(rgb(palette().line))
            .font_family(theme::MONO)
            .text_size(px(tokens::SMALL))
            .child(crate::selectable::selectable(code.clone()));
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(title(heading))
            // The copy icon in the code box's top-right corner.
            .child(
                div().relative().child(shown).child(
                    div()
                        .absolute()
                        // The icon's own -4 px margin keeps a toast line's height: 6 px in here.
                        .top(px(10.))
                        .right(px(6.))
                        .child(crate::widgets::motion_bits::copy_icon(
                            "alt-export-copy",
                            code,
                        )),
                ),
            )
            .child(theme_list("alt-export", themes))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .child(self.dialog_cancel("Close", cx)),
            )
    }
}

/// The themes of a code, one card each, scrolling when there are many.
fn theme_list(id: &'static str, themes: &[library::Custom]) -> Div {
    let mut list = div()
        .id(id)
        .flex()
        .flex_col()
        .gap(px(8.))
        .max_h(px(LIST_MAX))
        .overflow_y_scroll();
    for (i, theme) in themes.iter().enumerate() {
        list = list.child(theme_card(SharedString::from(format!("{id}-{i}")), theme));
    }
    div().child(list)
}
