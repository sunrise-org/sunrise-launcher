//! Settings › Appearance: the three text boxes (a colour's hex, a theme's name, a pasted
//! code), drawn by one function.
use super::*;
use crate::widgets::field::field_box;

/// One text box: where its text and its keyboard focus are kept, and what it says empty.
#[derive(Clone, Copy)]
pub(super) struct Field {
    pub(super) id: &'static str,
    pub(super) edit: fn(&mut Launcher) -> &mut line_edit::LineEdit,
    pub(super) focus: fn(&mut Launcher) -> &mut Option<FocusHandle>,
    pub(super) hint: &'static str,
}

pub(super) const HEX: Field = Field {
    id: "alt-colour-hex",
    edit: |this| &mut this.ui.settings.appearance.hex,
    focus: |this| &mut this.ui.settings.appearance.hex_focus,
    hint: crate::i18n::tr_key("#rrggbb"),
};

pub(super) const NAME: Field = Field {
    id: "alt-theme-name",
    edit: |this| &mut this.ui.settings.appearance.name,
    focus: |this| &mut this.ui.settings.appearance.name_focus,
    hint: crate::i18n::tr_key("Theme name"),
};

pub(super) const CODE: Field = Field {
    id: "alt-theme-code",
    edit: |this| &mut this.ui.settings.appearance.code,
    focus: |this| &mut this.ui.settings.appearance.code_focus,
    hint: crate::i18n::tr_key("Paste a theme code"),
};

/// What Enter does in a text box.
pub(super) type Enter = fn(&mut Launcher, &mut Window, &mut Context<Launcher>);

impl Launcher {
    /// The box for `field`. Enter runs `enter`, Escape runs `text_cancel`; the caller sets its
    /// width.
    pub(super) fn text_input(
        &mut self,
        field: Field,
        enter: Enter,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let focus = (field.focus)(self)
            .get_or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone();
        let focused = focus.is_focused(window);
        let theme = crate::mods::field_theme(self.theme);
        let edit = field.edit;
        let to_focus = focus.clone();
        let content = line_edit::content(edit(self), &theme, focused, false, false, field.hint);
        let boxed = field_box(focused, false)
            .id(field.id)
            .track_focus(&focus)
            .seen()
            .px(px(12.))
            .font_family(theme::MONO)
            .text_size(px(tokens::LABEL))
            .text_color(rgb(palette().text))
            .cursor_text()
            .child(content)
            .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
                window.focus(&to_focus, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
                match line_edit::handle(edit(this), ev, cx) {
                    line_edit::Key::Pass => return,
                    line_edit::Key::Enter => enter(this, window, cx),
                    line_edit::Key::Escape => this.text_cancel(window, cx),
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        line_edit::mouse(boxed, cx, edit)
    }
}
