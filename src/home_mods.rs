//! The selected profile’s Mods and Resource Packs overviews and the Play bar’s launch facts.
//!
//! What the profile has is read in the background ([`read`]) and kept ([`Inventory`]): drawing
//! Home opens no file of a mod or a pack. It is read again when the files it was read from
//! change ([`stamp`]).
use super::*;
use crate::frame::{button, Page};
use crate::home::tabs::{self, Content};
use crate::mod_icons::tile;
use gpui::RenderImage;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use sunrise_launcher::backend::lock::recover;
use sunrise_launcher::browser::{installed, mod_list, sunrise};
use sunrise_launcher::packs::{self, Kind as PackKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Mod,
    Resource,
}

/// Where a row's picture comes from.
#[derive(Clone)]
pub(crate) enum Picture {
    /// A registry mod's icon, by the mod's project there: asked for when its row is drawn.
    Project(String),
    /// A local package's icon, with a verified source receipt when it was built from source.
    Local {
        package: Option<Arc<gpui::Image>>,
        source: Option<Arc<sunrise_launcher::github_mods::Origin>>,
    },
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
        let mut thumbnail = recover(&self.thumbnail);
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
                    *recover(&requested.thumbnail) = Thumbnail::Ready(image);
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
    pub(crate) manual_badge: Option<crate::mods::ManualBadge>,
    // Matches the version in native-mods.json when counting what the build left out.
    mod_version: Option<String>,
}

/// Order is carried in from the inventory: mods by name; packs by priority, then name.
#[derive(Default)]
struct Collections {
    mods: Vec<Item>,
    mods_off: usize,
    resources: Vec<Item>,
}

impl FromIterator<Item> for Collections {
    fn from_iter<T: IntoIterator<Item = Item>>(items: T) -> Self {
        let mut tabs = Self::default();
        for item in items {
            match item.kind {
                Kind::Mod if item.on => tabs.mods.push(item),
                Kind::Mod => tabs.mods_off += 1,
                Kind::Resource => tabs.resources.push(item),
            }
        }
        tabs
    }
}

/// Everything the profile in `home` has installed: its registry mods and the mod files put
/// there by hand, by name, then its resource packs as the game stacks them.
/// Each is on or off, with references to pack artwork, not the images. Reads files:
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
                manual_badge: None,
                mod_version: Some(e.number.clone()),
            })
        })
        .collect();
    // Local native and content packages are also part of the profile.
    let by_hand = native_status::untracked(home)
        .into_iter()
        .chain(native_status::untracked_content(home));
    mods.extend(by_hand.map(|file| {
        let manual_badge = match &file.origin {
            None => Some(crate::mods::ManualBadge::Local),
            Some(origin) if origin.is_manual() => Some(crate::mods::ManualBadge::FromSource),
            Some(_) => None,
        };
        Item {
            on: mod_list::is_enabled(home, &file.id),
            manual_badge,
            mod_version: Some(file.version),
            name: file.name,
            kind: Kind::Mod,
            picture: Picture::Local {
                package: file
                    .icon
                    .ok()
                    .flatten()
                    .map(|icon| image(icon.bytes().to_vec())),
                source: file
                    .origin
                    .filter(|origin| origin.source.is_some())
                    .map(Arc::new),
            },
            open: Open::Installed(file.id.clone()),
            id: file.id,
        }
    }));
    if let Ok(Some(active)) = sunrise_launcher::managed_play::Active::read_pointer(home) {
        // Switch holds stay off on the committed build: keep them out of Home's enabled list,
        // but count them as turned off so the link opens Installed's off filter.
        let held: HashSet<_> = active.holds().map(|hold| hold.id()).collect();
        for item in &mut mods {
            if held.contains(item.id.as_str()) {
                item.on = false;
            }
        }
    }
    mods.sort_by_key(|item| item.name.to_lowercase());
    let pack = |pack: packs::Pack| Item {
        kind: Kind::Resource,
        picture: Picture::Pack(Arc::new(PackPicture {
            path: pack.path,
            kind: pack.kind,
            thumbnail: Mutex::default(),
        })),
        open: Open::Pack(pack.kind, pack.file.clone()),
        on: pack.enabled,
        manual_badge: None,
        mod_version: None,
        id: pack.file,
        name: pack.title,
    };
    let mut resource = packs::list_with(home, PackKind::Resource, packs::Pictures::None);
    let order = packs::resource_order(home);
    let rank: HashMap<_, _> = order
        .iter()
        .enumerate()
        .map(|(i, file)| (file, i))
        .collect();
    resource.sort_by_key(|p| rank.get(&p.file).copied().unwrap_or(usize::MAX));
    mods.into_iter()
        .chain(resource.into_iter().map(pack))
        .collect()
}

/// A home and the builds that can affect its launch summary. These inputs come from the UI's
/// in-memory profile and installed-build lists; files are read only by the worker.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Query {
    home: PathBuf,
    version: Option<String>,
    profile: native_mods::Profile,
}

/// Just the facts about installed mods needed to draw Play and its strip. The launch and
/// consent paths continue to read live state when the player actually acts.
#[derive(Default)]
pub(crate) struct LaunchSummary {
    pub(crate) mods_off: usize,
    pub(crate) pending_mods: bool,
    /// Mods an earlier switch held off while the player had them on: Home looks for a build
    /// with releases for them too.
    pub(crate) held_on: usize,
}

#[derive(Default)]
struct Snapshot {
    tabs: Collections,
    launch: LaunchSummary,
}

fn stamp(query: &Query) -> String {
    let home = &query.home;
    let build = query.version.as_deref().and_then(|version| {
        std::fs::metadata(backend::version_dir(version).join("native-mods.json")).ok()
    });
    // The picked versions and the switch's holds change which builds the mods have releases
    // for (Home's suggestion), without touching a mod file.
    let changed = |path: PathBuf| {
        std::fs::metadata(path)
            .ok()
            .map(|m| (m.len(), m.modified().ok()))
    };
    format!(
        "{}\n{}\n{}\n{:?}\n{:?}\n{:?}",
        native_status::fingerprint(home, query.version.as_deref()),
        packs::stamp(home),
        native_mods::kept_stamp(home),
        build.map(|m| (m.len(), m.modified().ok())),
        changed(home.join("mod-browser/chosen-versions.json")),
        changed(home.join("mod-browser/active-build.json")),
    )
}

fn read_changed(query: &Query, known: &str) -> Option<(String, Snapshot)> {
    let stamp = stamp(query);
    if stamp == known {
        return None;
    }
    let items = read(&query.home);
    let active = sunrise_launcher::managed_play::Active::read_pointer(&query.home)
        .ok()
        .flatten();
    let pending_mods = !native_status::pending_for(
        &query.home,
        active.as_ref(),
        query.profile,
        query.version.as_deref(),
    )
    .is_empty();
    let held_on = active.as_ref().map_or(0, |active| {
        active.holds().filter(|hold| hold.enabled_before()).count()
    });
    let build = query
        .version
        .as_deref()
        .and_then(native_status::Build::read);
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
            tabs: items.into_iter().collect(),
            launch: LaunchSummary {
                mods_off,
                pending_mods,
                held_on,
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
    /// Changes only when Home publishes a new inventory.
    revision: u64,
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

    /// Changes each time Home publishes a new read: its mods, archives, pins or holds changed.
    pub(crate) fn revision(&self) -> u64 {
        self.revision
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
                self.revision += 1;
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
    for item in &snapshot.tabs.resources {
        if let Picture::Pack(art) = &item.picture {
            if let Thumbnail::Ready(Some(image)) = &*recover(&art.thumbnail) {
                pictures.retire(image.clone());
            }
        }
    }
}

const PICTURE: f32 = 30.;

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
            profile: instance.native_profile,
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

    /// Home reads the same row facts as Installed, once per inventory or registry change.
    pub(crate) fn home_installed_tab(&mut self, packs: bool, cx: &mut Context<Self>) -> Content {
        use crate::mods::Installed;
        let manage = if packs {
            button("home-manage-resource", tr("Manage resource packs")).on_click(cx.listener(
                |this, _, _, cx| {
                    this.open_installed(Installed::Shelf(PackKind::Resource));
                    cx.notify();
                },
            ))
        } else {
            button("home-manage-mods", tr("Manage mods")).on_click(cx.listener(|this, _, _, cx| {
                this.open_installed(Installed::Mods);
                cx.notify();
            }))
        };
        let Some(snapshot) = self.ui.home.mods.snapshot.clone() else {
            return Content {
                count: None,
                turned_off: 0,
                manage: Some(manage),
                items: div(),
            };
        };
        let list = if packs {
            &snapshot.tabs.resources
        } else {
            &snapshot.tabs.mods
        };
        let turned_off = if packs { 0 } else { snapshot.tabs.mods_off };
        if list.is_empty() {
            let items = if packs {
                tabs::empty(
                    "package-01",
                    tr("No resource packs yet"),
                    tr("Resource packs you add to this profile show up here."),
                    Some(
                        button("home-find-packs", tr("Find resource packs")).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.show_resource_pack_discover();
                                cx.notify();
                            },
                        )),
                    ),
                )
            } else if turned_off == 0 {
                tabs::empty(
                    "puzzle",
                    tr("No mods yet"),
                    tr("Mods you add to this profile show up here."),
                    Some(
                        button("home-browse-mods", tr("Find mods")).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.show_discover();
                                cx.notify();
                            },
                        )),
                    ),
                )
            } else {
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(crate::home::muted(tr("No mods turned on")))
            };
            return Content {
                count: Some(0),
                turned_off,
                manage: (turned_off > 0).then_some(manage),
                items,
            };
        }
        let mod_facts = (!packs).then(|| self.home_mod_facts(self.ui.home.mods.revision));
        let (built, place) = self.home_installed_list(packs, list.len());
        let index = (!packs).then(sunrise::v2::shown_index).flatten();
        // No catalogue read yet this run: fetch it so registry mods get their icons, as the Mods
        // pages do.
        if !packs
            && index.is_none()
            && list[built.clone()]
                .iter()
                .any(|item| matches!(item.picture, Picture::Project(_)))
        {
            self.ensure_v2_catalog(false);
        }
        let mut rows = div().flex().flex_col();
        let mut priority = list[..built.start].iter().filter(|item| item.on).count();
        for item in &list[built] {
            let picture = match &item.picture {
                Picture::Project(project) => tile(
                    index
                        .as_ref()
                        .and_then(|catalog| Some(catalog.index.project(project)?.icon.clone()))
                        .and_then(|icon| {
                            self.v2_mod_image(
                                project,
                                &icon,
                                sunrise_launcher::browser::images::ROW,
                            )
                        }),
                    &item.id,
                    &item.name,
                    PICTURE,
                ),
                Picture::Local { package, source } => {
                    let listing = if let Some(origin) = source {
                        self.read_nightly_catalogue(false);
                        self.source_listing(origin)
                    } else {
                        None
                    };
                    if let Some(source) = listing {
                        self.source_tile(
                            source,
                            None,
                            self.nightly_catalogue_current(),
                            package.clone(),
                            PICTURE,
                            cx,
                        )
                    } else {
                        tile(package.clone(), &item.id, &item.name, PICTURE)
                    }
                }
                Picture::Pack(art) => tile(art.show(cx), &item.id, &item.name, PICTURE),
            };
            let (detail, badge, state) = if packs {
                let status = if item.on {
                    priority += 1;
                    trf(
                        "Enabled · {priority}",
                        &[("priority", &priority.to_string())],
                    )
                } else {
                    tr("Off").to_owned()
                };
                (None, None, Some(status))
            } else {
                let fact = mod_facts.as_ref().and_then(|facts| facts.get(&item.id));
                let kind = fact.map(|fact| fact.kind).filter(|kind| !kind.is_empty());
                let badge = fact.and_then(|fact| fact.badge).or(item.manual_badge);
                (
                    kind.map(str::to_owned),
                    badge,
                    fact.and_then(|fact| fact.status.clone()),
                )
            };
            let open = item.open.clone();
            rows = rows.child(self.home_overview_row(
                tabs::RowDisplay {
                    id: format!("home-item:{:?}:{}", item.kind, item.id).into(),
                    picture: picture.into_any_element(),
                    title: item.name.clone(),
                    detail,
                    badge,
                    tail: state,
                },
                cx,
                move |this, _| this.open_home_item(open.clone()),
            ));
        }
        Content {
            count: Some(list.len()),
            turned_off,
            manage: Some(manage),
            items: place(rows),
        }
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
            picture: Picture::Local {
                package: None,
                source: None,
            },
            open: Open::Installed(name.into()),
            on,
            manual_badge: None,
            mod_version: (kind == Kind::Mod).then(|| "1.0.0".into()),
        }
    }

    fn names(list: &[Item]) -> Vec<&str> {
        list.iter().map(|i| i.name.as_str()).collect()
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
        assert_eq!((art.size(0).width.0, art.size(0).height.0), (60, 60));
        assert!(
            pack_thumbnail(png(4097, 1)).is_none(),
            "reject over-limit dimensions"
        );
    }

    #[test]
    fn the_mod_tab_lists_enabled_mods_and_keeps_off_packs_in_the_pack_tab() {
        let tabs: Collections = [
            item("Lanternlight", Kind::Mod, true),
            item("Trailmarks", Kind::Mod, false),
            item("Quiet Rain", Kind::Mod, true),
            item("Soft Edges", Kind::Resource, true),
            item("Old Paper", Kind::Resource, false),
        ]
        .into_iter()
        .collect();
        assert_eq!(names(&tabs.mods), ["Lanternlight", "Quiet Rain"]);
        assert_eq!(tabs.mods_off, 1);
        assert_eq!(names(&tabs.resources), ["Soft Edges", "Old Paper"]);
        assert!(!tabs.resources[1].on);
    }

    fn query(path: impl Into<PathBuf>) -> Query {
        Query {
            home: path.into(),
            version: None,
            profile: native_mods::Profile::default(),
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
                    tabs: [item("Lanternlight", Kind::Mod, true)]
                        .into_iter()
                        .collect(),
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
        assert_eq!(names(&snapshot.tabs.mods), ["Lanternlight"]);
        assert_eq!(names(&snapshot.tabs.resources), ["Soft Edges.zip"]);
        assert_eq!(snapshot.tabs.mods_off, 0);
        assert!(snapshot.tabs.resources[0].on);
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
        assert!(snapshot.tabs.mods.is_empty());
        assert_eq!(snapshot.tabs.mods_off, 1);
        assert!(!snapshot.tabs.resources[0].on);
        assert_eq!(snapshot.launch.mods_off, 0);
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn home_read_marks_manual_from_source_and_local_mods_with_badges() {
        use sunrise_launcher::github_mods::{
            install_local_packages, read_local_folder, read_local_smod,
        };
        use sunrise_launcher_smod::Package;

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("profile");
        let smod = dir.path().join("local-mod.smod");
        let bytes = Package::write_files(
            "id = 'local_mod'\nname = 'Local Mod'\nversion = '0.1.0'\napi = '0.2'\nside = 'both'\n",
            &[],
        )
        .unwrap();
        std::fs::write(&smod, &bytes).unwrap();
        let local_pkg = read_local_smod(&smod).unwrap();

        let repo = dir.path().join("repo-mod");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(
            repo.join("sunrise-mod.toml"),
            "id = 'repo_mod'\nname = 'Repo Mod'\nversion = '0.2.0'\napi = '0.2'\nside = 'both'\n",
        )
        .unwrap();
        let folder_pkgs = read_local_folder(&repo).unwrap();

        install_local_packages(&home, &[local_pkg]).unwrap();
        install_local_packages(&home, &folder_pkgs).unwrap();

        let items = read(&home);
        let local_item = items.iter().find(|i| i.id == "local_mod").unwrap();
        let repo_item = items.iter().find(|i| i.id == "repo_mod").unwrap();
        assert_eq!(
            local_item.manual_badge,
            Some(crate::mods::ManualBadge::Local)
        );
        assert_eq!(
            repo_item.manual_badge,
            Some(crate::mods::ManualBadge::FromSource)
        );
    }

    #[test]
    fn switch_held_mods_count_as_turned_off_on_home() {
        let home = profile("held");
        let extra = sunrise_launcher_smod::Package::write_files(
            "id='creature'\nname='Creature Library'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n",
            &[],
        )
        .unwrap();
        let mods_dir = home.join("sunrise-mods");
        std::fs::create_dir_all(&mods_dir).unwrap();
        std::fs::write(mods_dir.join("creature.smod"), &extra).unwrap();
        std::fs::create_dir_all(home.join("mod-browser")).unwrap();
        std::fs::write(
            home.join("mod-browser/active-build.json"),
            serde_json::json!({
                "version": 1,
                "profile": vec![1u8; 16],
                "generation": 1,
                "base": {"tag": "dev-2026-11-22-bbbbbbb", "executable": vec![2u8; 32], "runtime": vec![2u8; 32]},
                "executable": {"tag": "dev-2026-11-22-bbbbbbb", "executable": vec![2u8; 32], "runtime": vec![2u8; 32]},
                "resources": {"primary": vec![3u8; 32], "accelerator": null},
                "inputs": vec![4u8; 32],
                "receipts": [{
                    "id": vec![5u8; 16],
                    "waiting_for_update": [{
                        "id": "creature",
                        "title": "Creature Library",
                        "digest": vec![6u8; 32],
                        "root": "content",
                        "leaf": "creature.smod",
                        "source": {"kind": "local", "version": "1.0.0"},
                        "enabled_before": true,
                        "reason": {"kind": "unproved", "build": "dev-2026-11-22-bbbbbbb"}
                    }],
                    "removed": [],
                    "acknowledged": false
                }]
            })
            .to_string(),
        )
        .unwrap();
        assert!(
            sunrise_launcher::managed_play::Active::read_pointer(&home)
                .unwrap()
                .is_some(),
            "fixture pointer must parse"
        );
        let tabs: Collections = read(&home).into_iter().collect();
        assert_eq!(names(&tabs.mods), ["Lanternlight"]);
        assert_eq!(tabs.mods_off, 1);
        std::fs::remove_dir_all(&home).unwrap();
    }
}
