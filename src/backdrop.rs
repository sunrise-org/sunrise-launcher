//! The window's still backdrop. It never owns a timer. The cat and visible operations wake
//! their own UI when they change; reduced motion does not need to stop a scenery animation.
use crate::paint::HomePhoto;
use crate::{backend, theme::Theme};
use gpui::{div, img, prelude::*, Div, ObjectFit, RenderImage, Window};
use std::sync::Arc;
use sunrise_launcher::themes;
use sunrise_launcher_scenery::{coast, sky::Sky};

/// How much of the page colour lies over a picture behind the pages.
const PICTURE_WASH: f32 = 0.12;

#[derive(Default)]
pub(super) struct Backdrop {
    coast: Option<Arc<RenderImage>>,
    sky: Sky,
}

impl Backdrop {
    pub(super) fn render(
        &mut self,
        settings: &backend::Settings,
        theme: &Theme,
        photo: Option<HomePhoto>,
        window: &mut Window,
    ) -> Div {
        let page: gpui::Hsla = gpui::rgb(crate::paint::palette().background).into();
        let back = div().absolute().inset_0().overflow_hidden().bg(page);
        if !settings.scenery() {
            return back;
        }
        if let Some(photo) = photo {
            let picture = img(photo.image);
            // Only a thin wash of the page ties the picture to the theme. Home shades
            // the scene separately under the profile so it can sit directly on the picture.
            return back
                .child(picture.size_full().object_fit(ObjectFit::Cover))
                .child(div().absolute().inset_0().bg(page.opacity(PICTURE_WASH)));
        }
        // A player's theme, there or missing, stands in Dawn's scenery.
        let resolved = themes::resolve(settings.theme.as_deref());
        let own = matches!(resolved, themes::Resolved::Local(_));
        let scene = match resolved {
            // Sunrise Light shares Sunrise's scene.
            themes::Resolved::Builtin(t) if t.is_sunrise() => Some(themes::DAWN_THEME.id),
            themes::Resolved::Builtin(t) => Some(t.id),
            themes::Resolved::Local(_) | themes::Resolved::Missing => Some(themes::DAWN_THEME.id),
            themes::Resolved::Twilight => None,
        };
        match scene {
            Some(id) if id == themes::DAWN_THEME.id => {
                let dawn = back.child(img("dawn.webp").size_full().object_fit(ObjectFit::Cover));
                if own {
                    // Dawn's scene was painted for Dawn's inks. Under a player's colours it is
                    // washed with their page, as a picture is, so their unplated titles read
                    // whether the theme is dark or light.
                    dawn.child(div().absolute().inset_0().bg(page.opacity(0.78)))
                } else {
                    dawn
                }
            }
            None => back.child(
                img(self.coast.get_or_insert_with(coast::poster).clone())
                    .size_full()
                    .object_fit(ObjectFit::Cover),
            ),
            Some(_) => {
                self.sky.free_replaced(window);
                let size = window.viewport_size();
                back.child(self.sky.frame_at(
                    &theme.night,
                    (size.width.into(), size.height.into()),
                    window.scale_factor(),
                    Default::default(),
                    0.,
                ))
                .child(div().absolute().inset_0().bg(page.opacity(0.46)))
            }
        }
    }
}
