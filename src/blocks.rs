//! The blocky look's parts (`docs/design-system.md`, "Blocks"): pixel type, pixel icons, panels
//! with a dark outline, a light bevel on top and a darker lip below, sunk wells, badges, the
//! inventory tabs, and the slots that hold icons downloaded at run time.
use crate::theme;
use gpui::{div, prelude::*, px, svg, AnyElement, Hsla, SharedString, Svg};
use std::borrow::Cow;

/// The icons, served to `svg()` as `icons/<name>.svg`.
pub struct Assets;

impl gpui::AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        if path == "dawn.webp" {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/media/dawn.webp"
            ))));
        }
        if let Some(name) = path
            .strip_prefix("icons/glass/")
            .and_then(|p| p.strip_suffix(".svg"))
        {
            return Ok(sunrise_launcher_theme::glass_icons::get(name).map(Cow::Borrowed));
        }
        let name = path
            .strip_prefix("icons/")
            .and_then(|p| p.strip_suffix(".svg"));
        Ok(name
            .and_then(sunrise_launcher_theme::icons::get)
            .map(Cow::Borrowed))
    }

    fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(sunrise_launcher_theme::icons::ALL
            .iter()
            .map(|(name, _)| format!("icons/{name}.svg").into())
            .chain(
                sunrise_launcher_theme::glass_icons::ALL
                    .iter()
                    .map(|(name, _)| format!("icons/glass/{name}.svg").into()),
            )
            .collect())
    }
}

/// A pixel icon (`sunrise_launcher_theme::icons`) in `color`.
pub fn icon(name: &'static str, size: f32, color: Hsla) -> Svg {
    let glass = match name {
        "home" => "dashboard-square-01",
        "package" | "box" | "archive" => "package-01",
        "puzzle" => "puzzle",
        "server" => "computer",
        "sliders" => "sliders-horizontal",
        "play" => "play",
        "chevron-down" => "arrow-down-01",
        "chevron-right" => "arrow-right-01",
        "search" => "search-01",
        "check" => "tick-02",
        "user" => "user",
        "shirt" => "t-shirt",
        "map" => "map",
        "plus" => "plus-sign",
        "globe" => "earth",
        "message" => "chat",
        "folder" => "folder-01",
        "close" => "cancel-01",
        _ => {
            return svg()
                .path(format!("icons/{name}.svg"))
                .flex_none()
                .size(px(size))
                .text_color(color)
        }
    };
    svg()
        .path(format!("icons/glass/{glass}.svg"))
        .flex_none()
        .size(px(size))
        .text_color(color)
}

/// The light edge along the top of a raised block and the darker lip along its bottom; the
/// block must be `relative`.
pub fn edges(top: Hsla, bottom: Hsla, lip: f32) -> [AnyElement; 2] {
    [
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(theme::BEVEL))
            .bg(top)
            .into_any_element(),
        div()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .h(px(lip))
            .bg(bottom)
            .into_any_element(),
    ]
}
