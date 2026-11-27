//! Drawing the page: the title and the preview on the left, with the player model and Save under
//! it; the account's skins and capes on the right, as tiles. The preview turns by dragging.
use super::*;
use crate::widgets::button::{primary, secondary};
use crate::widgets::chip::choices;
use crate::widgets::feedback::{banner, empty_state, progress, Tone};
use crate::widgets::focus::c;
use sunrise_launcher::account::capes::Cape;

/// The left column: the preview's card, wide enough for the card's padding.
const SIDE_W: f32 = PREVIEW_W + 32.;

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
            .overflow_y_scroll();
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
                let columns = self.skins_unsigned(account, offline, window, cx);
                return self.turned_by_dragging(page.child(columns), cx);
            }
        };
        let worn = self.worn_look(&account);
        let (skin, slim, cape) = self.skins.chosen(&worn);
        let preview = self.look_preview(&account.uuid, (&skin, slim, cape.as_ref()), window, cx);
        let under = self.look_actions(slim, cx);
        let choices = div()
            .flex_1()
            .min_w(px(320.))
            .flex()
            .flex_col()
            .gap(px(28.))
            .child(self.skin_choices(&account, &worn, &skin, slim, cx))
            .child(self.cape_choices(cape.as_ref(), cx));
        self.turned_by_dragging(
            page.child(skins_columns(
                skins_side(Some(preview), Some(under)),
                choices,
            )),
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

    /// The page with no choice to make: the game's look in the preview, and either why the
    /// skin can't be changed here (the offline developer view) or the sign-in (signed out).
    fn skins_unsigned(
        &mut self,
        account: Option<account::Account>,
        offline: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        // In the offline developer view, the game's offline look: the account's default skin
        // (Steve's with no account). Nothing the page knows from online is shown.
        let (uuid, skin, slim) = match account.as_ref().filter(|_| offline) {
            Some(a) => (
                a.uuid.clone(),
                SkinPick::AccountDefault,
                self.default_slim(&a.uuid),
            ),
            None => (String::new(), SkinPick::Default("steve"), false),
        };
        // With no game prepared there is no picture to show.
        let preview = self
            .skin_resources()
            .is_some()
            .then(|| self.look_preview(&uuid, (&skin, slim, None), window, cx));
        let beside = if offline {
            crate::settings::hint(tr(
                "Offline developer session: skins and capes can't be changed here.",
            ))
        } else {
            empty_state(
                tr("Sign in to choose your skin"),
                tr(
                    "Sign in with the Microsoft account that owns Minecraft: Java Edition to choose its skin and cape.",
                ),
                Some(
                    secondary("skins-sign-in", tr("Sign in with Microsoft"))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.sign_in_to_play();
                            cx.notify();
                        }))
                        .into_any_element(),
                ),
            )
        };
        skins_columns(
            skins_side(preview, None),
            div().flex_1().min_w(px(320.)).child(beside),
        )
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

    /// The preview's picture of the look at the window's scale, drawn again only when the look or
    /// the turn changes (the one drawn last is kept, and the old one dropped).
    fn preview_image(
        &mut self,
        skin_key: &str,
        skin: &Texture,
        slim: bool,
        cape: Option<&(String, Rc<Texture>)>,
        window: &mut Window,
    ) -> Arc<RenderImage> {
        let (w, h) = (
            (PREVIEW_W * self.scale).round() as usize,
            (PREVIEW_H * self.scale).round() as usize,
        );
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            (
                skin_key,
                slim,
                cape.map(|c| &c.0),
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
                        skin,
                        slim,
                        cape: cape.map(|c| c.1.as_ref()),
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
    }

    /// The player in the chosen look, turned by dragging, in its card. `look` is the skin, its
    /// arms and the cape, on account `uuid`'s page.
    pub(super) fn look_preview(
        &mut self,
        uuid: &str,
        look: (&SkinPick, bool, Option<&CapePick>),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let (skin, slim, cape) = look;
        let texture = self.skin_texture(uuid, skin, slim);
        let cape = cape.and_then(|cape| self.cape_texture(cape));
        let image = texture.map(|(skin_key, skin)| {
            self.preview_image(&skin_key, &skin, slim, cape.as_ref(), window)
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
        crate::home::card()
            .flex_none()
            .w(px(SIDE_W))
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(stage)
            .child(
                div()
                    .text_center()
                    .child(crate::settings::hint(tr("Drag to turn"))),
            )
    }

    /// What goes under the preview: the player model, Save and Undo, the progress of a Save or of
    /// saved skins being written, and what the last Save said.
    pub(super) fn look_actions(&mut self, slim: bool, cx: &mut Context<Self>) -> Div {
        let p = palette();
        let changed = !self.skins.edit.is_empty();
        // A Save also waits while what an earlier one confirmed is being recorded.
        let saving = self.skins.saving.is_some() || self.skins.recording > 0;
        let this = cx.entity();
        let arms: [(bool, SharedString); 2] =
            [(false, tr("Wide").into()), (true, tr("Slim").into())];
        let model = div()
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
            // The arms the skin is drawn for: wide (4 pixels, like Steve) or slim (3, like Alex),
            // named as the official launcher names them. Only an edit: the saved skin takes these
            // arms once it is worn with them.
            .child(choices("skins-model", &arms, slim, move |wanted, _, cx| {
                this.update(cx, |this, cx| {
                    this.choose(|edit| edit.slim = Some(wanted));
                    cx.notify();
                });
            }));
        let save = primary(
            "skins-save",
            if saving { tr("Saving…") } else { tr("Save") },
        )
        .flex_1()
        .when(!changed || saving, |d| d.opacity(0.5).cursor_default())
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.look_save();
            cx.notify();
        }));
        let actions = div()
            .flex()
            .gap(px(8.))
            .child(save)
            .when(changed && !saving, |d| {
                d.child(secondary("skins-undo", tr("Undo")).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.skins.edit = Edit::default();
                        this.skins.said.clear();
                        cx.notify();
                    },
                )))
            });
        let saving_line = saving.then(|| progress(tr("Saving to your account…"), None, None));
        let writing_line = (self.skins.disk_writes > 0)
            .then(|| progress(tr("Updating your saved skins…"), None, None));
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(model)
            .children(saving_line)
            .children(writing_line)
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

    /// The Skin section's heading and hint, with the Upload button beside them.
    fn skin_heading(cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_wrap()
            .items_end()
            .justify_between()
            .gap(px(12.))
            .child(Self::section(
                crate::i18n::tr_key("Skin"),
                crate::i18n::tr_key(
                    "Add a skin from a PNG file, or drop one here. Saved skins stay on this computer.",
                ),
            ))
            .child(
                primary("skin-add", tr("Upload skin")).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.skins_pick_file(cx);
                    },
                )),
            )
    }

    /// One choice: a picture and its name (none for an empty one). The chosen one has the
    /// selected fill and an accent edge; a row's tiles stretch to its tallest, so a two-line name
    /// keeps them lined up.
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
            .p(px(8.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(6.))
            .rounded(crate::tokens::r(crate::tokens::R_CARD))
            .border_1()
            .cursor_pointer()
            .when(chosen, |d| {
                d.bg(rgb(p.selected)).border_color(rgb(p.accent_ink))
            })
            .when(!chosen, |d| {
                d.bg(p.card())
                    .border_color(p.rule())
                    .hover(|s| s.bg(rgb(p.hover)))
            })
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
                .bg(rgb(palette().field))
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

    /// Chooses the skin picked, with arms `slim` when given.
    fn pick_skin(&mut self, skin: SkinPick, slim: Option<bool>) {
        self.choose(|edit| {
            edit.skin = Some(skin);
            if let Some(slim) = slim {
                edit.slim = Some(slim);
            }
        });
    }

    pub(super) fn skin_choices(
        &mut self,
        account: &account::Account,
        worn: &Worn,
        chosen: &SkinPick,
        chosen_slim: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let uuid = account.uuid.as_str();
        let mut tiles = div().flex().flex_wrap().gap(px(8.));
        if let Some(tile) = self.worn_tile(uuid, worn, chosen, cx) {
            tiles = tiles.child(tile);
        }
        let mut history = self.skins.saved.clone();
        history.reverse();
        history.sort_by_key(|saved| SkinPick::Saved(saved.id.clone()) != worn.skin);
        for saved in &history {
            tiles = tiles.child(self.saved_tile(uuid, saved, worn, chosen, cx));
        }
        let mut section = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(Self::skin_heading(cx))
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
        let defaults = self
            .skin_resources_found()
            .map(|(_, defaults)| defaults)
            .unwrap_or_default();
        if !defaults.is_empty() {
            section = section
                .child(Self::section(crate::i18n::tr_key("Default skins"), ""))
                .child(self.default_tiles(uuid, defaults, chosen, chosen_slim, cx));
        }
        section
    }

    /// The skin worn, while no saved skin keeps it: the picture it is now, so a click on it
    /// during a Save stays that picture whatever the Save leaves worn.
    fn worn_tile(
        &self,
        uuid: &str,
        worn: &Worn,
        chosen: &SkinPick,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        if !matches!(worn.skin, SkinPick::Texture(_) | SkinPick::AccountDefault) {
            return None;
        }
        let skin = worn.skin.clone();
        let (slim, on) = (worn.slim, *chosen == skin);
        Some(
            self.tile(
                "skin-worn".into(),
                self.skin_thumb(uuid, &skin, slim),
                tr("Your skin").into(),
                on,
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.pick_skin(skin.clone(), Some(slim));
                cx.notify();
            })),
        )
    }

    /// A saved skin's tile. The skin worn is shown with the arms the profile wears it with, not
    /// the ones saved for it; it can't be forgotten, it would be kept again at once.
    fn saved_tile(
        &self,
        uuid: &str,
        saved: &Saved,
        worn: &Worn,
        chosen: &SkinPick,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let skin = SkinPick::Saved(saved.id.clone());
        let is_worn = skin == worn.skin;
        let slim = if is_worn { worn.slim } else { saved.slim };
        let group = SharedString::from(format!("skin-{}", saved.id));
        let forget = Self::forget_button(&saved.id, group.clone(), cx);
        self.tile(
            group,
            self.skin_thumb(uuid, &skin, slim),
            saved.name.clone().into(),
            *chosen == skin,
        )
        .when(!is_worn, |d| d.child(forget))
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.pick_skin(skin.clone(), Some(slim));
            cx.notify();
        }))
    }

    /// The button that forgets saved skin `id`: shown on its tile's hover, and on keyboard focus,
    /// where it is a Tab stop of its own.
    fn forget_button(id: &str, group: SharedString, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = palette();
        let target = id.to_owned();
        div()
            .id(SharedString::from(format!("skin-forget-{id}")))
            .reach()
            .absolute()
            .top(px(4.))
            .right(px(4.))
            .size(px(22.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(crate::tokens::r(crate::tokens::R_SMALL))
            .bg(rgb(p.field))
            .opacity(0.)
            .group_hover(group, |s| s.opacity(1.))
            .focus_visible(|s| s.opacity(1.))
            .hover(|s| s.bg(rgb(p.hover)))
            .child(crate::glass_ui::icon("close", 12., c(p.muted)))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.skin_forget(&target);
                cx.notify();
            }))
    }

    /// The game's default skins, each a tile.
    fn default_tiles(
        &self,
        uuid: &str,
        names: Vec<&'static str>,
        chosen: &SkinPick,
        chosen_slim: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut grid = div().flex().flex_wrap().gap(px(8.));
        for name in names {
            let skin = SkinPick::Default(name);
            let title: String = name[..1].to_uppercase() + &name[1..];
            let on = *chosen == skin;
            grid = grid.child(
                self.tile(
                    SharedString::from(format!("skin-default-{name}")),
                    self.skin_thumb(uuid, &skin, chosen_slim),
                    title.into(),
                    on,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.pick_skin(skin.clone(), None);
                    cx.notify();
                })),
            );
        }
        grid
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
        let (list, problem) = self.cape_listing(cx);
        // No cape is a choice even while the account's capes can't be listed.
        let mut choices: Vec<(CapePick, String)> = vec![(CapePick::None, tr("No cape").into())];
        choices.extend(
            list.iter()
                .map(|cape| (CapePick::Minecraft(cape.id.clone()), cape.alias.clone())),
        );
        let mut tiles = div().flex().flex_wrap().gap(px(8.));
        for (cape, label) in choices {
            let picture = self.cape_picture(&cape);
            let id = SharedString::from(match &cape {
                CapePick::None => "cape-none".to_string(),
                CapePick::Minecraft(id) => format!("cape-{id}"),
            });
            let on = chosen == Some(&cape);
            tiles = tiles.child(
                self.tile(id, picture, label.into(), on)
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

    /// The capes the account owns, and what keeps them from being listed, if anything does.
    fn cape_listing(&self, cx: &mut Context<Self>) -> (Vec<Cape>, Option<Div>) {
        match &self.skins.profile {
            None => (
                Vec::new(),
                Some(crate::settings::hint(tr("Looking up your capes…"))),
            ),
            Some(Err(_)) if self.skins.reading.is_some() => (
                Vec::new(),
                Some(crate::settings::hint(tr("Looking up your capes…"))),
            ),
            Some(Err(why)) => (Vec::new(), Some(self.cape_problem(why, cx))),
            Some(Ok(profile)) => (profile.capes.clone(), None),
        }
    }

    /// Why the capes can't be listed, with the action that helps: sign in again, or try again.
    fn cape_problem(&self, why: &str, cx: &mut Context<Self>) -> Div {
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
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(12.))
            .child(banner(Tone::Danger, why.to_owned(), None))
            .child(action)
    }

    /// A cape's picture: its front, or the close mark for no cape.
    fn cape_picture(&self, cape: &CapePick) -> AnyElement {
        let face = self.cape_texture(cape).map(|(key, texture)| {
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
        let none = *cape == CapePick::None;
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .rounded(crate::tokens::r(crate::tokens::R_SMALL))
            .bg(rgb(palette().field))
            .map(|d| match face {
                Some(face) => d.child(face),
                None if none => d.child(crate::glass_ui::icon("close", 22., c(palette().muted))),
                None => d,
            })
            .into_any_element()
    }
}

/// The left column: the title, then the preview and what goes under it.
fn skins_side(preview: Option<Div>, under: Option<Div>) -> Div {
    div()
        .flex_none()
        .w(px(SIDE_W))
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(
            div()
                .text_size(px(tokens::TITLE))
                .font_weight(FontWeight::BOLD)
                .child(tr("Skins")),
        )
        .children(preview)
        .children(under)
}

/// The page's two columns: the side (title, preview) and the choices, wrapping on a narrow window.
fn skins_columns(side: Div, choices: Div) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_start()
        .gap(px(32.))
        .child(side)
        .child(choices)
}
