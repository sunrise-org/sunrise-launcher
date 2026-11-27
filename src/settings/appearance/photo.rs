//! Settings › Appearance: what stands behind Home — the player's own picture or their
//! theme's — and its decode off the window's thread.
use super::*;
use crate::paint::HomePhoto;
use gpui::RenderImage;
use std::sync::Arc;
use sunrise_launcher::appearance::{self, Behind};
use sunrise_launcher::theme_files;
use sunrise_launcher::themes;

impl Launcher {
    /// What stands behind Home, for the frame's backdrop: the player's picture, else their
    /// theme's once it is decoded. None is the theme's scene (or nothing, with scenery off).
    /// Worked out when the settings it depends on change, not every frame.
    pub(crate) fn home_photo(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<HomePhoto> {
        let key = (
            self.settings.theme.clone(),
            self.settings.background.clone(),
            self.settings.scenery(),
        );
        let state = &mut self.ui.settings.appearance;
        for old in state.retired.drain(..) {
            let _ = window.drop_image(old);
        }
        if state.behind.as_ref().is_none_or(|(old, _)| *old != key) {
            let behind = match appearance::behind_home(&self.settings, &backend::data_dir()) {
                Behind::Personal(path) if state.unshown.as_ref() == Some(&path) => {
                    appearance::behind_without_own(&self.settings)
                }
                behind => behind,
            };
            state.behind = Some((key, behind));
        }
        let (path, own) = match state.behind.as_ref().map(|(_, behind)| behind.clone())? {
            Behind::Plain | Behind::Scene => return None,
            Behind::Personal(path) => (path, true),
            Behind::Theme(path) => (path, false),
        };
        if let Some((_, picture)) = state.photo.as_ref().filter(|(of, _)| *of == path) {
            return picture.clone();
        }
        if state.decoding {
            return None;
        }
        state.decoding = true;
        let read = path.clone();
        let task = cx.background_executor().spawn(async move {
            if own {
                let name = read
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                themes::background_picture(&backend::data_dir(), name)
            } else {
                theme_files::picture(&read).map_err(|reason| reason.text())
            }
            .map(|still| theme_files::fitted(still, sunrise_launcher::pictures::gpu_limit()))
        });
        cx.spawn(async move |this, cx| {
            let decoded = task.await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.ui.settings.appearance;
                state.decoding = false;
                let picture = match decoded {
                    Ok(mut rgba) => {
                        // The window draws BGRA, its alpha premultiplied.
                        for px in rgba.pixels_mut() {
                            let a = u16::from(px.0[3]);
                            let [r, g, b] = [0, 1, 2].map(|i| (u16::from(px.0[i]) * a / 255) as u8);
                            px.0 = [b, g, r, a as u8];
                        }
                        Some(HomePhoto {
                            image: Arc::new(RenderImage::new([image::Frame::new(rgba)])),
                        })
                    }
                    Err(said) => {
                        state.error = Some(said);
                        if own {
                            // A kept copy that can't be decoded is never claimed as shown:
                            // Home goes on to the theme's picture, or the scene.
                            state.unshown = Some(path.clone());
                            if let Some((_, behind)) = &mut state.behind {
                                *behind = appearance::behind_without_own(&this.settings);
                            }
                        }
                        None
                    }
                };
                if let Some((_, Some(old))) = state.photo.replace((path, picture)) {
                    state.retired.push(old.image);
                }
                cx.notify();
            });
        })
        .detach();
        None
    }
}
