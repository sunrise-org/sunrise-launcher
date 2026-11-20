//! The Skins tab: how the account that plays looks. A 3D preview of the player (drag to turn
//! it) beside the choices: the skin (the one worn now, the saved ones, the game's default
//! skins, or a new PNG), classic or slim arms, and the cape (none, one of the Minecraft capes
//! the account owns, or the OptiFine cape when it has one). Choices show on the preview at
//! once; Save sends them to the Minecraft services, which every client and server then shows.
//!
//! Choosing the OptiFine cape takes the Minecraft cape off: the OptiFine cape is the one players
//! with OptiFine see (and Sunrise players, once a cape mod shows it: sunrise-org/sunrise#3589), and
//! no Minecraft cape shows for the others.
//!
//! Every request runs on a worker and comes back as a message (`Msg::Capes`, `Picture`,
//! `OptifineCape`, `LookSaved`); one numbered for another account or replaced by a later
//! request is dropped.
use super::*;
use crate::frame::Reach as _;
use crate::paint::palette;
use gpui::{img, rgb, ExternalPaths, MouseButton, MouseDownEvent, MouseMoveEvent, RenderImage};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use sunrise_launcher::account::capes::{self, Cape};
use sunrise_launcher::player_model::{self, Pose, Texture};
use sunrise_launcher::skin_library::{self, Saved};

/// The preview's size, in points.
const PREVIEW_W: f32 = 280.;
const PREVIEW_H: f32 = 420.;
/// A tile's picture: a skin's model or a cape's face (10 by 16 texels at 6 points).
const PICTURE_W: f32 = 60.;
const PICTURE_H: f32 = 96.;
const TILE_W: f32 = 104.;
/// The game's default skins, each with classic and slim arms.
const DEFAULT_SKINS: [&str; 9] = [
    "steve", "alex", "ari", "efe", "kai", "makena", "noor", "sunny", "zuri",
];

/// Which skin is chosen.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SkinPick {
    /// The one the account wears now.
    Worn,
    Saved(String),
    Default(&'static str),
}

/// Which cape is chosen.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CapePick {
    None,
    Minecraft(String),
    Optifine,
}

/// What the player looks like, chosen or worn.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Look {
    pub skin: SkinPick,
    pub slim: bool,
    pub cape: CapePick,
}

#[derive(Default)]
pub(crate) struct Skins {
    /// The account shown.
    uuid: Option<String>,
    /// The number of the last listing or Save asked for: only its answer is kept.
    asked: u64,
    /// None while the first listing is under way.
    capes: Option<Result<Vec<Cape>, String>>,
    /// Pictures from Mojang's texture server by URL: none while asked for, or when one can't be had.
    pictures: HashMap<String, Option<Vec<u8>>>,
    /// The name the OptiFine cape was asked for, and its picture once the answer is in.
    optifine: Option<(String, Option<Option<Vec<u8>>>)>,
    saved: Vec<Saved>,
    /// The worn skin's URL and the saved skin that keeps it: every skin worn is kept, so one
    /// replaced by a Save stays in the list to wear again.
    worn_saved: Option<(String, String)>,
    /// The choices not saved yet; none: what the account wears.
    chosen: Option<Look>,
    /// A Save under way.
    saving: bool,
    /// What the last Save, or adding a skin, said: done, or why not.
    said: Option<Result<String, String>>,
    yaw: f32,
    pitch: f32,
    /// Where a drag on the preview began, and the turn then.
    drag: Option<(gpui::Point<gpui::Pixels>, f32, f32)>,
    /// Pictures read from disk (default and saved skins), by key.
    files: RefCell<HashMap<String, Option<Rc<Vec<u8>>>>>,
    /// Pictures decoded, by key.
    textures: RefCell<HashMap<String, Option<Rc<Texture>>>>,
    /// Tile pictures as the window draws them, by key and scale.
    baked: RefCell<HashMap<(String, u32), Arc<RenderImage>>>,
    /// The preview drawn last, by what it shows.
    shown: Option<(u64, Arc<RenderImage>)>,
}

impl Skins {
    fn new_for(uuid: &str) -> Skins {
        Skins {
            uuid: Some(uuid.to_owned()),
            yaw: 0.55,
            pitch: 0.12,
            ..Default::default()
        }
    }

    /// A listing answered (`Msg::Capes`).
    pub(crate) fn answered(
        &mut self,
        uuid: &str,
        asked: u64,
        result: Result<Vec<Cape>, String>,
    ) -> bool {
        if self.uuid.as_deref() != Some(uuid) || asked != self.asked {
            return false;
        }
        self.capes = Some(result);
        true
    }

    /// The pictures of `list` not yet asked for.
    fn unasked(&self, list: &[Cape]) -> Vec<String> {
        list.iter()
            .map(|c| c.url.clone())
            .filter(|url| !self.pictures.contains_key(url))
            .collect()
    }

    pub(crate) fn picture(&mut self, url: String, png: Option<Vec<u8>>) {
        self.forget(&url);
        self.pictures.insert(url, png);
    }

    pub(crate) fn optifine_cape(&mut self, name: &str, png: Option<Vec<u8>>) {
        if let Some((asked, answer)) = &mut self.optifine {
            if asked == name {
                *answer = Some(png);
            }
        }
        self.forget(&format!("optifine:{name}"));
    }

    /// Drops what was drawn from the picture `key`.
    fn forget(&mut self, key: &str) {
        self.textures.get_mut().remove(key);
        self.baked.get_mut().retain(|(k, _), _| !k.starts_with(key));
    }

    /// The OptiFine cape's picture, when the account has one.
    fn optifine_png(&self, name: &str) -> Option<&Vec<u8>> {
        match &self.optifine {
            Some((asked, Some(Some(png)))) if asked == name => Some(png),
            _ => None,
        }
    }

    /// A picture read from `path` once.
    fn file(
        &self,
        key: &str,
        path: impl FnOnce() -> Option<std::path::PathBuf>,
    ) -> Option<Rc<Vec<u8>>> {
        self.files
            .borrow_mut()
            .entry(key.to_owned())
            .or_insert_with(|| path().and_then(|p| std::fs::read(p).ok()).map(Rc::new))
            .clone()
    }

    /// The picture with `key`, decoded once by `decode`.
    fn texture(
        &self,
        key: &str,
        png: Option<&[u8]>,
        decode: fn(&[u8]) -> Option<Texture>,
    ) -> Option<Rc<Texture>> {
        if let Some(known) = self.textures.borrow().get(key) {
            return known.clone();
        }
        let png = png?;
        let texture = decode(png).map(Rc::new);
        self.textures
            .borrow_mut()
            .insert(key.to_owned(), texture.clone());
        texture
    }
}

/// A default skin's file in the prepared game files.
fn default_skin_path(resources: &std::path::Path, name: &str, slim: bool) -> std::path::PathBuf {
    resources
        .join("assets/minecraft/textures/entity/player")
        .join(if slim { "slim" } else { "wide" })
        .join(format!("{name}.png"))
}

impl Launcher {
    /// The prepared game files the default skins are read from: the version the selected profile
    /// plays when it has them, else the first installed version that does. A Nightly just
    /// downloaded has none until it is prepared, and the page would lose its default skins.
    fn skin_resources(&self) -> std::path::PathBuf {
        let has =
            |resources: &std::path::Path| default_skin_path(resources, "steve", false).is_file();
        let played = self.game_resources();
        if has(&played) {
            return played;
        }
        self.installed
            .iter()
            .map(|tag| backend::resources(tag))
            .find(|resources| has(resources))
            .unwrap_or(played)
    }

    /// The account whose look the page shows: the one that plays (the selected one, else the
    /// first), as the game picks it.
    fn look_account(&self) -> Option<account::Account> {
        self.accounts
            .selected()
            .or_else(|| self.accounts.accounts.first())
            .cloned()
    }

    /// The page opened, or another account plays: what it needs is asked for.
    pub(crate) fn skins_open(&mut self) {
        let Some(account) = self.look_account() else {
            return;
        };
        if self.skins.uuid.as_deref() != Some(&account.uuid) {
            self.skins = skins_page::Skins::new_for(&account.uuid);
        }
        self.skins.saved = skin_library::list(&skin_library::folder(&account.uuid));
        if !matches!(self.skins.capes, Some(Ok(_))) {
            self.capes_ask(&account);
        }
        if let Some((url, _)) = &account.skin {
            self.picture_ask(url.clone());
        }
        let asked = self
            .skins
            .optifine
            .as_ref()
            .is_some_and(|(name, _)| *name == account.name);
        if !asked {
            self.optifine_ask(&account.name);
        }
    }

    fn capes_ask(&mut self, account: &account::Account) {
        self.skins.asked += 1;
        self.skins.capes = None;
        let (asked, account, tx) = (self.skins.asked, account.clone(), self.tx.clone());
        std::thread::spawn(move || {
            let ep = account::Endpoints::live();
            let result = account::token_for(
                &ep,
                account::client_id().as_deref(),
                account::store(),
                &account,
            )
            .and_then(|token| capes::list(&ep, &token));
            let _ = tx.send(Msg::Capes(account.uuid, asked, result));
        });
    }

    /// Fetches a picture from Mojang's texture server once.
    fn picture_ask(&mut self, url: String) {
        if self.skins.pictures.contains_key(&url) {
            return;
        }
        self.skins.pictures.insert(url.clone(), None);
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let png = capes::picture(&url);
            let _ = tx.send(Msg::Picture(url, png));
        });
    }

    fn optifine_ask(&mut self, name: &str) {
        self.skins.optifine = Some((name.to_owned(), None));
        let (name, tx) = (name.to_owned(), self.tx.clone());
        std::thread::spawn(move || {
            // An error is shown as no cape: it is asked again next time the page opens.
            let png = capes::optifine(capes::OPTIFINE, &name).ok().flatten();
            let _ = tx.send(Msg::OptifineCape(name, png));
        });
    }

    /// A listing answered: the capes' pictures are asked for.
    pub(crate) fn capes_answered(
        &mut self,
        uuid: String,
        asked: u64,
        result: Result<Vec<Cape>, String>,
    ) {
        if !self.skins.answered(&uuid, asked, result) {
            return;
        }
        if let Some(Ok(list)) = &self.skins.capes {
            for url in self.skins.unasked(list) {
                self.picture_ask(url);
            }
        }
    }

    /// What the account wears now, as the page's choices say it.
    fn worn_look(&self, account: &account::Account) -> Look {
        let slim = match &account.skin {
            Some((_, model)) => model == "slim",
            None => {
                sunrise_launcher::heads::default_skin_file(&self.skin_resources(), &account.uuid)
                    .is_some_and(|(_, slim)| slim)
            }
        };
        let minecraft = match &self.skins.capes {
            Some(Ok(list)) => list.iter().find(|c| c.active).map(|c| c.id.clone()),
            _ => None,
        };
        let cape = match minecraft {
            Some(id) => CapePick::Minecraft(id),
            None if self.skins.optifine_png(&account.name).is_some() => CapePick::Optifine,
            None => CapePick::None,
        };
        let skin = match (&account.skin, &self.skins.worn_saved) {
            (Some((url, _)), Some((kept, id))) if url == kept => SkinPick::Saved(id.clone()),
            _ => SkinPick::Worn,
        };
        Look { skin, slim, cape }
    }

    /// A picture from the texture server arrived (`Msg::Picture`).
    pub(crate) fn skins_picture(&mut self, url: String, png: Option<Vec<u8>>) {
        self.skins.picture(url, png);
        self.remember_worn();
    }

    /// Keeps the skin worn now among the saved skins (once: the same skin is saved once).
    fn remember_worn(&mut self) {
        let Some(account) = self.look_account() else {
            return;
        };
        let Some((url, model)) = &account.skin else {
            return;
        };
        if self
            .skins
            .worn_saved
            .as_ref()
            .is_some_and(|(kept, _)| kept == url)
        {
            return;
        }
        let Some(Some(png)) = self.skins.pictures.get(url) else {
            return;
        };
        let dir = skin_library::folder(&account.uuid);
        // Kept without a name: its tile shows the skin alone.
        match skin_library::add(&dir, png, "", Some(model == "slim")) {
            Ok(saved) => {
                // The choice made before it was kept means the same skin under its saved name.
                if self
                    .skins
                    .chosen
                    .as_ref()
                    .is_some_and(|c| c.skin == SkinPick::Worn)
                {
                    if let Some(chosen) = &mut self.skins.chosen {
                        chosen.skin = SkinPick::Saved(saved.id.clone());
                    }
                }
                self.skins.worn_saved = Some((url.clone(), saved.id));
                self.skins.saved = skin_library::list(&dir);
                if self.skins.chosen == Some(self.worn_look(&account)) {
                    self.skins.chosen = None;
                }
            }
            Err(e) => eprintln!("Sunrise: the skin worn now couldn't be kept: {e}"),
        }
    }

    fn chosen_look(&self, account: &account::Account) -> Look {
        self.skins
            .chosen
            .clone()
            .unwrap_or_else(|| self.worn_look(account))
    }

    /// Chooses `change` of the look; choosing what is worn again forgets the choice.
    fn choose(&mut self, change: impl FnOnce(&mut Look)) {
        let Some(account) = self.look_account() else {
            return;
        };
        let mut look = self.chosen_look(&account);
        change(&mut look);
        let worn = self.worn_look(&account);
        self.skins.chosen = (look != worn).then_some(look);
        self.skins.said = None;
    }

    /// The key and picture of `skin` with slim or classic arms.
    fn skin_png(
        &self,
        account: &account::Account,
        skin: &SkinPick,
        slim: bool,
    ) -> (String, Option<Rc<Vec<u8>>>) {
        match skin {
            SkinPick::Worn => match &account.skin {
                Some((url, _)) => (
                    url.clone(),
                    self.skins.pictures.get(url).cloned().flatten().map(Rc::new),
                ),
                None => {
                    let resources = self.skin_resources();
                    let key = format!("uuid-default:{}", account.uuid);
                    let png = self.skins.file(&key, || {
                        sunrise_launcher::heads::default_skin_file(&resources, &account.uuid)
                            .map(|(path, _)| path)
                    });
                    (key, png)
                }
            },
            SkinPick::Saved(id) => {
                let key = format!("saved:{id}");
                let dir = skin_library::folder(&account.uuid);
                let png = self
                    .skins
                    .file(&key, || Some(dir.join(format!("{id}.png"))));
                (key, png)
            }
            SkinPick::Default(name) => {
                let key = format!("default:{name}:{slim}");
                let resources = self.skin_resources();
                let png = self
                    .skins
                    .file(&key, || Some(default_skin_path(&resources, name, slim)));
                (key, png)
            }
        }
    }

    fn skin_texture(
        &self,
        account: &account::Account,
        skin: &SkinPick,
        slim: bool,
    ) -> Option<(String, Rc<Texture>)> {
        let (key, png) = self.skin_png(account, skin, slim);
        let texture =
            self.skins
                .texture(&key, png.as_deref().map(Vec::as_slice), player_model::skin)?;
        Some((key, texture))
    }

    fn cape_texture(
        &self,
        account: &account::Account,
        cape: &CapePick,
    ) -> Option<(String, Rc<Texture>)> {
        let (key, png) = match cape {
            CapePick::None => return None,
            CapePick::Minecraft(id) => {
                let Some(Ok(list)) = &self.skins.capes else {
                    return None;
                };
                let url = &list.iter().find(|c| &c.id == id)?.url;
                (url.clone(), self.skins.pictures.get(url)?.as_ref())
            }
            CapePick::Optifine => (
                format!("optifine:{}", account.name),
                self.skins.optifine_png(&account.name),
            ),
        };
        let texture = self
            .skins
            .texture(&key, png.map(Vec::as_slice), player_model::cape)?;
        Some((key, texture))
    }

    /// Sends the chosen look to the Minecraft services: the skin when it (or its arms)
    /// changed, then the cape when it did.
    fn look_save(&mut self) {
        let (Some(account), Some(chosen)) = (self.look_account(), self.skins.chosen.clone()) else {
            return;
        };
        if self.skins.saving {
            return;
        }
        let worn = self.worn_look(&account);
        let skin = if chosen.skin != worn.skin || chosen.slim != worn.slim {
            match self.skin_png(&account, &chosen.skin, chosen.slim).1 {
                Some(png) => Some((png.as_ref().clone(), chosen.slim)),
                None => {
                    self.skins.said = Some(Err(tr("This skin's picture isn't here yet.").into()));
                    return;
                }
            }
        } else {
            None
        };
        let cape = (chosen.cape != worn.cape).then(|| match &chosen.cape {
            CapePick::Minecraft(id) => Some(id.clone()),
            // The OptiFine cape shows without a Minecraft cape over it.
            CapePick::None | CapePick::Optifine => None,
        });
        let capes = match &self.skins.capes {
            Some(Ok(list)) => list.clone(),
            _ => Vec::new(),
        };
        self.skins.asked += 1;
        self.skins.saving = true;
        self.skins.said = None;
        let (asked, tx) = (self.skins.asked, self.tx.clone());
        std::thread::spawn(move || {
            let ep = account::Endpoints::live();
            let result = account::token_for(
                &ep,
                account::client_id().as_deref(),
                account::store(),
                &account,
            )
            .and_then(|token| {
                let skin = match &skin {
                    Some((png, slim)) => account::skins::upload(&ep, &token, png, *slim)?,
                    None => None,
                };
                let capes = match &cape {
                    Some(id) => capes::wear(&ep, &token, id.as_deref())?,
                    None => capes,
                };
                Ok(backend::SavedLook { skin, capes })
            });
            let _ = tx.send(Msg::LookSaved(account.uuid, asked, Box::new(result)));
        });
    }

    /// A Save ended: what the account wears now is shown, kept in accounts.json and on its head.
    pub(crate) fn look_saved(
        &mut self,
        uuid: String,
        asked: u64,
        result: Result<backend::SavedLook, String>,
    ) {
        if self.skins.uuid.as_deref() != Some(&uuid) || asked != self.skins.asked {
            return;
        }
        self.skins.saving = false;
        let saved = match result {
            Ok(saved) => saved,
            Err(why) => {
                self.skins.said = Some(Err(why));
                return;
            }
        };
        // The skin just sent is the picture at its new URL: no need to fetch it back.
        let sent = self.skins.chosen.clone().and_then(|chosen| {
            let account = self.look_account()?;
            self.skin_png(&account, &chosen.skin, chosen.slim).1
        });
        if let (Some((url, _)), Some(png)) = (&saved.skin, sent) {
            self.skins.picture(url.clone(), Some(png.as_ref().clone()));
        }
        let worn_cape = saved.capes.iter().find(|c| c.active).map(|c| c.url.clone());
        self.skins.capes = Some(Ok(saved.capes));
        if let Some(a) = self.accounts.accounts.iter_mut().find(|a| a.uuid == uuid) {
            if saved.skin.is_some() {
                a.skin = saved.skin;
            }
            a.cape = worn_cape;
            let accounts = self.accounts.clone();
            std::thread::spawn(move || {
                if let Err(e) = accounts.save_renewed() {
                    eprintln!("Sunrise: the new skin and cape couldn't be saved: {e}");
                }
            });
        }
        self.skins.chosen = None;
        self.remember_worn();
        self.skins.said = Some(Ok(tr("Saved. Everyone sees your new look.").into()));
        self.load_profile_heads();
    }

    /// Adds skin files (chosen or dropped) to the saved skins and chooses the last one.
    fn skins_add(&mut self, paths: Vec<std::path::PathBuf>) {
        let Some(uuid) = self.skins.uuid.clone() else {
            return;
        };
        let dir = skin_library::folder(&uuid);
        for path in paths {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let added = std::fs::metadata(&path)
                .map_err(|e| e.to_string())
                .and_then(|m| {
                    if m.len() > account::skins::MAX_SKIN as u64 {
                        Err(tr("This picture is too big for a skin.").to_string())
                    } else {
                        std::fs::read(&path).map_err(|e| e.to_string())
                    }
                })
                .and_then(|png| skin_library::add(&dir, &png, &name, None));
            match added {
                Ok(saved) => {
                    self.skins.saved = skin_library::list(&dir);
                    self.choose(|look| {
                        look.skin = SkinPick::Saved(saved.id.clone());
                        look.slim = saved.slim;
                    });
                }
                Err(why) => self.skins.said = Some(Err(why)),
            }
        }
    }

    fn skins_pick_file(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(tr("Add").into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                let _ = this.update(cx, |this, cx| {
                    this.skins_add(paths);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn skin_forget(&mut self, id: &str) {
        let Some(uuid) = self.skins.uuid.clone() else {
            return;
        };
        let dir = skin_library::folder(&uuid);
        if let Err(why) = skin_library::remove(&dir, id) {
            self.skins.said = Some(Err(why));
            return;
        }
        self.skins.saved = skin_library::list(&dir);
        self.skins.forget(&format!("saved:{id}"));
        self.skins.files.get_mut().remove(&format!("saved:{id}"));
        if self
            .skins
            .chosen
            .as_ref()
            .is_some_and(|c| c.skin == SkinPick::Saved(id.to_owned()))
        {
            self.choose(|look| look.skin = SkinPick::Worn);
        }
    }

    pub(crate) fn skins_page(
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
            .track_scroll(&crate::frame::scrolls("skins-page"))
            .child(
                div()
                    .text_size(px(tokens::TITLE))
                    .font_weight(FontWeight::BOLD)
                    .child(tr("Skins")),
            );
        let Some(account) = self.look_account() else {
            return page.child(
                crate::home::card()
                    .p(px(20.))
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(12.))
                    .child(crate::settings::hint(tr(
                        "Sign in with the Microsoft account that owns Minecraft: Java Edition to choose its skin and cape.",
                    )))
                    .child(crate::frame::button("skins-sign-in", tr("Sign in with Microsoft")).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.sign_in_to_play();
                            cx.notify();
                        }),
                    )),
            );
        };
        // Another account plays now (switched in the account menu while the page is open).
        if self.skins.uuid.as_deref() != Some(&account.uuid) {
            self.skins_open();
        }
        let look = self.chosen_look(&account);
        let preview = self.look_preview(&account, &look, window, cx);
        let choices = div()
            .flex_1()
            .min_w(px(280.))
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(self.skin_choices(&account, &look, cx))
            .child(self.cape_choices(&account, &look, cx));
        page.child(
            div()
                .flex()
                .flex_wrap()
                .items_start()
                .gap(px(24.))
                .child(preview)
                .child(choices),
        )
        // A drag begun on the preview turns it wherever the pointer goes on the page.
        .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
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
        // A skin file dropped anywhere on the page is saved and chosen.
        .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(rgb(palette().hover)))
        .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
            this.skins_add(paths.paths().to_vec());
            cx.stop_propagation();
            cx.notify();
        }))
    }

    /// The player in the chosen look, turned by dragging, with the arm model and Save under it.
    fn look_preview(
        &mut self,
        account: &account::Account,
        look: &Look,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = palette();
        let skin = self.skin_texture(account, &look.skin, look.slim);
        let cape = self.cape_texture(account, &look.cape);
        let image = skin.map(|(skin_key, skin)| {
            let (w, h) = (
                (PREVIEW_W * self.scale).round() as usize,
                (PREVIEW_H * self.scale).round() as usize,
            );
            let key = {
                use std::hash::{Hash, Hasher};
                let mut hash = std::collections::hash_map::DefaultHasher::new();
                (
                    &skin_key,
                    look.slim,
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
                            slim: look.slim,
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
                Some(image) => d.child(img(image).w(px(PREVIEW_W)).h(px(PREVIEW_H))),
                None => d.child(crate::settings::hint(tr("Loading your skin…"))),
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    this.skins.drag = Some((ev.position, this.skins.yaw, this.skins.pitch));
                    cx.notify();
                }),
            );
        let arms = |slim: bool, label: &'static str| {
            let on = look.slim == slim;
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
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.choose(|look| look.slim = slim);
                    if let Some(Look {
                        skin: SkinPick::Saved(id),
                        ..
                    }) = &this.skins.chosen
                    {
                        if let Some(uuid) = this.skins.uuid.clone() {
                            let dir = skin_library::folder(&uuid);
                            let _ = skin_library::set_slim(&dir, id, slim);
                            this.skins.saved = skin_library::list(&dir);
                        }
                    }
                    cx.notify();
                }))
        };
        let changed = self.skins.chosen.is_some();
        let saving = self.skins.saving;
        let actions = div()
            .flex()
            .gap(px(8.))
            .child(
                crate::frame::primary("skins-save", tr(if saving { "Saving…" } else { "Save" }))
                    .flex_1()
                    .when(!changed || saving, |d| d.opacity(0.5).cursor_default())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.look_save();
                        cx.notify();
                    })),
            )
            .when(changed && !saving, |d| {
                d.child(
                    crate::frame::button("skins-undo", tr("Undo")).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.skins.chosen = None;
                            this.skins.said = None;
                            cx.notify();
                        },
                    )),
                )
            });
        crate::home::card()
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
            )
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
                            .child(arms(false, "Wide"))
                            .child(arms(true, "Slim")),
                    ),
            )
            .child(actions)
            .when_some(self.skins.said.clone(), |d, said| {
                d.child(match said {
                    Ok(done) => crate::settings::hint(done),
                    Err(why) => crate::settings::warning(why),
                })
            })
    }

    fn section(title: &str, about: &str) -> Div {
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
    fn tile(
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
    fn skin_thumb(&self, account: &account::Account, skin: &SkinPick, slim: bool) -> AnyElement {
        let Some((key, texture)) = self.skin_texture(account, skin, slim) else {
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

    fn skin_choices(
        &mut self,
        account: &account::Account,
        look: &Look,
        cx: &mut Context<Self>,
    ) -> Div {
        let worn_slim = self.worn_look(account).slim;
        let mut tiles = div().flex().flex_wrap().gap(px(10.));
        let pick = |this: &mut Launcher, skin: SkinPick, slim: Option<bool>| {
            this.choose(|look| {
                look.skin = skin;
                if let Some(slim) = slim {
                    look.slim = slim;
                }
            });
        };
        let worn = self.worn_look(account).skin;
        if worn == SkinPick::Worn {
            tiles = tiles.child(
                self.tile(
                    "skin-worn".into(),
                    self.skin_thumb(account, &SkinPick::Worn, worn_slim),
                    tr("Your skin").into(),
                    look.skin == SkinPick::Worn,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    pick(this, SkinPick::Worn, Some(worn_slim));
                    cx.notify();
                })),
            );
        }
        let mut history = self.skins.saved.clone();
        history.reverse();
        history.sort_by_key(|saved| SkinPick::Saved(saved.id.clone()) != worn);
        for saved in history {
            let skin = SkinPick::Saved(saved.id.clone());
            let is_worn = skin == worn;
            let id = saved.id.clone();
            let forget = div()
                .id(SharedString::from(format!("skin-forget-{}", saved.id)))
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
                .group_hover(SharedString::from(format!("skin-{}", saved.id)), |s| {
                    s.opacity(1.)
                })
                .hover(|s| s.bg(rgb(palette().hover)))
                .child(blocks::icon("close", 12., self.theme.muted))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.skin_forget(&id);
                    cx.notify();
                }));
            let slim = saved.slim;
            tiles = tiles.child(
                self.tile(
                    SharedString::from(format!("skin-{}", saved.id)),
                    self.skin_thumb(account, &skin, slim),
                    saved.name.clone().into(),
                    look.skin == skin,
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
                .border_color(rgb(p.divider))
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
        let resources = self.skin_resources();
        let defaults: Vec<&'static str> = DEFAULT_SKINS
            .into_iter()
            .filter(|name| default_skin_path(&resources, name, false).is_file())
            .collect();
        let mut section = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(Self::section(
                "Skin",
                "Add a skin from a PNG file, or drop one here. Saved skins stay on this computer.",
            ))
            .child(tiles);
        if !defaults.is_empty() {
            let mut grid = div().flex().flex_wrap().gap(px(10.));
            for name in defaults {
                let skin = SkinPick::Default(name);
                let title: String = name[..1].to_uppercase() + &name[1..];
                grid = grid.child(
                    self.tile(
                        SharedString::from(format!("skin-default-{name}")),
                        self.skin_thumb(account, &skin, look.slim),
                        title.into(),
                        look.skin == skin,
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
                .child(Self::section("Default skins", ""))
                .child(grid);
        }
        section
    }

    fn cape_choices(
        &mut self,
        account: &account::Account,
        look: &Look,
        cx: &mut Context<Self>,
    ) -> Div {
        let section = div().flex().flex_col().gap(px(12.)).child(Self::section(
            "Cape",
            "The capes your account owns. Everyone sees the one you wear, in every launcher and on every server.",
        ));
        // The OptiFine cape and No cape are choices even while the account's capes can't be listed.
        let (list, problem) = match &self.skins.capes {
            None => (
                Vec::new(),
                Some(crate::settings::hint(tr("Looking up your capes…"))),
            ),
            Some(Err(why)) => (
                Vec::new(),
                Some(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(crate::settings::warning(why.clone()))
                        .child(
                            crate::frame::button("capes-retry", tr("Try again")).on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| {
                                    if let Some(account) = this.look_account() {
                                        this.capes_ask(&account);
                                    }
                                    cx.notify();
                                }),
                            ),
                        ),
                ),
            ),
            Some(Ok(list)) => (list.clone(), None),
        };
        let mut choices: Vec<(CapePick, String)> = vec![(CapePick::None, tr("No cape").into())];
        choices.extend(
            list.iter()
                .map(|c| (CapePick::Minecraft(c.id.clone()), c.alias.clone())),
        );
        if self.skins.optifine_png(&account.name).is_some() {
            choices.push((CapePick::Optifine, tr("OptiFine").into()));
        }
        let mut tiles = div().flex().flex_wrap().gap(px(10.));
        for (cape, label) in choices {
            let face = self.cape_texture(account, &cape).map(|(key, texture)| {
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
                CapePick::Optifine => "cape-optifine".to_string(),
            });
            tiles = tiles.child(
                self.tile(id, picture, label.into(), look.cape == cape)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let cape = cape.clone();
                        this.choose(|look| look.cape = cape);
                        cx.notify();
                    })),
            );
        }
        section
            .child(tiles)
            .when_some(problem, |d, problem| d.child(problem))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cape(id: &str, active: bool) -> Cape {
        Cape {
            id: id.into(),
            alias: id.into(),
            url: format!("http://textures.minecraft.net/texture/{id}"),
            active,
        }
    }

    #[test]
    fn only_the_last_listing_for_the_account_shown_is_kept() {
        let mut skins = Skins::new_for("a");
        skins.asked = 2;
        assert!(!skins.answered("a", 1, Ok(vec![cape("old", true)])));
        assert!(!skins.answered("b", 2, Ok(vec![cape("other", true)])));
        assert!(skins.capes.is_none());
        assert!(skins.answered("a", 2, Ok(vec![cape("c1", true)])));
        assert_eq!(skins.capes, Some(Ok(vec![cape("c1", true)])));
    }

    #[test]
    fn each_picture_is_asked_for_once() {
        let mut skins = Skins::new_for("a");
        let list = [cape("c1", true), cape("c2", false)];
        assert_eq!(skins.unasked(&list).len(), 2);
        skins.picture(list[0].url.clone(), None);
        assert_eq!(skins.unasked(&list), [list[1].url.clone()]);
    }

    #[test]
    fn an_optifine_answer_counts_only_for_the_name_asked() {
        let mut skins = Skins::new_for("a");
        skins.optifine = Some(("Notch".into(), None));
        skins.optifine_cape("jeb_", Some(vec![1]));
        assert!(skins.optifine_png("Notch").is_none());
        skins.optifine_cape("Notch", Some(vec![1]));
        assert_eq!(skins.optifine_png("Notch"), Some(&vec![1]));
        assert!(skins.optifine_png("jeb_").is_none());
        skins.optifine_cape("Notch", None);
        assert!(skins.optifine_png("Notch").is_none(), "no cape, no tile");
    }
}
