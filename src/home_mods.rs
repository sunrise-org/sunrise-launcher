//! The selected profile’s Mods and Resource Packs overviews and the Play bar’s launch facts.
//!
//! What the profile has is read in the background ([`read`]) and kept ([`Inventory`]): drawing
//! Home opens no file of a mod or a pack. It is read again when the files it was read from
//! change ([`stamp`]).
use super::*;
use crate::frame::Page;
use crate::home::tabs::{self, Content};
use crate::mod_icons::tile;
use crate::widgets::button::secondary;
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
    /// The mods that are on: what the launch summary and the "enabled" count read.
    mods: Vec<Item>,
    mods_off: usize,
    /// Every mod, on or off, by name: the Mods tab lists them all, each with its switch.
    all_mods: Vec<Item>,
    resources: Vec<Item>,
}

impl FromIterator<Item> for Collections {
    fn from_iter<T: IntoIterator<Item = Item>>(items: T) -> Self {
        let mut tabs = Self::default();
        for item in items {
            match item.kind {
                Kind::Mod if item.on => {
                    tabs.all_mods.push(item.clone());
                    tabs.mods.push(item);
                }
                Kind::Mod => {
                    tabs.all_mods.push(item);
                    tabs.mods_off += 1;
                }
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
}

/// Just the facts about installed mods needed to draw Play and its strip. The launch and
/// consent paths continue to read live state when the player actually acts.
#[derive(Default)]
pub(crate) struct LaunchSummary {
    pub(crate) mods_off: usize,
    pub(crate) pending_mods: bool,
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
    let active = sunrise_launcher::managed_play::Active::read_pointer(&query.home)
        .ok()
        .flatten();
    let pending_mods = !native_status::pending(&query.home, active.as_ref()).is_empty();
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
    /// A pack was switched on Home: the next published list keeps the packs' places, so the
    /// card flipped doesn't move to another row.
    keep_order: bool,
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
            Some((stamp, mut snapshot)) => {
                if std::mem::take(&mut self.keep_order) {
                    if let Some(old) = &self.snapshot {
                        keep_order(&old.tabs.resources, &mut snapshot.tabs.resources);
                    }
                }
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

/// Puts `new` in the order of `old` when both hold the same packs.
fn keep_order(old: &[Item], new: &mut [Item]) {
    if old.len() != new.len() {
        return;
    }
    let place: HashMap<&str, usize> = old
        .iter()
        .enumerate()
        .map(|(at, item)| (item.id.as_str(), at))
        .collect();
    if new.iter().any(|item| !place.contains_key(item.id.as_str())) {
        return;
    }
    new.sort_by_key(|item| place[item.id.as_str()]);
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

/// A card's picture, as large as a world's.
const PICTURE: f32 = 40.;

mod add;
mod cards;
mod menu;
mod view;

#[cfg(test)]
mod tests;
