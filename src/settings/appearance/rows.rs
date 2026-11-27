//! Settings › Appearance: the theme cards' preview, and the rows for colours, opacity,
//! layout and the section's own picture and cat settings.
use super::*;
use crate::paint::Palette;
use crate::widgets::button::secondary;
use crate::widgets::check::switch;
use crate::widgets::feedback::{banner, Tone};
use gpui::SharedString;
use sunrise_launcher::appearance::{self, Behind};
use sunrise_launcher::backend::{PANEL_OPACITY_MAX, PANEL_OPACITY_MIN};
use sunrise_launcher::theme_files::{self, Entry};
use sunrise_launcher::themes::{self, Accent};

/// An accent swatch's size, its ring included.
const SWATCH: f32 = 30.;

/// What [`Accent::ALL`]'s swatches are called, in their order.
const ACCENT_NAMES: [&str; 7] = ["Violet", "Blue", "Teal", "Green", "Amber", "Red", "Pink"];

/// A theme's preview: its page, a panel on it with both inks, and Play in its own fill.
/// The least width of a theme's card, and the room between cards.
const CARD_MIN: f32 = 168.;
const GAP: f32 = 12.;

thread_local! {
    /// Where the grid of themes was last drawn, for its number of columns.
    static THEME_GRID: gpui::ScrollHandle = gpui::ScrollHandle::new();
}

fn preview(p: &Palette) -> Div {
    let bar = |w: f32, color: u32| {
        div()
            .h(px(4.))
            .w(px(w))
            .rounded(crate::tokens::r(crate::tokens::R_BAR))
            .bg(rgb(color))
    };
    div()
        .h(px(92.))
        .p(px(10.))
        .bg(rgb(p.background))
        .flex()
        .gap(px(6.))
        .child(
            div()
                .w(px(14.))
                .h_full()
                .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                .bg(rgb(p.chrome))
                .border_1()
                .border_color(rgb(p.chrome_line))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.))
                .pt(px(6.))
                .child(bar(6., p.chrome_text))
                .child(bar(6., p.chrome_muted)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                .bg(rgb(p.card_color))
                .border_1()
                .border_color(rgb(p.divider))
                .p(px(8.))
                .flex()
                .flex_col()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(px(tokens::LABEL))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(p.text))
                                .truncate()
                                .child(tr("Selected profile")),
                        )
                        .child(
                            div()
                                .text_size(px(tokens::CAPTION))
                                .text_color(rgb(p.muted))
                                .truncate()
                                .child(tr("Not played yet")),
                        ),
                )
                .child(
                    div().flex().child(
                        div()
                            .px(px(10.))
                            .h(px(20.))
                            .flex()
                            .items_center()
                            .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                            .border_1()
                            .border_color(p.input_edge())
                            .bg(p.play_gradient())
                            .text_size(px(tokens::CAPTION))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(p.accent_on))
                            .child(tr("Play")),
                    ),
                ),
        )
}

const OPACITY_SPAN: f32 = (PANEL_OPACITY_MAX - PANEL_OPACITY_MIN) as f32;

fn opacity_percent(value: f32) -> u8 {
    PANEL_OPACITY_MIN + (value * OPACITY_SPAN).round() as u8
}

impl Launcher {
    /// The frame previews a held slider, but navigation away discards it without saving.
    pub(crate) fn panel_opacity_preview(&self) -> Option<u8> {
        let slider = &self.ui.settings.appearance.panel_opacity;
        if self.ui.page == crate::frame::Page::Settings
            && self.ui.settings.section == Section::Appearance
        {
            slider.preview().map(opacity_percent)
        } else {
            slider.cancel();
            None
        }
    }

    pub(in crate::settings) fn settings_appearance(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        self.appearance_choices();
        let missing = appearance::missing(&self.settings);
        let selected = match &self.settings.theme {
            // A missing theme is still the choice: nothing else is ticked in its place.
            Some(id) if missing => id.clone(),
            _ => appearance::effective_id(&self.settings),
        };
        // As many columns as fit at a card's least width, and the cards share the row between
        // them: no strip of empty space on the right at any width. The row's width is the one it
        // was last drawn at.
        let grid = THEME_GRID.with(|g| g.clone());
        let width = f32::from(grid.bounds().size.width);
        let columns = (((width + GAP) / (CARD_MIN + GAP)).floor() as u16).max(1);
        let mut cards = div()
            .id("alt-theme-grid")
            .track_scroll(&grid)
            .grid()
            .grid_cols(columns)
            .gap(px(GAP));
        let choices = self
            .ui
            .settings
            .appearance
            .choices
            .as_deref()
            .unwrap_or_default();
        for (n, choice) in choices.iter().enumerate() {
            let on = choice.id == selected;
            let id = choice.id.clone();
            cards = cards.child(
                div()
                    .id(ElementId::named_usize("alt-theme", n))
                    .tab_index(0)
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .rounded(crate::tokens::r(crate::tokens::R_CARD))
                    .overflow_hidden()
                    .border_1()
                    .border_color(rgb(if on {
                        palette().text
                    } else {
                        palette().divider
                    }))
                    .bg(rgb(palette().card_color))
                    .cursor_pointer()
                    .focus_visible(|s| s.shadow(crate::frame::focus_ring(palette().focus)))
                    .seen()
                    .child(preview(&choice.palette))
                    .child(
                        div()
                            .px(px(10.))
                            .py(px(8.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_size(px(tokens::LABEL))
                                            .truncate()
                                            .child(choice.name.clone()),
                                    )
                                    // Chosen is said in words and by the tick, not by colour
                                    // alone.
                                    .child(if on {
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(4.))
                                            .text_size(px(tokens::SMALL))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(blocks::icon(
                                                "check",
                                                13.,
                                                rgb(palette().text).into(),
                                            ))
                                            .child(tr("Selected"))
                                    } else {
                                        hint(choice.about.clone()).truncate()
                                    }),
                            ),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.appearance_change(|s| appearance::select_theme(s, &id), window);
                        cx.notify();
                    })),
            );
        }
        // A folder that isn't a theme is listed with why, and can't be chosen.
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
        let theme = stacked(
            tr("Theme"),
            tr("Colours and scenery for the whole launcher. Play uses white text in every theme."),
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .children(gone)
                .child(cards),
        );
        let mut rows = vec![self.panel_opacity_row(cx), theme];
        if !bad.is_empty() {
            rows.push(div().flex().flex_col().gap(px(10.)).children(bad));
        }
        rows.push(self.accent_row(missing, window, cx));
        rows.push(self.background_row(cx));
        rows.push(self.scenery_row(cx));
        let reduced = self.settings.reduced_graphics();
        rows.push(row(
            tr("Reduced graphics"),
            tr("Turns off glass blur and keeps the scenery and cat still. Also reduces motion in the launcher."),
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
        ));
        let layout = self.settings.world_layout;
        let layouts: Vec<(backend::WorldLayout, SharedString)> = [
            (backend::WorldLayout::Rows, crate::i18n::tr_key("Rows")),
            (backend::WorldLayout::Tiles, crate::i18n::tr_key("Tiles")),
        ]
        .into_iter()
        .map(|(way, label)| (way, tr(label).into()))
        .collect();
        let this = cx.entity();
        rows.push(row(
            tr("Worlds on Home"),
            tr(match layout {
                backend::WorldLayout::Rows => "Continue playing lists one world a row.",
                backend::WorldLayout::Tiles => {
                    "Continue playing shows the worlds' pictures side by side, as many as fit across; the rest wrap under them."
                }
            }),
            {
                let state = &mut self.ui.settings.appearance;
                let focus = state
                    .worlds_focus
                    .get_or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
                    .clone();
                // Opened from the search: the page scrolls here and the keyboard is on it.
                let asked = state.worlds_asked.clone().map(|done| {
                    crate::mods::reveal(
                        crate::frame::scrolls("settings"),
                        done,
                        Some(focus.clone()),
                    )
                });
                div().child(
                    crate::widgets::chip::choices("alt-world-layout", &layouts, layout, move |way, _, cx| {
                        this.update(cx, |this, cx| {
                            this.settings.world_layout = way;
                            this.save_settings();
                            cx.notify();
                        })
                    })
                    .track_focus(&focus)
                        .gap(px(6.))
                        .children(asked),
                )
            },
        ));
        rows.push(
            self.wide_row(
                tr("Your themes"),
                tr("A theme is a folder with a theme.toml: up to six colours and one picture. Reload reads the folder again."),
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
            ),
        );
        rows.push(self.corners_row(cx));
        let cat = self.settings.show_cat();
        rows.push(row(
            tr("Show the cat"),
            tr("It visits Home now and then. It stays away while motion is reduced."),
            div().child(switch("alt-cat-toggle", cat).seen().on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    this.settings.cat = Some(!cat);
                    this.save_settings();
                    cx.notify();
                },
            ))),
        ));
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(group(rows))
            .children(said)
    }

    fn panel_opacity_row(&self, cx: &mut Context<Self>) -> Div {
        let saved = self.settings.panel_opacity();
        let control = &self.ui.settings.appearance.panel_opacity;
        let actual = (palette().alpha * 100.).round() as u8;
        let chosen = control.preview().map(opacity_percent).or(saved);
        let percent = chosen.unwrap_or(actual);
        let thumb = (f32::from(percent) - f32::from(PANEL_OPACITY_MIN)) / OPACITY_SPAN;
        let slider = control.render(
            "panel-opacity",
            thumb.clamp(0., 1.),
            1. / OPACITY_SPAN,
            |this, value, cx| {
                this.settings
                    .set_panel_opacity(Some(opacity_percent(value)));
                this.save_settings();
                cx.notify();
            },
            cx,
        );
        let reset = secondary("panel-opacity-reset", tr("Use default"));
        let reset = if saved.is_none() {
            off(reset)
        } else {
            reset.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.settings.set_panel_opacity(None);
                this.save_settings();
                cx.notify();
            }))
        };
        let about = if !self.settings.scenery() {
            tr("Turn on scenery to see how the backdrop shows through panels.")
        } else {
            tr("How much of the backdrop shows through cards and panels.")
        };
        self.wide_row(
            tr("Panel opacity"),
            about,
            div()
                .child(slider)
                .child(
                    div()
                        .min_w(px(44.))
                        .text_size(px(tokens::LABEL))
                        .text_center()
                        .child(format!("{percent}%")),
                )
                .child(reset),
        )
    }

    /// The accent: the theme's own colour and the named ones to pick from, and one field that
    /// takes any name, hue or `#rrggbb`. What is typed is taken on Enter or when the field is
    /// left; Escape leaves the accent as it was.
    fn accent_row(&mut self, missing: bool, window: &Window, cx: &mut Context<Self>) -> Div {
        let state = &mut self.ui.settings.appearance;
        let focus = state
            .accent_focus
            .get_or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone();
        let focused = focus.is_focused(window);
        let left = std::mem::replace(&mut state.accent_editing, focused) && !focused;
        let shown = appearance::accent(&self.settings);
        let typed = state.accent.text().trim().to_string();
        if left && !missing && !typed.is_empty() && typed != shown.name() {
            self.appearance_change(|s| appearance::type_accent(s, &typed), window);
            // This frame has started in the old colours.
            cx.notify();
        }
        let shown = appearance::accent(&self.settings);
        let state = &mut self.ui.settings.appearance;
        if !focused {
            state.accent.set(shown.name());
        }
        let own = themes::own_accent(self.settings.theme.as_deref());
        let edit: fn(&mut Self) -> &mut line_edit::LineEdit =
            |this| &mut this.ui.settings.appearance.accent;
        let field_theme = crate::mods::field_theme(self.theme);
        let to_focus = focus.clone();
        let field = crate::widgets::field::field_box(focused, false)
            .id("alt-accent")
            .track_focus(&focus)
            .seen()
            .w(px(150.))
            .h(px(tokens::FIELD))
            .px(px(12.))
            .font_family(theme::MONO)
            .text_size(px(tokens::LABEL))
            .text_color(rgb(palette().text))
            .cursor_text()
            .child(line_edit::content(
                &self.ui.settings.appearance.accent,
                &field_theme,
                focused,
                false,
                false,
                "teal, 176 or #2563eb",
            ))
            .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
                window.focus(&to_focus, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
                match line_edit::handle(edit(this), ev, cx) {
                    line_edit::Key::Pass => return,
                    // Enter takes what is typed; Escape leaves the accent as it was.
                    line_edit::Key::Enter => {
                        let typed = edit(this).text().to_string();
                        this.appearance_change(|s| appearance::type_accent(s, &typed), window);
                    }
                    line_edit::Key::Escape => {
                        let shown = appearance::accent(&this.settings).name();
                        edit(this).set(shown);
                        window.blur(cx);
                    }
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        let field = line_edit::mouse(field, cx, edit);
        // The theme's own colour first, then each named accent the theme doesn't already wear.
        let picks = std::iter::once((own, true)).chain(
            Accent::ALL
                .into_iter()
                .filter(|a| *a != own)
                .map(|a| (a, false)),
        );
        let mut swatches = div().flex().flex_wrap().items_center().gap(px(8.));
        for (i, (accent, theirs)) in picks.enumerate() {
            let on = accent == shown;
            let tip = if theirs {
                tr("Theme accent")
            } else {
                Accent::ALL
                    .iter()
                    .position(|a| *a == accent)
                    .map_or("", |i| tr(ACCENT_NAMES[i]))
            };
            let swatch = div()
                .id(("alt-accent-swatch", i))
                .flex_none()
                .size(px(SWATCH))
                .rounded_full()
                .p(px(3.))
                .border_2()
                .border_color(if on {
                    rgb(palette().text).into()
                } else {
                    gpui::transparent_black()
                })
                .reach_edge()
                .tooltip(crate::glass_ui::tip(tip))
                .child(div().size_full().rounded_full().bg(rgb(accent.rgb())));
            swatches = swatches.child(if missing || on {
                swatch.cursor_default()
            } else {
                swatch
                    .cursor_pointer()
                    .hover(|s| s.border_color(rgb(palette().muted)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        if theirs {
                            this.appearance_change(appearance::reset_accent, window);
                        } else {
                            this.appearance_change(|s| appearance::set_accent(s, accent), window);
                        }
                        cx.notify();
                    }))
            });
        }
        stacked(
            tr("Accent"),
            tr("Pick a colour, or type a colour name, a hue in degrees or #rrggbb. Play darkens a pale accent just enough for its white label."),
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(16.))
                .child(swatches.when(missing, |d| d.opacity(0.4)))
                .child(if missing {
                    div().opacity(0.4).child(field)
                } else {
                    div().child(field)
                }),
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
            Behind::Plain => tr("Off: the theme's plain background."),
            Behind::Personal(_) => tr("Your picture, behind every page."),
            Behind::Theme(_) => tr("The theme's picture, behind every page."),
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
        row(tr("Show background"), "", div().child(show))
    }
}
