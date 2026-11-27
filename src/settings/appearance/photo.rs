//! Settings › Appearance: what stands behind Home — the player's own picture or their
//! theme's — its tone, and its decode off the window's thread.
use super::*;
use crate::paint::HomePhoto;
use gpui::RenderImage;
use std::sync::Arc;
use sunrise_launcher::appearance::{self, Behind};
use sunrise_launcher::theme_files;
use sunrise_launcher::themes;

/// Sample the centres of a grid bounded to 32 × 32, independent of image size. Weight sRGB
/// channels by coverage, so transparent pixels (which may carry arbitrary RGB) do not tint us.
pub(super) fn photo_tone(image: &image::RgbaImage) -> Option<u32> {
    let (w, h) = image.dimensions();
    let (cols, rows) = (w.min(32), h.min(32));
    let mut sum = [0_u32; 3];
    let mut weight = 0;
    let mut strongest = (0, 0);
    for y in 0..rows {
        for x in 0..cols {
            let [r, g, b, a] =
                image[((2 * x + 1) * w / (2 * cols), (2 * y + 1) * h / (2 * rows))].0;
            let a = u32::from(a);
            weight += a;
            let chroma = a * u32::from(r.max(g).max(b) - r.min(g).min(b));
            if chroma > strongest.0 {
                strongest = (chroma, u32::from_be_bytes([0, r, g, b]));
            }
            for (total, channel) in sum.iter_mut().zip([r, g, b]) {
                *total += u32::from(channel) * a;
            }
        }
    }
    (weight > 0).then(|| {
        let [r, g, b] = sum.map(|c| (c + weight / 2) / weight);
        // A colourful picture can average to gray if its hues cancel. In that case use
        // the most visibly chromatic sample instead of losing its colour altogether.
        if r == g && g == b && strongest.0 > 0 {
            strongest.1
        } else {
            r << 16 | g << 8 | b
        }
    })
}

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
            .map(|still| {
                let rgba = theme_files::fitted(still, sunrise_launcher::pictures::gpu_limit());
                let tone = photo_tone(&rgba);
                (rgba, tone)
            })
        });
        cx.spawn(async move |this, cx| {
            let decoded = task.await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.ui.settings.appearance;
                state.decoding = false;
                let picture = match decoded {
                    Ok((mut rgba, tone)) => {
                        // The window draws BGRA, its alpha premultiplied.
                        for px in rgba.pixels_mut() {
                            let a = u16::from(px.0[3]);
                            let [r, g, b] = [0, 1, 2].map(|i| (u16::from(px.0[i]) * a / 255) as u8);
                            px.0 = [b, g, r, a as u8];
                        }
                        Some(HomePhoto {
                            image: Arc::new(RenderImage::new([image::Frame::new(rgba)])),
                            tone,
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
