//! The Mods page's project page, search filters and project icons (`sunrise_launcher::browser`:
//! `details` for the data, `images` for the icons and gallery). Drawn only with the launcher's
//! shared components, so a restyle of those restyles this.
//!
//! - A result's title opens its page inside the launcher: icon, author, downloads, followers,
//!   licence, how well it runs and Install, then Description (Markdown), Gallery, Versions and
//!   the project's links.
//! - Filters: sort, game version and categories, with the values the
//!   source lists for the kind.
//! - Icons and gallery images come from the source's image servers at run time, cached on disk;
//!   a placeholder stands in until one arrives.
use super::mods_page::Event;
use super::*;

use std::collections::HashMap;
use std::path::PathBuf;

use std::sync::Arc;
use sunrise_launcher::browser::{self, details::Details, images, Project, SourceId, Version};

/// The open project page.
pub(super) struct Opened {
    pub(super) project: Project,
    details: Option<Result<Arc<Details>, String>>,
    versions: Option<Result<Arc<[Version]>, String>>,
    version_list: cull::Cull,
}

/// The project page, filters and images' share of the Mods page's state.
#[derive(Default)]
pub(super) struct Browse {
    pub(super) open: Option<Opened>,
    /// Images on disk, by URL and size; `None` while they load or when they failed.
    images: HashMap<(String, u32), Option<PathBuf>>,
    image_queue: Option<std::sync::mpsc::Sender<(String, u32)>>,
}

/// What the background sends this module.
#[allow(dead_code)]
pub(super) enum Loaded {
    Details((SourceId, String), Result<Arc<Details>, String>),
    Versions((SourceId, String), Result<Vec<Version>, String>),
    Image((String, u32), Option<PathBuf>),
}

/// Images fetched at once.
const IMAGE_WORKERS: usize = 4;

impl Launcher {
    pub(super) fn mods_loaded(&mut self, loaded: Loaded) {
        let b = &mut self.mods.browse;
        fn open<'a>(b: &'a mut Browse, key: &(SourceId, String)) -> Option<&'a mut Opened> {
            b.open
                .as_mut()
                .filter(|o| (o.project.source, &o.project.id) == (key.0, &key.1))
        }
        match loaded {
            Loaded::Details(key, d) => {
                if let Some(o) = open(b, &key) {
                    o.details = Some(d);
                }
            }
            Loaded::Versions(key, v) => {
                if let Some(o) = open(b, &key) {
                    o.versions = Some(v.map(Arc::from));
                    o.version_list.reset();
                }
            }
            Loaded::Image(key, path) => {
                // A refresh that failed (offline) keeps the cached image.
                if path.is_some() || !matches!(b.images.get(&key), Some(Some(_))) {
                    b.images.insert(key, path);
                }
            }
        }
    }

    /// Opens a project's page and asks its source for the rest of it. Nothing opens one now; the
    /// Modrinth and Java removal deletes it with the state around it.
    #[allow(dead_code)]
    pub(super) fn mods_open(&mut self, project: Project) {
        let key = (project.source, project.id.clone());
        let tx = self.mods.tx.clone();
        let p = project.clone();
        std::thread::spawn(move || {
            let source = browser::source(p.source);
            let d = source.details(&p).map(Arc::new);
            let _ = tx.send(Event::Loaded(Loaded::Details(key.clone(), d)));
            let v = source.versions(&p);
            let _ = tx.send(Event::Loaded(Loaded::Versions(key, v)));
        });
        self.mods.browse.open = Some(Opened {
            project,
            details: None,
            versions: None,
            version_list: cull::Cull::default(),
        });
    }

    /// The image of `url` at `size` when it is on disk; else asks for it once. One cached longer
    /// than `images::FRESH` is shown and asked for again, with its validators.
    pub(super) fn mods_image(&mut self, url: &str, size: u32) -> Option<PathBuf> {
        let key = (url.to_owned(), size);
        if let Some(state) = self.mods.browse.images.get(&key) {
            return state.clone();
        }
        // The launcher's data folder, shared by every instance.
        let home = backend::data_dir();
        if let Some(p) = images::cached(&home, url, size) {
            self.mods.browse.images.insert(key.clone(), Some(p.clone()));
            if images::stale(&home, url, size) {
                let _ = self.mods_image_queue().send(key);
            }
            return Some(p);
        }
        self.mods.browse.images.insert(key.clone(), None);
        if images::refuse(url).is_some() {
            return None;
        }
        let _ = self.mods_image_queue().send(key);
        None
    }

    /// The image workers' queue, started the first time an image is asked for.
    fn mods_image_queue(&mut self) -> std::sync::mpsc::Sender<(String, u32)> {
        let tx = self.mods.tx.clone();
        self.mods
            .browse
            .image_queue
            .get_or_insert_with(|| {
                let (send, receive) = std::sync::mpsc::channel::<(String, u32)>();
                let receive = std::sync::Arc::new(std::sync::Mutex::new(receive));
                let home = backend::data_dir();
                for _ in 0..IMAGE_WORKERS {
                    let (receive, tx, home) = (receive.clone(), tx.clone(), home.clone());
                    std::thread::spawn(move || loop {
                        let Ok(next) = receive.lock().map(|r| r.recv()) else {
                            break;
                        };
                        let Ok((url, size)) = next else {
                            break;
                        };
                        let path = images::fetch(&home, &url, size).ok();
                        if tx
                            .send(Event::Loaded(Loaded::Image((url, size), path)))
                            .is_err()
                        {
                            break;
                        }
                    });
                }
                send
            })
            .clone()
    }
}
