//! Home's Mods column: what the selected profile plays with, to read. Its mods and its
//! resource and shader packs that are on, each a row that opens the item's own page; what is
//! off is one quiet line that leads to Mods, where things are turned on and off.
//!
//! What the profile has is read in the background ([`read`]) and kept ([`Inventory`]): drawing
//! Home opens no file of a mod or a pack. It is read again when the files it was read from
//! change ([`stamp`]).
use super::*;
use crate::frame::{button, Page, Reach as _};
use crate::home::{card, heading, muted};
use crate::mod_icons::tile;
use crate::paint::palette;
use gpui::{rgb, RenderImage};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use sunrise_launcher::browser::{installed, mod_list, sunrise};
use sunrise_launcher::packs::{self, Kind as PackKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Mod,
    Resource,
    Shader,
}

impl Kind {
    const ALL: [Kind; 3] = [Kind::Mod, Kind::Resource, Kind::Shader];

    pub(crate) fn of(pack: PackKind) -> Kind {
        match pack {
            PackKind::Resource => Kind::Resource,
            PackKind::Shader => Kind::Shader,
        }
    }

    /// Where the ones of this kind that are turned off are listed.
    pub(crate) fn turned_off(self) -> crate::mods::Installed {
        use crate::mods::Installed;
        match self {
            Kind::Mod => Installed::ModsOff,
            Kind::Resource => Installed::Shelf(PackKind::Resource),
            Kind::Shader => Installed::Shelf(PackKind::Shader),
        }
    }

    /// "2 mods", for the line that counts what is turned off.
    fn counted(self, count: usize) -> String {
        let (one, many) = match self {
            Kind::Mod => ("1 mod", "{count} mods"),
            Kind::Resource => ("1 resource pack", "{count} resource packs"),
            Kind::Shader => ("1 shader pack", "{count} shader packs"),
        };
        match count {
            1 => tr(one).to_string(),
            _ => trf(many, &[("count", &count.to_string())]),
        }
    }

    /// What a row says under the name.
    fn label(self) -> &'static str {
        match self {
            Kind::Mod => "Mod",
            Kind::Resource => "Resource pack",
            Kind::Shader => "Shader pack",
        }
    }
}

/// Where a row's picture comes from.
#[derive(Clone)]
pub(crate) enum Picture {
    /// A registry mod's icon, by the mod's project there: asked for when its row is drawn.
    Project(String),
    /// The local mod's own picture, read with it.
    Image(Option<Arc<gpui::Image>>),
    /// Pack artwork is read and decoded only when its row is first built.
    Pack(Arc<PackPicture>),
}

pub(crate) struct PackPicture {
    path: PathBuf,
    kind: PackKind,
    thumbnail: Mutex<Thumbnail>,
}

#[derive(Default)]
enum Thumbnail {
    #[default]
    Unasked,
    Loading,
    Ready(Option<Arc<RenderImage>>),
}

impl PackPicture {
    fn show(self: &Arc<Self>, cx: &mut Context<Launcher>) -> Option<Arc<RenderImage>> {
        let mut thumbnail = self.thumbnail.lock().unwrap();
        match &*thumbnail {
            Thumbnail::Ready(image) => return image.clone(),
            Thumbnail::Loading => return None,
            Thumbnail::Unasked => *thumbnail = Thumbnail::Loading,
        }
        let (path, kind) = (self.path.clone(), self.kind);
        let requested = Arc::downgrade(self);
        let task = cx
            .background_executor()
            .spawn(async move { pack_thumbnail(packs::artwork(&path, kind)?) });
        cx.spawn(async move |this, cx| {
            let image = task.await;
            let _ = this.update(cx, |_, cx| {
                if let Some(requested) = requested.upgrade() {
                    let has_picture = image.is_some();
                    *requested.thumbnail.lock().unwrap() = Thumbnail::Ready(image);
                    if has_picture {
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        None
    }
}

/// Decode the source only here, with limits; GPUI gets a paint-ready square, never the source.
fn pack_thumbnail(bytes: Vec<u8>) -> Option<Arc<RenderImage>> {
    const SIDE: u32 = PICTURE as u32 * 2;
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 << 20);
    reader.limits(limits);
    let decoded = reader.decode().ok()?;
    let edge = decoded.width().min(decoded.height());
    // Crop before scaling so a very thin source can't create a huge intermediate image.
    let mut rgba = decoded
        .crop_imm(
            (decoded.width() - edge) / 2,
            (decoded.height() - edge) / 2,
            edge,
            edge,
        )
        .resize_exact(SIDE, SIDE, image::imageops::FilterType::Triangle)
        .into_rgba8();
    for p in rgba.pixels_mut() {
        let [r, g, b, a] = p.0;
        let premultiply = |v| (u16::from(v) * u16::from(a) / 255) as u8;
        p.0 = [premultiply(b), premultiply(g), premultiply(r), a];
    }
    Some(Arc::new(RenderImage::new([image::Frame::new(rgba)])))
}

/// What a press on a row opens.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Open {
    /// A registry mod's page, by its id.
    Project(String),
    /// A mod the registry doesn't know: Mods › Installed, at the mod with this ID.
    Installed(String),
    /// A pack: its shelf, at the pack that is this file.
    Pack(PackKind, String),
}

/// One installed mod or pack of the profile.
#[derive(Clone)]
pub(crate) struct Item {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub picture: Picture,
    pub open: Open,
    pub on: bool,
    // Matches the version in native-mods.json when counting what the build left out.
    mod_version: Option<String>,
}

/// What the column lists: the items that are on, in the order given, and how many of each
/// kind are off.
pub(crate) struct Column {
    pub shown: Vec<Item>,
    /// By kind, as [`Kind::ALL`] orders them.
    pub off: [usize; 3],
}

/// The column for a profile with `items` installed; `None` when it has nothing at all.
pub(crate) fn column(items: Vec<Item>) -> Option<Column> {
    if items.is_empty() {
        return None;
    }
    Some(Column {
        off: Kind::ALL.map(|kind| items.iter().filter(|i| !i.on && i.kind == kind).count()),
        shown: items.into_iter().filter(|i| i.on).collect(),
    })
}

/// Everything the profile in `home` has installed: its registry mods and the mod files put
/// there by hand, by name, then its resource packs as the game stacks them, then its shader
/// packs. Each is on or off, with references to pack artwork, not the images. Reads files:
/// never called while drawing.
pub(crate) fn read(home: &Path) -> Vec<Item> {
    let image = |png: Vec<u8>| Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, png));
    let mut mods: Vec<Item> = installed::Installed::load(home)
        .entries
        .iter()
        .filter(|e| {
            e.source == sunrise_launcher::browser::SourceId::Sunrise
                && e.kind == sunrise_launcher::browser::Kind::Mod
        })
        .filter_map(|e| {
            let id = sunrise::mod_id(e)?;
            Some(Item {
                id: id.to_owned(),
                name: e.title.clone(),
                kind: Kind::Mod,
                picture: Picture::Project(e.project.clone()),
                open: Open::Project(e.project.clone()),
                on: mod_list::is_enabled(home, id),
                mod_version: Some(e.number.clone()),
            })
        })
        .collect();
    // Local native and content packages are also part of the profile.
    let by_hand = native_status::untracked(home)
        .into_iter()
        .chain(native_status::untracked_content(home));
    mods.extend(by_hand.map(|file| {
        Item {
            on: mod_list::is_enabled(home, &file.id),
            mod_version: Some(file.version),
            name: file.name,
            kind: Kind::Mod,
            picture: Picture::Image(
                file.icon
                    .ok()
                    .flatten()
                    .map(|icon| image(icon.bytes().to_vec())),
            ),
            open: Open::Installed(file.id.clone()),
            id: file.id,
        }
    }));
    if let Ok(Some(active)) = sunrise_launcher::managed_play::Active::read_pointer(home) {
        // A Switch hold is shown in Installed, not as a Home mod or in its off count.
        let held: HashSet<_> = active.holds().map(|hold| hold.id()).collect();
        mods.retain(|item| !held.contains(item.id.as_str()));
    }
    mods.sort_by_key(|item| item.name.to_lowercase());
    let pack = |pack: packs::Pack| Item {
        kind: Kind::of(pack.kind),
        picture: Picture::Pack(Arc::new(PackPicture {
            path: pack.path,
            kind: pack.kind,
            thumbnail: Mutex::default(),
        })),
        open: Open::Pack(pack.kind, pack.file.clone()),
        on: pack.enabled,
        mod_version: None,
        id: pack.file,
        name: pack.title,
    };
    let mut resource = packs::list_with(home, PackKind::Resource, packs::Pictures::None);
    let order = packs::resource_order(home);
    // Stable: the packs that are off stay by name after the stack.
    resource.sort_by_key(|p| {
        order
            .iter()
            .position(|file| *file == p.file)
            .unwrap_or(usize::MAX)
    });
    let shader = packs::list_with(home, PackKind::Shader, packs::Pictures::None);
    mods.into_iter()
        .chain(resource.into_iter().map(pack))
        .chain(shader.into_iter().map(pack))
        .collect()
}

/// A home and the builds that can affect its launch summary. These inputs come from the UI's
/// in-memory profile and installed-build lists; files are read only by the worker.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Query {
    home: PathBuf,
    version: Option<String>,
    newest: Option<String>,
}

/// Just the facts about installed mods needed to draw Play and its strip. The launch and
/// consent paths continue to read live state when the player actually acts.
#[derive(Default)]
pub(crate) struct LaunchSummary {
    pub(crate) mods_off: usize,
    pub(crate) update_check: Option<String>,
}

#[derive(Default)]
struct Snapshot {
    column: Option<Column>,
    launch: LaunchSummary,
}

fn stamp(query: &Query) -> String {
    let home = &query.home;
    let build = query.version.as_deref().and_then(|version| {
        std::fs::metadata(backend::version_dir(version).join("native-mods.json")).ok()
    });
    format!(
        "{}\n{}\n{}\n{:?}",
        native_status::fingerprint(home, query.version.as_deref()),
        packs::stamp(home),
        native_mods::kept_stamp(home),
        build.map(|m| (m.len(), m.modified().ok())),
    )
}

fn read_changed(query: &Query, known: &str) -> Option<(String, Snapshot)> {
    let stamp = stamp(query);
    if stamp == known {
        return None;
    }
    let items = read(&query.home);
    let build = query
        .version
        .as_deref()
        .and_then(native_status::Build::read);
    let update_check = build
        .as_ref()
        .zip(query.newest.as_deref())
        .filter(|(build, next)| backend::compare_versions(next, &build.base).is_gt())
        .filter(|_| !native_status::enabled(&query.home).is_empty())
        .map(|(_, next)| crate::sheets::check_key(&query.home, next));
    let left_out = match build {
        Some(build) => build.left_out,
        None => query.version.as_deref().map_or_else(Vec::new, |version| {
            native_mods::kept_left_out(&query.home, version)
        }),
    };
    let left: HashSet<_> = left_out
        .iter()
        .map(|m| (m.id.as_str(), m.version.as_str()))
        .collect();
    let mods_off = items
        .iter()
        .filter(|item| {
            item.on
                && item
                    .mod_version
                    .as_deref()
                    .is_some_and(|version| left.contains(&(item.id.as_str(), version)))
        })
        .count();
    Some((
        stamp,
        Snapshot {
            column: column(items),
            launch: LaunchSummary {
                mods_off,
                update_check,
            },
        },
    ))
}

/// What the selected profile has installed, as last read.
#[derive(Default)]
pub(crate) struct Inventory {
    query: Option<Query>,
    /// None until the first read ends: Home says nothing about mods before it knows.
    snapshot: Option<Arc<Snapshot>>,
    stamp: String,
    generation: u64,
    reading: bool,
    /// Something may have changed: check the stamp, in the background, when Home next draws.
    stale: bool,
    drawn: Option<Instant>,
    watched: bool,
}

/// Home drawn again after this long checks the files once more: the player was elsewhere.
const AWAY: Duration = Duration::from_secs(1);
/// How often the files are checked while Home stays open.
const WATCH: Duration = Duration::from_secs(2);

impl Inventory {
    pub(crate) fn launch(&self) -> Option<&LaunchSummary> {
        self.snapshot.as_deref().map(|snapshot| &snapshot.launch)
    }

    /// Starts a read only if needed. Returns the generation and the stamp to read against.
    fn draw(&mut self, query: &Query, now: Instant) -> Option<(u64, String)> {
        if self.query.as_ref() != Some(query) {
            self.query = Some(query.clone());
            self.snapshot = None;
            self.stamp.clear();
            self.reading = false;
        }
        let away = self
            .drawn
            .is_none_or(|last| now.duration_since(last) > AWAY);
        self.drawn = Some(now);
        if self.reading || !(self.snapshot.is_none() || self.stale || away) {
            return None;
        }
        self.reading = true;
        self.stale = false;
        self.generation += 1;
        Some((self.generation, self.stamp.clone()))
    }

    /// Only the active request may clear the in-flight flag or publish a snapshot, even when
    /// switching A → B → A before the first read of A returns.
    fn took(&mut self, generation: u64, read: Option<(String, Snapshot)>) -> Option<Arc<Snapshot>> {
        if self.generation != generation {
            return None;
        }
        self.reading = false;
        match read {
            Some((stamp, snapshot)) => {
                self.stamp = stamp;
                self.snapshot.replace(Arc::new(snapshot))
            }
            None => {
                self.snapshot.get_or_insert_default();
                None
            }
        }
    }
}

/// Release superseded pack thumbnails from the window's sprite atlas on its next frame.
fn retire(snapshot: &Snapshot, pictures: &mut crate::picture_cache::Pictures) {
    for item in snapshot.column.iter().flat_map(|column| &column.shown) {
        if let Picture::Pack(art) = &item.picture {
            if let Thumbnail::Ready(Some(image)) = &*art.thumbnail.lock().unwrap() {
                pictures.retire(image.clone());
            }
        }
    }
}

/// Home's two columns stand side by side from this window width; under it they stack.
const SIDE_BY_SIDE_FROM: f32 = 1000.;

pub(crate) fn side_by_side(window_width: f32) -> bool {
    window_width >= SIDE_BY_SIDE_FROM
}

const PICTURE: f32 = 40.;

/// A line of text that is pressed like a link: a Tab stop with the ring.
fn link(id: &'static str, label: String) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .cursor_pointer()
        .tab_index(0)
        .rounded(crate::tokens::r(crate::tokens::R_SMALL))
        .text_size(px(tokens::SMALL))
        .focus_visible(|s| s.shadow(crate::frame::focus_ring(palette().focus)))
        .seen()
        .child(label)
}

impl Launcher {
    fn open_home_item(&mut self, open: Open) {
        match open {
            Open::Project(id) => self.mods_show_project(id),
            Open::Installed(id) => self.open_installed(crate::mods::Installed::Mod(id)),
            Open::Pack(kind, file) => self.open_installed(crate::mods::Installed::Pack(kind, file)),
        }
    }

    /// Refresh what the selected profile has installed off-thread. Home and the status strip
    /// read the same answer; no launch summary reads a file while drawing.
    pub(crate) fn refresh_home_inventory(&mut self, cx: &mut Context<Self>) {
        let Some(instance) = self.selected_instance() else {
            if let Some(old) = self.ui.home.mods.snapshot.take() {
                retire(&old, &mut self.pictures);
            }
            // Invalidate any answer for the previously selected profile.
            if self.ui.home.mods.query.take().is_some() {
                self.ui.home.mods.generation += 1;
            }
            return;
        };
        let query = Query {
            home: instance.home.clone(),
            version: self.version_of(instance).cloned(),
            newest: native_mods::newest_installed(&self.installed).cloned(),
        };
        let mods = &mut self.ui.home.mods;
        if mods.query.as_ref() != Some(&query) {
            if let Some(old) = mods.snapshot.take() {
                retire(&old, &mut self.pictures);
            }
        }
        if let Some((generation, known)) = mods.draw(&query, Instant::now()) {
            let task = cx
                .background_executor()
                .spawn(async move { read_changed(&query, &known) });
            cx.spawn(async move |this, cx| {
                let read = task.await;
                let _ = this.update(cx, |this, cx| {
                    if this.ui.home.mods.generation != generation {
                        return;
                    }
                    let changed = read.is_some();
                    if let Some(old) = this.ui.home.mods.took(generation, read) {
                        retire(&old, &mut this.pictures);
                    }
                    if changed {
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        if !std::mem::replace(&mut self.ui.home.mods.watched, true) {
            // While Home shows, its files are looked at now and then, never while drawing: a
            // change made elsewhere (the game, the command line) reaches the column.
            cx.spawn(async move |this, cx| loop {
                cx.background_executor().timer(WATCH).await;
                let Ok(shown) = this.update(cx, |this, _| {
                    let mods = &this.ui.home.mods;
                    if this.ui.page != Page::Home || mods.reading {
                        return None;
                    }
                    Some((mods.query.clone()?, mods.generation, mods.stamp.clone()))
                }) else {
                    break;
                };
                let Some((query, generation, known)) = shown else {
                    continue;
                };
                let now = cx
                    .background_executor()
                    .spawn(async move { stamp(&query) })
                    .await;
                if now != known {
                    let _ = this.update(cx, |this, cx| {
                        if this.ui.home.mods.generation == generation {
                            this.ui.home.mods.stale = true;
                            cx.notify();
                        }
                    });
                }
            })
            .detach();
        }
    }

    /// Home's Mods column for the profile whose game folder is `home`.
    pub(crate) fn home_mods(&mut self, home: &Path, cx: &mut Context<Self>) -> Div {
        let snapshot = self.ui.home.mods.snapshot.clone();
        let column = snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.column.as_ref());
        let index = sunrise::known_index();
        let head = div()
            .h(px(tokens::CONTROL))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .child(heading(tr("Mods")))
            .child(
                button("alt-home-manage-mods", tr("Manage mods")).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.navigate(Page::Discover);
                        cx.notify();
                    },
                )),
            );
        let body = match column {
            // Still being read: no "none yet" until that is known.
            None if snapshot.is_none() => div(),
            None => div()
                .flex()
                .items_center()
                .gap(px(8.))
                .py(px(8.))
                .child(muted(tr("No mods yet")))
                .child(
                    link("alt-home-browse-mods", tr("Browse mods").to_string())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette().text))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            // Discover itself, wherever Mods was last left.
                            this.show_discover();
                            cx.notify();
                        })),
                ),
            Some(column) => {
                let (built, place) = self.ui.home.mods_at.clone().list(
                    column.shown.len(),
                    1,
                    crate::home::ROW,
                    crate::home::ROW_GAP,
                );
                let mut rows = div().flex().flex_col().gap(px(crate::home::ROW_GAP));
                for item in &column.shown[built] {
                    let picture = match &item.picture {
                        Picture::Project(project) => {
                            self.ensure_mod_catalog();
                            tile(
                                index
                                    .as_ref()
                                    .and_then(|index| index.project(project)?.icon.clone())
                                    .and_then(|url| {
                                        self.mods_image(
                                            &url,
                                            sunrise_launcher::browser::images::ROW,
                                        )
                                    }),
                                &item.id,
                                &item.name,
                                PICTURE,
                            )
                        }
                        Picture::Image(image) => tile(image.clone(), &item.id, &item.name, PICTURE),
                        Picture::Pack(art) => tile(art.show(cx), &item.id, &item.name, PICTURE),
                    };
                    let open = item.open.clone();
                    rows = rows.child(
                        card()
                            // By what it is, not where it stands: when the list changes
                            // under the keyboard, the focus does not pass to a neighbour.
                            .id(SharedString::from(format!(
                                "alt-home-mod:{}:{:?}:{}",
                                home.display(),
                                item.kind,
                                item.id
                            )))
                            .flex_none()
                            .min_h(px(crate::home::ROW))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .p(px(5.))
                            .pr(px(10.))
                            .cursor_pointer()
                            .tab_index(0)
                            .hover(|s| s.bg(rgb(palette().hover)))
                            .focus_visible(|s| s.border_color(rgb(palette().focus)))
                            .seen()
                            .child(picture)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .text_size(px(tokens::LABEL))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(palette().text))
                                            .truncate()
                                            .child(item.name.clone()),
                                    )
                                    .child(muted(tr(item.kind.label()))),
                            )
                            .child(blocks::icon(
                                "chevron-right",
                                14.,
                                rgb(palette().muted).into(),
                            ))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.open_home_item(open.clone());
                                cx.notify();
                            })),
                    );
                }
                let mut rows = div().flex().flex_col().gap(px(4.)).child(place(rows));
                // What is off is counted by kind, each count leading to where those are listed.
                if column.off.iter().any(|&count| count > 0) {
                    let mut line = div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_x(px(10.))
                        .gap_y(px(4.))
                        .mt(px(4.))
                        .px(px(6.))
                        .child(muted(tr("Turned off:")));
                    for (kind, count) in Kind::ALL.into_iter().zip(column.off) {
                        if count == 0 {
                            continue;
                        }
                        let id = match kind {
                            Kind::Mod => "alt-home-off-mods",
                            Kind::Resource => "alt-home-off-resource",
                            Kind::Shader => "alt-home-off-shader",
                        };
                        line = line.child(
                            link(id, kind.counted(count))
                                .text_color(rgb(palette().muted))
                                .hover(|s| s.text_color(rgb(palette().text)))
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.open_installed(kind.turned_off());
                                    cx.notify();
                                })),
                        );
                    }
                    rows = rows.child(line);
                }
                rows
            }
        };
        div().flex().flex_col().gap(px(10.)).child(head).child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, kind: Kind, on: bool) -> Item {
        Item {
            id: name.to_lowercase(),
            name: name.into(),
            kind,
            picture: Picture::Image(None),
            open: Open::Installed(name.into()),
            on,
            mod_version: (kind == Kind::Mod).then(|| "1.0.0".into()),
        }
    }

    fn names(column: &Column) -> Vec<&str> {
        column.shown.iter().map(|i| i.name.as_str()).collect()
    }

    #[test]
    fn only_a_paint_sized_thumbnail_reaches_the_renderer() {
        let png = |wide: u32, tall: u32| {
            let mut bytes = Vec::new();
            image::RgbaImage::from_pixel(wide, tall, image::Rgba([220, 80, 40, 255]))
                .write_to(
                    &mut std::io::Cursor::new(&mut bytes),
                    image::ImageFormat::Png,
                )
                .unwrap();
            assert!(
                bytes.len() < 4 << 20,
                "under the pack entry's compressed read cap"
            );
            bytes
        };
        let art = pack_thumbnail(png(1024, 64)).expect("an ordinary large PNG is resized");
        assert_eq!((art.size(0).width.0, art.size(0).height.0), (80, 80));
        assert!(
            pack_thumbnail(png(4097, 1)).is_none(),
            "reject over-limit dimensions"
        );
    }

    #[test]
    fn the_column_lists_only_what_is_on_and_counts_what_is_off() {
        let column = column(vec![
            item("Lanternlight", Kind::Mod, true),
            item("Trailmarks", Kind::Mod, false),
            item("Quiet Rain", Kind::Mod, true),
            item("Soft Edges", Kind::Resource, true),
            item("Old Paper", Kind::Resource, false),
            item("Evening Glow", Kind::Shader, false),
        ])
        .unwrap();
        assert_eq!(names(&column), ["Lanternlight", "Quiet Rain", "Soft Edges"]);
        assert_eq!(column.off, [1, 1, 1], "mods, resource packs, shader packs");
    }

    #[test]
    fn each_kind_that_is_off_is_counted_and_leads_to_its_own_list() {
        use crate::mods::Installed;
        let all_on = column(vec![
            item("Lanternlight", Kind::Mod, true),
            item("Soft Edges", Kind::Resource, true),
        ])
        .unwrap();
        assert_eq!((all_on.shown.len(), all_on.off), (2, [0, 0, 0]));
        // Only a shader pack is off: it is counted as one, not as a mod.
        let shader = column(vec![
            item("Lanternlight", Kind::Mod, true),
            item("Evening Glow", Kind::Shader, false),
        ])
        .unwrap();
        assert_eq!(shader.off, [0, 0, 1]);
        assert!(Kind::Mod.turned_off() == Installed::ModsOff);
        assert!(Kind::Resource.turned_off() == Installed::Shelf(PackKind::Resource));
        assert!(Kind::Shader.turned_off() == Installed::Shelf(PackKind::Shader));
    }

    #[test]
    fn a_profile_with_nothing_is_empty_and_one_with_everything_off_is_not() {
        assert!(column(Vec::new()).is_none());
        let all_off = column(vec![
            item("Trailmarks", Kind::Mod, false),
            item("Old Paper", Kind::Resource, false),
        ])
        .unwrap();
        assert!(all_off.shown.is_empty());
        assert_eq!(all_off.off, [1, 1, 0]);
    }

    fn query(path: impl Into<PathBuf>) -> Query {
        Query {
            home: path.into(),
            version: None,
            newest: None,
        }
    }

    #[test]
    fn the_profiles_list_is_read_once_and_again_only_when_asked() {
        let a = query("/profiles/a");
        let start = Instant::now();
        let mut kept = Inventory::default();
        let (generation, stamp) = kept.draw(&a, start).unwrap();
        assert!(stamp.is_empty() && kept.snapshot.is_none());
        assert!(kept.draw(&a, start).is_none(), "one read at a time");
        kept.took(
            generation,
            Some((
                "one".into(),
                Snapshot {
                    column: column(vec![item("Lanternlight", Kind::Mod, true)]),
                    launch: LaunchSummary::default(),
                },
            )),
        );
        let soon = start + Duration::from_millis(100);
        let shown = kept.snapshot.clone().unwrap();
        assert!(kept.draw(&a, soon).is_none(), "a redraw reads nothing");
        kept.stale = true;
        let (generation, known) = kept.draw(&a, soon).unwrap();
        assert_eq!(known, "one");
        kept.took(generation, None);
        assert!(
            Arc::ptr_eq(kept.snapshot.as_ref().unwrap(), &shown),
            "unchanged: kept"
        );
        let later = soon + AWAY + Duration::from_millis(1);
        assert_eq!(kept.draw(&a, later).unwrap().1, "one");
    }

    #[test]
    fn going_back_to_a_profile_does_not_accept_its_old_in_flight_answer() {
        let (a, b) = (query("/profiles/a"), query("/profiles/b"));
        let now = Instant::now();
        let mut kept = Inventory::default();
        let first_a = kept.draw(&a, now).unwrap().0;
        let for_b = kept.draw(&b, now).unwrap().0;
        let second_a = kept.draw(&a, now).unwrap().0;
        kept.took(for_b, None);
        kept.took(first_a, Some(("old-a".into(), Snapshot::default())));
        assert!(kept.snapshot.is_none());
        assert!(
            kept.reading,
            "older answers can't clear the new read's flag"
        );
        kept.took(second_a, Some(("new-a".into(), Snapshot::default())));
        kept.took(first_a, Some(("old-a".into(), Snapshot::default())));
        assert_eq!(kept.stamp, "new-a");
        assert!(kept.snapshot.is_some() && !kept.reading);
    }

    /// A profile's game folder with one resource pack and one registry mod, both on.
    fn profile(name: &str) -> PathBuf {
        use std::io::Write as _;
        let home = std::env::temp_dir().join(format!(
            "sunrise-launcher-home-mods-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("resourcepacks")).unwrap();
        let file = std::fs::File::create(home.join("resourcepacks/Soft Edges.zip")).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("pack.mcmeta", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(br#"{"pack":{"pack_format":99,"description":"made for the test"}}"#)
            .unwrap();
        zip.finish().unwrap();
        std::fs::write(
            home.join("options.txt"),
            "resourcePacks:[\"vanilla\",\"file/Soft Edges.zip\"]\n",
        )
        .unwrap();
        std::fs::create_dir_all(home.join("sunrise-native-mods")).unwrap();
        std::fs::write(
            home.join("sunrise-native-mods/lanternlight.smod"),
            b"placeholder",
        )
        .unwrap();
        std::fs::write(home.join("sunrise-mods.txt"), "mod.lanternlight:true\n").unwrap();
        std::fs::write(
            home.join("mod-browser.json"),
            serde_json::json!({"entries": [{
                "source": "sunrise", "project": "lanternlight", "slug": "lanternlight",
                "title": "Lanternlight", "kind": "mod", "version": "1.0.0", "number": "1.0.0",
                "file": "sunrise-native-mods/lanternlight.smod",
                "checksum": format!("SHA-256:{}", "0".repeat(64)), "page": "", "dependency": false,
            }]})
            .to_string(),
        )
        .unwrap();
        home
    }

    #[test]
    fn the_watched_snapshot_updates_pack_mod_and_launch_state_together() {
        let home = profile("cli");
        let mut query = query(&home);
        query.version = Some("test-base".into());
        std::fs::write(
            home.join("native-left-out.json"),
            r#"{
            "base": "test-base",
            "left_out": [
                {"id": "lanternlight", "version": "1.0.0", "why": "did-not-build"},
                {"id": "lanternlight", "version": "0.9.0", "why": "did-not-build"},
                {"id": "absent", "version": "1.0.0", "why": "did-not-build"}
            ]
        }"#,
        )
        .unwrap();
        let mut kept = Inventory::default();
        let now = Instant::now();
        let (generation, known) = kept.draw(&query, now).unwrap();
        kept.took(generation, read_changed(&query, &known));
        let snapshot = kept.snapshot.as_deref().unwrap();
        let column = snapshot.column.as_ref().unwrap();
        assert_eq!(names(column), ["Lanternlight", "Soft Edges.zip"]);
        assert_eq!(column.off, [0, 0, 0]);
        assert_eq!(
            snapshot.launch.mods_off, 1,
            "only the enabled recorded version"
        );
        assert!(read_changed(&query, &kept.stamp).is_none());

        // `sunrise-launcher packs off` and `mods disable` use these same setters.
        let pack = packs::find(&home, PackKind::Resource, "Soft Edges.zip").unwrap();
        packs::set_enabled(&home, &pack, false).unwrap();
        mod_list::set_enabled(&home, "lanternlight", false).unwrap();
        assert_eq!(
            kept.launch().unwrap().mods_off,
            1,
            "launch redraw reads only its current snapshot"
        );
        assert_ne!(stamp(&query), kept.stamp);
        kept.stale = true;
        let (generation, known) = kept.draw(&query, now).unwrap();
        kept.took(generation, read_changed(&query, &known));
        let snapshot = kept.snapshot.as_deref().unwrap();
        let column = snapshot.column.as_ref().unwrap();
        assert!(column.shown.is_empty());
        assert_eq!(column.off, [1, 1, 0]);
        assert_eq!(snapshot.launch.mods_off, 0);
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn every_kind_names_itself() {
        assert_eq!(
            [Kind::Mod, Kind::Resource, Kind::Shader].map(Kind::label),
            ["Mod", "Resource pack", "Shader pack"]
        );
    }

    #[test]
    fn the_columns_stack_on_a_narrow_window() {
        assert!(side_by_side(1280.));
        assert!(side_by_side(1000.));
        assert!(!side_by_side(999.));
        assert!(!side_by_side(800.));
    }
}
