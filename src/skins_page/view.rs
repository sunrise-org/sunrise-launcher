//! Drawing the page: the drag-to-turn preview, the tiles, and the skin and cape choices.
use super::*;
use crate::widgets::button::{primary, secondary};
use crate::widgets::feedback::{banner, Tone};

impl Launcher {
    pub(crate) fn skins_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let page = self.skins_view(window, cx);
        // The files the page asked for while it was drawn.
        self.skins_read_files();
        crate::scrollbar::wrap_page("skins-page", page, &self.theme)
    }

    pub(super) fn skins_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let page = div()
            .id("skins-page")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .p(px(24.))
            .gap(px(20.))
            .overflow_y_scroll()
            .child(
                div()
                    .text_size(px(tokens::TITLE))
                    .font_weight(FontWeight::BOLD)
                    .child(tr("Skins")),
            );
        let offline = self.skins_offline();
        let account = self.look_account();
        // Another account plays now (switched in the account menu while the page is open), or
        // another profile turned the offline developer view on or off.
        let other_account = account
            .as_ref()
            .is_some_and(|a| self.skins.uuid.as_deref() != Some(&a.uuid));
        if self.skins.opened_offline != Some(offline) || other_account {
            self.skins_open();
        }
        let account = match account {
            Some(account) if !offline => account,
            account => {
                // A player of the page's own, from the game files alone: in the offline developer
                // view the game's offline look (the account's default skin, Steve's with no
                // account), and Steve signed out. Nothing the page knows from online (a profile,
                // edits) is shown. With no game prepared there is no picture to show.
                let (uuid, skin, slim) = match account.as_ref().filter(|_| offline) {
                    Some(a) => (
                        a.uuid.clone(),
                        SkinPick::AccountDefault,
                        self.default_slim(&a.uuid),
                    ),
                    None => (String::new(), SkinPick::Default("steve"), false),
                };
                let preview = self
                    .skin_resources()
                    .is_some()
                    .then(|| self.look_preview(&uuid, (&skin, slim, None), true, window, cx));
                let beside = if offline {
                    crate::settings::hint(tr(
                        "Offline developer session: skins and capes can't be changed here.",
                    ))
                } else {
                    crate::home::card()
                    .flex_1()
                    .min_w(px(280.))
                    .p(px(20.))
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(12.))
                    .child(crate::settings::hint(tr(
                        "Sign in with the Microsoft account that owns Minecraft: Java Edition to choose its skin and cape.",
                    )))
                    .child(secondary("skins-sign-in", tr("Sign in with Microsoft")).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.sign_in_to_play();
                            cx.notify();
                        }),
                    ))
                };
                return self.turned_by_dragging(
                    page.child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_start()
                            .gap(px(24.))
                            .children(preview)
                            .child(beside),
                    ),
                    cx,
                );
            }
        };
        let worn = self.worn_look(&account);
        let (skin, slim, cape) = self.skins.chosen(&worn);
        let preview = self.look_preview(
            &account.uuid,
            (&skin, slim, cape.as_ref()),
            false,
            window,
            cx,
        );
        let choices = div()
            .flex_1()
            .min_w(px(280.))
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(self.skin_choices(&account, &worn, &skin, slim, cx))
            .child(self.cape_choices(cape.as_ref(), cx));
        self.turned_by_dragging(
            page.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_start()
                    .gap(px(24.))
                    .child(preview)
                    .child(choices),
            ),
            cx,
        )
        // A skin file dropped anywhere on the page is saved and chosen.
        .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(rgb(palette().hover)))
        .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
            this.skins_add(paths.paths().to_vec());
            cx.stop_propagation();
            cx.notify();
        }))
    }

    /// A drag begun on the preview turns it wherever the pointer goes on `page`.
    pub(super) fn turned_by_dragging(
        &self,
        page: Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        page.on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
            let Some((from, yaw, pitch)) = this.skins.drag else {
                return;
            };
            if ev.pressed_button != Some(MouseButton::Left) {
                this.skins.drag = None;
                cx.notify();
                return;
            }
            let (dx, dy) = (
                f32::from(ev.position.x - from.x),
                f32::from(ev.position.y - from.y),
            );
            this.skins.yaw = yaw + dx * 0.012;
            this.skins.pitch = (pitch + dy * 0.008).clamp(-0.5, 0.6);
            cx.notify();
        }))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                if this.skins.drag.take().is_some() {
                    cx.notify();
                }
            }),
        )
    }

    /// The player in the chosen look, turned by dragging, with the arm model and Save under it.
    /// `look` is the skin, its arms and the cape, on account `uuid`'s page; `read_only` shows
    /// the preview alone, without the arm model or Save.
    pub(super) fn look_preview(
        &mut self,
        uuid: &str,
        look: (&SkinPick, bool, Option<&CapePick>),
        read_only: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let (skin, slim, cape) = look;
        let p = palette();
        let texture = self.skin_texture(uuid, skin, slim);
        let cape = cape.and_then(|cape| self.cape_texture(cape));
        let image = texture.map(|(skin_key, skin)| {
            let (w, h) = (
                (PREVIEW_W * self.scale).round() as usize,
                (PREVIEW_H * self.scale).round() as usize,
            );
            let key = {
                use std::hash::{Hash, Hasher};
                let mut hash = std::collections::hash_map::DefaultHasher::new();
                (
                    &skin_key,
                    slim,
                    cape.as_ref().map(|c| &c.0),
                    self.skins.yaw.to_bits(),
                    self.skins.pitch.to_bits(),
                    w,
                    h,
                )
                    .hash(&mut hash);
                hash.finish()
            };
            match &self.skins.shown {
                Some((shown, image)) if *shown == key => image.clone(),
                _ => {
                    let texels = player_model::render(
                        &Pose {
                            skin: &skin,
                            slim,
                            cape: cape.as_ref().map(|c| c.1.as_ref()),
                            yaw: self.skins.yaw,
                            pitch: self.skins.pitch,
                        },
                        w,
                        h,
                    );
                    let image = art::bake(&texels, w, h, 1. / self.scale, self.scale);
                    if let Some((_, old)) = self.skins.shown.replace((key, image.clone())) {
                        let _ = window.drop_image(old);
                    }
                    image
                }
            }
        });
        let stage = div()
            .id("skin-preview")
            .w(px(PREVIEW_W))
            .h(px(PREVIEW_H))
            .flex()
            .items_center()
            .justify_center()
            .cursor(if self.skins.drag.is_some() {
                gpui::CursorStyle::ClosedHand
            } else {
                gpui::CursorStyle::OpenHand
            })
            .map(|d| match image {
                Ok(image) => d.child(img(image).w(px(PREVIEW_W)).h(px(PREVIEW_H))),
                Err(true) => d.child(crate::settings::hint(tr("Loading your skin…"))),
                Err(false) => d.child(crate::settings::hint(tr(
                    "This skin's picture couldn't be loaded.",
                ))),
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    this.skins.drag = Some((ev.position, this.skins.yaw, this.skins.pitch));
                    cx.notify();
                }),
            );
        let card = crate::home::card()
            .flex_none()
            .w(px(PREVIEW_W + 32.))
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(stage)
            .child(
                div()
                    .text_center()
                    .child(crate::settings::hint(tr("Drag to turn"))),
            );
        if read_only {
            return card;
        }
        let arms = |wanted: bool, label: &'static str| {
            let on = slim == wanted;
            div()
                .id(label)
                .reach()
                .flex_1()
                .h(px(32.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                .text_size(px(tokens::SMALL))
                .font_weight(FontWeight::SEMIBOLD)
                .cursor_pointer()
                .when(on, |d| d.bg(rgb(p.selected)))
                .when(!on, |d| {
                    d.text_color(rgb(p.muted)).hover(|s| s.bg(rgb(p.hover)))
                })
                .child(tr(label))
                // Only an edit: the saved skin takes these arms once it is worn with them.
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.choose(|edit| edit.slim = Some(wanted));
                    cx.notify();
                }))
        };
        let changed = !self.skins.edit.is_empty();
        // A Save also waits while what an earlier one confirmed is being recorded.
        let saving = self.skins.saving.is_some() || self.skins.recording > 0;
        let actions = div()
            .flex()
            .gap(px(8.))
            .child(
                primary(
                    "skins-save",
                    if saving { tr("Saving…") } else { tr("Save") },
                )
                .flex_1()
                .when(!changed || saving, |d| d.opacity(0.5).cursor_default())
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.look_save();
                    cx.notify();
                })),
            )
            .when(changed && !saving, |d| {
                d.child(secondary("skins-undo", tr("Undo")).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.skins.edit = Edit::default();
                        this.skins.said.clear();
                        cx.notify();
                    },
                )))
            });
        card
            // The arms the skin is drawn for: wide (4 pixels, like Steve) or slim (3, like Alex),
            // named as the official launcher names them.
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(tokens::SMALL))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(p.muted))
                            .child(tr("Player model")),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(4.))
                            .p(px(3.))
                            .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                            .bg(rgb(p.field))
                            .child(arms(false, crate::i18n::tr_key("Wide")))
                            .child(arms(true, crate::i18n::tr_key("Slim"))),
                    ),
            )
            .child(actions)
            .children(self.skins.said.iter().map(|line| match line.clone() {
                Ok(done) => crate::settings::hint(done),
                Err(why) => banner(Tone::Danger, why, None),
            }))
    }

    pub(super) fn section(title: &str, about: &str) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(crate::home::heading(tr(title)))
            .when(!about.is_empty(), |d| {
                d.child(crate::settings::hint(tr(about)))
            })
    }

    /// One choice: a picture and its name (none for an empty one), the chosen one edged in the
    /// accent. A row's tiles stretch to its tallest, so a two-line name keeps them lined up.
    pub(super) fn tile(
        &self,
        id: SharedString,
        picture: AnyElement,
        label: SharedString,
        chosen: bool,
    ) -> Stateful<Div> {
        let p = palette();
        div()
            .id(id.clone())
            .group(id)
            .reach_edge()
            .relative()
            .w(px(TILE_W))
            .p(px(10.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(6.))
            .rounded(crate::tokens::r(crate::tokens::R_CARD))
            .border_1()
            .border_color(rgb(if chosen { p.accent } else { p.divider }))
            .bg(palette().card())
            .cursor_pointer()
            .when(!chosen, |d| d.hover(|s| s.bg(rgb(p.hover))))
            .child(
                div()
                    .w(px(PICTURE_W))
                    .h(px(PICTURE_H))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(picture),
            )
            .when(!label.is_empty(), |d| {
                d.child(
                    div()
                        .w_full()
                        .text_center()
                        .text_size(px(tokens::SMALL))
                        .font_weight(FontWeight::SEMIBOLD)
                        .line_height(px(17.))
                        .overflow_hidden()
                        .line_clamp(2)
                        .child(label),
                )
            })
    }

    /// A small picture of the model in `skin`, for its tile.
    pub(super) fn skin_thumb(&self, uuid: &str, skin: &SkinPick, slim: bool) -> AnyElement {
        let Ok((key, texture)) = self.skin_texture(uuid, skin, slim) else {
            return div()
                .size_full()
                .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                .bg(self.theme.well)
                .into_any_element();
        };
        let (w, h) = (
            (PICTURE_W * self.scale).round() as usize,
            (PICTURE_H * self.scale).round() as usize,
        );
        let image = self
            .skins
            .baked
            .borrow_mut()
            .entry((format!("{key}:thumb:{slim}"), self.scale.to_bits()))
            .or_insert_with(|| {
                let texels = player_model::render(
                    &Pose {
                        skin: &texture,
                        slim,
                        cape: None,
                        yaw: 0.45,
                        pitch: 0.1,
                    },
                    w,
                    h,
                );
                art::bake(&texels, w, h, 1. / self.scale, self.scale)
            })
            .clone();
        img(image)
            .w(px(PICTURE_W))
            .h(px(PICTURE_H))
            .into_any_element()
    }

    pub(super) fn skin_choices(
        &mut self,
        account: &account::Account,
        worn: &Worn,
        chosen: &SkinPick,
        chosen_slim: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut tiles = div().flex().flex_wrap().gap(px(10.));
        let pick = |this: &mut Launcher, skin: SkinPick, slim: Option<bool>| {
            this.choose(|edit| {
                edit.skin = Some(skin);
                if let Some(slim) = slim {
                    edit.slim = Some(slim);
                }
            });
        };
        let worn_slim = worn.slim;
        let uuid = account.uuid.as_str();
        // The skin worn, while no saved skin keeps it: the picture it is now, so a click on it
        // during a Save stays that picture whatever the Save leaves worn.
        if matches!(worn.skin, SkinPick::Texture(_) | SkinPick::AccountDefault) {
            let skin = worn.skin.clone();
            tiles = tiles.child(
                self.tile(
                    "skin-worn".into(),
                    self.skin_thumb(uuid, &skin, worn_slim),
                    tr("Your skin").into(),
                    *chosen == skin,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    pick(this, skin.clone(), Some(worn_slim));
                    cx.notify();
                })),
            );
        }
        let mut history = self.skins.saved.clone();
        history.reverse();
        history.sort_by_key(|saved| SkinPick::Saved(saved.id.clone()) != worn.skin);
        for saved in history {
            let skin = SkinPick::Saved(saved.id.clone());
            let is_worn = skin == worn.skin;
            // The worn skin with the arms the profile wears it with, not the ones saved for it.
            let slim = if is_worn { worn_slim } else { saved.slim };
            let id = saved.id.clone();
            let group = SharedString::from(format!("skin-{}", saved.id));
            // Shown on hover, and on keyboard focus, where it is a Tab stop of its own.
            let forget = div()
                .id(SharedString::from(format!("skin-forget-{}", saved.id)))
                .reach()
                .absolute()
                .top(px(4.))
                .right(px(4.))
                .size(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                .bg(rgb(palette().field))
                .opacity(0.)
                .group_hover(group.clone(), |s| s.opacity(1.))
                .focus_visible(|s| s.opacity(1.))
                .hover(|s| s.bg(rgb(palette().hover)))
                .child(blocks::icon("close", 12., self.theme.muted))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.skin_forget(&id);
                    cx.notify();
                }));
            tiles = tiles.child(
                self.tile(
                    group,
                    self.skin_thumb(uuid, &skin, slim),
                    saved.name.clone().into(),
                    *chosen == skin,
                )
                // The worn skin can't be forgotten: it would be kept again at once.
                .when(!is_worn, |d| d.child(forget))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    pick(this, skin.clone(), Some(slim));
                    cx.notify();
                })),
            );
        }
        let p = palette();
        tiles = tiles.child(
            div()
                .id("skin-add")
                .reach_edge()
                .w(px(TILE_W))
                // As tall as a tile's picture, or its row's tallest tile.
                .min_h(px(PICTURE_H + 20.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .rounded(crate::tokens::r(crate::tokens::R_CARD))
                .border_1()
                .border_dashed()
                .border_color(p.rule())
                .cursor_pointer()
                .hover(|s| s.bg(rgb(p.hover)))
                .child(blocks::icon("plus", 20., self.theme.muted))
                .child(
                    div()
                        .text_size(px(tokens::SMALL))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(tr("Add skin")),
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.skins_pick_file(cx);
                })),
        );
        let defaults = self
            .skin_resources_found()
            .map(|(_, defaults)| defaults)
            .unwrap_or_default();
        let mut section = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(Self::section(
                crate::i18n::tr_key("Skin"),
                crate::i18n::tr_key("Add a skin from a PNG file, or drop one here. Saved skins stay on this computer."),
            ))
            .child(tiles)
            // Files that couldn't be added, beside the one that was (chosen, with its tile).
            .children(
                self.skins
                    .added
                    .iter()
                    .map(|why| banner(Tone::Danger, why.clone(), None)),
            )
            .when_some(self.skins.library_error.clone(), |d, why| {
                d.child(banner(Tone::Danger, why, None))
            })
            .when_some(self.skins.keep_error.clone(), |d, why| {
                d.child(banner(Tone::Danger, why, None))
            });
        if !defaults.is_empty() {
            let mut grid = div().flex().flex_wrap().gap(px(10.));
            for name in defaults {
                let skin = SkinPick::Default(name);
                let title: String = name[..1].to_uppercase() + &name[1..];
                grid = grid.child(
                    self.tile(
                        SharedString::from(format!("skin-default-{name}")),
                        self.skin_thumb(uuid, &skin, chosen_slim),
                        title.into(),
                        *chosen == skin,
                    )
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            pick(this, skin.clone(), None);
                            cx.notify();
                        },
                    )),
                );
            }
            section = section
                .child(Self::section(crate::i18n::tr_key("Default skins"), ""))
                .child(grid);
        }
        section
    }

    pub(super) fn cape_choices(
        &mut self,
        chosen: Option<&CapePick>,
        cx: &mut Context<Self>,
    ) -> Div {
        let section = div().flex().flex_col().gap(px(12.)).child(Self::section(
            crate::i18n::tr_key("Cape"),
            crate::i18n::tr_key("The capes your account owns. Everyone sees the one you wear, in every launcher and on every server."),
        ));
        // No cape is a choice even while the account's capes can't be listed.
        let (list, problem) = match &self.skins.profile {
            None => (
                Vec::new(),
                Some(crate::settings::hint(tr("Looking up your capes…"))),
            ),
            Some(Err(_)) if self.skins.reading.is_some() => (
                Vec::new(),
                Some(crate::settings::hint(tr("Looking up your capes…"))),
            ),
            Some(Err(why)) => {
                let needs_sign_in = self.look_account().is_some_and(|a| {
                    self.auth_status
                        .get(&a.uuid)
                        .is_some_and(|s| s.state == crate::account::session::State::SignInRequired)
                });
                let action = if needs_sign_in {
                    secondary("capes-sign-in", tr("Sign in again")).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            if let Some(account) = this.look_account() {
                                this.re_sign_in(&account.uuid);
                            }
                            cx.notify();
                        },
                    ))
                } else {
                    secondary("capes-retry", tr("Try again")).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            if let Some(account) = this.look_account() {
                                this.profile_read(&account);
                            }
                            cx.notify();
                        },
                    ))
                };
                (
                    Vec::new(),
                    Some(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(12.))
                            .child(banner(Tone::Danger, why.clone(), None))
                            .child(action),
                    ),
                )
            }
            Some(Ok(profile)) => (profile.capes.clone(), None),
        };
        let mut choices: Vec<(CapePick, String)> = vec![(CapePick::None, tr("No cape").into())];
        choices.extend(
            list.iter()
                .map(|c| (CapePick::Minecraft(c.id.clone()), c.alias.clone())),
        );
        let mut tiles = div().flex().flex_wrap().gap(px(10.));
        for (cape, label) in choices {
            let face = self.cape_texture(&cape).map(|(key, texture)| {
                let front = player_model::cape_front(&texture);
                let image = self
                    .skins
                    .baked
                    .borrow_mut()
                    .entry((format!("{key}:front"), self.scale.to_bits()))
                    .or_insert_with(|| {
                        art::bake(
                            &front.texels,
                            front.w,
                            front.h,
                            PICTURE_W / front.w as f32,
                            self.scale,
                        )
                    })
                    .clone();
                img(image).w(px(PICTURE_W)).h(px(PICTURE_H))
            });
            let picture = div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                .bg(self.theme.well)
                .map(|d| match face {
                    Some(face) => d.child(face),
                    None if cape == CapePick::None => {
                        d.child(blocks::icon("close", 22., self.theme.muted))
                    }
                    None => d,
                })
                .into_any_element();
            let id = SharedString::from(match &cape {
                CapePick::None => "cape-none".to_string(),
                CapePick::Minecraft(id) => format!("cape-{id}"),
            });
            tiles = tiles.child(
                self.tile(id, picture, label.into(), chosen == Some(&cape))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let cape = cape.clone();
                        this.choose(|edit| edit.cape = Some(cape));
                        cx.notify();
                    })),
            );
        }
        section
            .child(tiles)
            .when_some(problem, |d, problem| d.child(problem))
    }
}
