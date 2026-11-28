//! Settings › Appearance: the page itself (the theme gallery and the current theme's bar, then
//! what is behind Home, the layout and the cat) and the rows that are not about colours.
use super::*;
use crate::widgets::button::secondary;
use crate::widgets::check::switch;
use crate::widgets::feedback::{banner, Tone};
use gpui::SharedString;
use sunrise_launcher::appearance::{self, Behind};
use sunrise_launcher::theme_files::{self, Entry};
use sunrise_launcher::themes;

impl Launcher {
    pub(in crate::settings) fn settings_appearance(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        // A dialog closed without Done is undone before the page is drawn from the look.
        if self.edit_revert(window) {
            cx.notify();
        }
        let missing = appearance::missing(&self.settings);
        let mut rows: Vec<Div> = self.unusable_themes().into_iter().collect();
        rows.push(self.folder_row(cx));
        rows.push(self.background_row(cx));
        rows.push(self.scenery_row(cx));
        rows.push(self.reduced_graphics_row(cx));
        rows.push(self.header_row(cx));
        rows.push(self.worlds_row(cx));
        rows.push(self.corners_row(cx));
        rows.push(self.cat_row(cx));
        let state = &self.ui.settings.appearance;
        let said = state
            .error
            .clone()
            .map(|t| banner(Tone::Danger, t, None))
            .or_else(|| state.note.clone().map(hint));
        let gone = missing.then(|| {
            banner(Tone::Danger, trf(
                "The chosen theme, {id}, can't be read. Dawn is shown until it is repaired: fix its file, then press Reload.",
                &[("id", self.settings.theme.as_deref().unwrap_or_default())],
            ), None)
        });
        div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .children(gone)
            .child(self.themes_section(cx))
            .child(self.your_themes_section(cx))
            .child(self.current_card(cx))
            .child(group(rows))
            .children(said)
    }

    /// The folders in the themes folder that aren't themes, each with why.
    fn unusable_themes(&self) -> Option<Div> {
        let bad: Vec<Div> = theme_files::loaded()
            .iter()
            .filter_map(|entry| match entry {
                Entry::Bad { folder, reason } => Some(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(10.))
                        .opacity(0.9)
                        .child(
                            blocks::icon("info-box", 15., rgb(palette().warning).into()).mt(px(2.)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_size(px(tokens::LABEL))
                                        .child(trf(
                                            "{folder} can't be used",
                                            &[("folder", folder)],
                                        )),
                                )
                                .child(hint(reason.text())),
                        ),
                ),
                Entry::Good(_) => None,
            })
            .collect();
        (!bad.is_empty()).then(|| div().flex().flex_col().gap(px(10.)).children(bad))
    }

    fn reduced_graphics_row(&mut self, cx: &mut Context<Self>) -> Div {
        let reduced = self.settings.reduced_graphics();
        row(
            tr("Reduced graphics"),
            tr("Keeps the Home banner's scene and the cat still. Also reduces motion in the launcher."),
            div().child(
                switch("alt-reduced-graphics", reduced)
                    .seen()
                    .tooltip(crate::glass_ui::tip(tr("Reduced graphics")))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.appearance_change(
                            |s| {
                                s.reduced_graphics = Some(!reduced);
                                Ok(())
                            },
                            window,
                        );
                        cx.notify();
                    })),
            ),
        )
    }

    /// Whether Home lists its worlds as rows or as tiles.
    fn worlds_row(&mut self, cx: &mut Context<Self>) -> Div {
        let layout = self.settings.world_layout;
        let layouts: Vec<(backend::WorldLayout, SharedString)> = [
            (backend::WorldLayout::Rows, crate::i18n::tr_key("Rows")),
            (backend::WorldLayout::Tiles, crate::i18n::tr_key("Tiles")),
        ]
        .into_iter()
        .map(|(way, label)| (way, tr(label).into()))
        .collect();
        let this = cx.entity();
        // Both descriptions are laid out, so the row is as tall for Rows as for Tiles.
        let about = hint_of(
            vec![
                tr("Continue playing lists one world a row.").into(),
                tr("Continue playing shows the worlds' pictures side by side, as many as fit across; the rest wrap under them.").into(),
            ],
            match layout {
                backend::WorldLayout::Rows => 0,
                backend::WorldLayout::Tiles => 1,
            },
        );
        row_with(tr("Worlds on Home"), Some(about), {
            let state = &mut self.ui.settings.appearance;
            let focus = state
                .worlds_focus
                .get_or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
                .clone();
            // Opened from the search: the page scrolls here and the keyboard is on it.
            let asked = state.worlds_asked.clone().map(|done| {
                crate::mods::reveal(crate::frame::scrolls("settings"), done, Some(focus.clone()))
            });
            div().child(
                crate::widgets::choice::choices(
                    "alt-world-layout",
                    &layouts,
                    layout,
                    move |way, _, cx| {
                        this.update(cx, |this, cx| {
                            this.settings.world_layout = way;
                            this.save_settings();
                            cx.notify();
                        })
                    },
                )
                .track_focus(&focus)
                .children(asked),
            )
        })
    }

    /// The themes folder: opening it, and reading it again.
    fn folder_row(&mut self, cx: &mut Context<Self>) -> Div {
        self.wide_row(
                tr("Themes folder"),
                tr("A theme is a folder with a theme.toml: up to nine colours and one picture. Reload reads the folder again."),
                div()
                    .child(
                        secondary("alt-themes-folder", tr("Open themes folder")).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                let dir = theme_files::dir();
                                // A link there is not the launcher's folder: never opened
                                // or made through.
                                if theme_files::root_linked(&dir) {
                                    this.ui.settings.appearance.error = Some(format!(
                                        "{}: {}",
                                        dir.display(),
                                        theme_files::Reason::Linked(crate::i18n::tr_key("folder")).text()
                                    ));
                                    cx.notify();
                                    return;
                                }
                                match std::fs::create_dir_all(&dir) {
                                    Ok(()) => crate::toast::open(cx, &dir),
                                    Err(e) => {
                                        this.ui.settings.appearance.error =
                                            Some(format!("{}: {e}", dir.display()))
                                    }
                                }
                                cx.notify();
                            }),
                        ),
                    )
                    .child(secondary("alt-themes-reload", tr("Reload")).on_click(cx.listener(
                        |this, _: &ClickEvent, window, cx| {
                            this.appearance_reload(window);
                            cx.notify();
                        },
                    ))),
            )
    }

    fn cat_row(&mut self, cx: &mut Context<Self>) -> Div {
        let cat = self.settings.show_cat();
        row(
            tr("Show the cat"),
            tr("It visits Home now and then. It stays away while motion is reduced."),
            // On, the switch runs from the pointer (a keyboard still turns it off).
            crate::widgets::shy_switch::wrap(
                "alt-cat-toggle",
                cat,
                switch("alt-cat-toggle", cat).seen().on_click(cx.listener(
                    move |this, ev: &ClickEvent, _, cx| {
                        if cat && !ev.is_keyboard() && !crate::motion::still() {
                            return;
                        }
                        this.settings.cat = Some(!cat);
                        this.save_settings();
                        cx.notify();
                    },
                )),
            ),
        )
    }

    /// What is behind the pages: where it comes from, a picture of the player's own, the way back
    /// to the theme's, and the switch for all of it.
    fn background_row(&mut self, cx: &mut Context<Self>) -> Div {
        let data = backend::data_dir();
        // A kept copy the launcher could not decode is not what is behind the pages.
        let unshown = self
            .ui
            .settings
            .appearance
            .unshown
            .as_ref()
            .is_some_and(|path| {
                appearance::behind_home(&self.settings, &data) == Behind::Personal(path.clone())
            });
        let behind = if unshown {
            appearance::behind_without_own(&self.settings)
        } else {
            appearance::behind_home(&self.settings, &data)
        };
        let lost = appearance::background_lost(&self.settings, &data);
        let source = match &behind {
            Behind::Plain => tr("Off: the Home banner is plain."),
            Behind::Personal(_) => tr("Your picture, in the Home banner."),
            Behind::Theme(_) => tr("The theme's picture, in the Home banner."),
            Behind::Scene => tr("The theme's scene."),
        };
        let about = if unshown {
            format!(
                "{} {}",
                source,
                tr("The picture you chose can't be shown; choose it again or use the theme's.")
            )
        } else if lost {
            format!(
                "{} {}",
                source,
                tr("The picture you chose is no longer there; choose it again or use the theme's.")
            )
        } else {
            source.to_string()
        };
        let choose = secondary("alt-background-choose", tr("Choose picture…")).on_click(
            cx.listener(|_, _: &ClickEvent, window, cx| {
                let paths = cx.prompt_for_paths(PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: false,
                    prompt: Some(tr("Choose").into()),
                });
                let window = window.window_handle();
                cx.spawn(async move |this, cx| {
                    // Closing the dialog keeps the picture there was.
                    let Ok(Ok(Some(mut paths))) = paths.await else {
                        return;
                    };
                    let Some(file) = paths.pop() else {
                        return;
                    };
                    // Read, checked and decoded off the window's thread, then kept.
                    let kept = cx
                        .background_executor()
                        .spawn(async move { themes::keep_background(&backend::data_dir(), &file) })
                        .await;
                    let _ = window.update(cx, |_, window, cx| {
                        let _ = this.update(cx, |this, cx| {
                            this.appearance_change(
                                |s| {
                                    s.background = Some(kept?);
                                    Ok(())
                                },
                                window,
                            );
                            cx.notify();
                        });
                    });
                })
                .detach();
            }),
        );
        let theirs = secondary("alt-background-theme", tr("Use theme background"));
        let theirs = if self.settings.background.is_some() {
            theirs.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.appearance_change(
                    |s| {
                        s.background = None;
                        Ok(())
                    },
                    window,
                );
                cx.notify();
            }))
        } else {
            off(theirs)
        };
        self.wide_row(
            tr("Custom background"),
            about,
            div().child(choose).child(theirs),
        )
    }

    /// Whether Home shows its background at all: a row of its own, its switch on the right like
    /// every other switch on the page.
    fn scenery_row(&mut self, cx: &mut Context<Self>) -> Div {
        let on = self.settings.scenery();
        let show = switch("alt-scenery", on).seen().on_click(cx.listener(
            move |this, _: &ClickEvent, window, cx| {
                this.appearance_change(
                    |s| {
                        s.scenery_on = Some(!on);
                        Ok(())
                    },
                    window,
                );
                cx.notify();
            },
        ));
        row(tr("Show the Home banner scene"), "", div().child(show))
    }
}
