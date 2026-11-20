//! Skin and cape pictures as the Skins tab's player model wears them: decoded, completed and laid
//! out the way the game does it. The model itself is drawn by `sunrise_launcher_scenery::model`.

pub use sunrise_launcher_scenery::model::{render, Pose, Texture};

/// A skin picture as the model wears it: 64 by 64 (or a multiple), the old 64 by 32 skins given
/// their left arm and leg the way the game gives them (`processLegacySkin`). None for any other
/// shape or something that isn't a PNG.
pub fn skin(png: &[u8]) -> Option<Texture> {
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .ok()?
        .to_rgba8();
    let (w, h) = (image.width() as usize, image.height() as usize);
    if w == 0 || w % 64 != 0 || w > 1024 || (h != w && h * 2 != w) {
        return None;
    }
    let mut texels = vec![[0; 4]; w * w];
    for (i, p) in image.pixels().enumerate() {
        texels[i] = p.0;
    }
    if h * 2 == w {
        let k = (w / 64) as i64;
        // (from x, from y, to x offset, to y offset, width, height): the right arm and leg,
        // mirrored into the left ones' places, as vanilla's `processLegacySkin`.
        const RECTS: [(i64, i64, i64, i64, i64, i64); 12] = [
            (4, 16, 16, 32, 4, 4),
            (8, 16, 16, 32, 4, 4),
            (0, 20, 24, 32, 4, 12),
            (4, 20, 16, 32, 4, 12),
            (8, 20, 8, 32, 4, 12),
            (12, 20, 16, 32, 4, 12),
            (44, 16, -8, 32, 4, 4),
            (48, 16, -8, 32, 4, 4),
            (40, 20, 0, 32, 4, 12),
            (44, 20, -8, 32, 4, 12),
            (48, 20, -16, 32, 4, 12),
            (52, 20, -8, 32, 4, 12),
        ];
        let at = |x: i64, y: i64| (y * w as i64 + x) as usize;
        for (sx, sy, ox, oy, rw, rh) in RECTS {
            let (sx, sy, ox, oy, rw, rh) = (sx * k, sy * k, ox * k, oy * k, rw * k, rh * k);
            for y in 0..rh {
                for x in 0..rw {
                    texels[at(sx + ox + rw - 1 - x, sy + oy + y)] = texels[at(sx + x, sy + y)];
                }
            }
        }
        let k = k as usize;
        let area = |x0: usize, y0: usize, x1: usize, y1: usize| {
            (y0 * k..y1 * k).flat_map(move |y| (x0 * k..x1 * k).map(move |x| y * w + x))
        };
        // The base layer is opaque (`setNoAlpha`), and a hat with no clear texel at all is an
        // old skin's filler, not a hat: it is cleared (`doNotchTransparencyHack`).
        for i in area(0, 0, 32, 16)
            .chain(area(0, 16, 64, 32))
            .chain(area(16, 48, 48, 64))
        {
            texels[i][3] = 255;
        }
        if area(32, 0, 64, 16).all(|i| texels[i][3] >= 128) {
            for i in area(32, 0, 64, 16) {
                texels[i] = [0; 4];
            }
        }
    }
    Some(Texture { texels, w, h: w })
}

/// A cape picture on the smallest 64·2ⁿ by 32·2ⁿ canvas it fits, as OptiFine lays its own out:
/// Mojang's 64 by 32 capes, OptiFine's 46 by 22 and 92 by 44, and the old 22 by 17 ones all keep
/// the 64 by 32 layout's coordinates.
pub fn cape(png: &[u8]) -> Option<Texture> {
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .ok()?
        .to_rgba8();
    let (w, h) = (image.width() as usize, image.height() as usize);
    if w == 0 || h == 0 || w > 2048 || h > 1024 {
        return None;
    }
    let mut k = 1;
    while 64 * k < w || 32 * k < h {
        k *= 2;
    }
    // The outer face must be inside the picture.
    if 11 * k > w || 17 * k > h {
        return None;
    }
    let (cw, ch) = (64 * k, 32 * k);
    let mut texels = vec![[0; 4]; cw * ch];
    for (x, y, p) in image.enumerate_pixels() {
        texels[y as usize * cw + x as usize] = p.0;
    }
    Some(Texture {
        texels,
        w: cw,
        h: ch,
    })
}

/// The cape's outer face (10 by 16 of the 64 by 32 layout, at 1, 1), as a player behind it sees
/// it: what a cape's tile shows.
pub fn cape_front(cape: &Texture) -> Texture {
    let k = cape.w / 64;
    let (w, h) = (10 * k, 16 * k);
    let texels = (k..k + h)
        .flat_map(|y| (k..k + w).map(move |x| (x, y)))
        .map(|(x, y)| cape.texel(x, y))
        .collect();
    Texture { texels, w, h }
}

/// Whether a skin is made for slim arms: its right arm's fourth column of front texels is clear,
/// as slim skins leave it.
pub fn looks_slim(skin: &Texture) -> bool {
    let k = skin.w / 64;
    (20 * k..32 * k).all(|y| skin.texel(54 * k, y)[3] == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let image = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba(f(x, y)));
        let mut out = std::io::Cursor::new(Vec::new());
        image.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    /// A skin whose every base texel says which part it belongs to: red for the head, green for
    /// the body, blue for the arms and legs; its outer layer clear.
    fn marked() -> Texture {
        skin(&png(64, 64, |x, y| {
            let outer =
                y < 16 && x >= 32 || (32..48).contains(&y) || y >= 48 && !(16..48).contains(&x);
            if outer {
                [0, 0, 0, 0]
            } else if y < 16 {
                [255, 0, 0, 255]
            } else if (16..32).contains(&y) && (16..40).contains(&x) {
                [0, 255, 0, 255]
            } else {
                [0, 0, 255, 255]
            }
        }))
        .unwrap()
    }

    fn colour_at(out: &[[u8; 4]], w: usize, x: usize, y: usize) -> [u8; 4] {
        out[y * w + x]
    }

    #[test]
    fn the_model_stands_head_up_and_centred() {
        let skin = marked();
        let pose = Pose {
            skin: &skin,
            slim: false,
            cape: None,
            yaw: 0.,
            pitch: 0.,
        };
        let (w, h) = (120, 190);
        let out = render(&pose, w, h);
        let head = colour_at(&out, w, w / 2, h / 2 - 60);
        let body = colour_at(&out, w, w / 2, h / 2 - 15);
        assert!(
            head[0] > 100 && head[1] == 0,
            "the head is up top: {head:?}"
        );
        assert!(body[1] > 100 && body[0] == 0, "the body under it: {body:?}");
        assert_eq!(colour_at(&out, w, 2, 2)[3], 0, "clear around the model");
    }

    #[test]
    fn a_turned_model_shows_its_back_and_the_cape_on_it() {
        let skin = marked();
        let cape = cape(&png(64, 32, |_, _| [250, 250, 0, 255])).unwrap();
        let (w, h) = (120, 190);
        let pose = |yaw: f32, cape: Option<&Texture>| {
            render(
                &Pose {
                    skin: &skin,
                    slim: false,
                    cape,
                    yaw,
                    pitch: 0.,
                },
                w,
                h,
            )
        };
        let front = pose(0., Some(&cape));
        let at_back = colour_at(&front, w, w / 2, h / 2 - 15);
        assert!(
            at_back[1] > 100 && at_back[2] == 0,
            "the body hides the cape from the front"
        );
        let back = pose(std::f32::consts::PI, Some(&cape));
        let caped = colour_at(&back, w, w / 2, h / 2 - 15);
        assert!(
            caped[0] > 100 && caped[1] > 100,
            "the cape covers the back: {caped:?}"
        );
        let bare = pose(std::f32::consts::PI, None);
        let body = colour_at(&bare, w, w / 2, h / 2 - 15);
        assert!(
            body[1] > 100 && body[0] == 0,
            "no cape, the body's back: {body:?}"
        );
    }

    #[test]
    fn legacy_skins_get_their_left_limbs_and_others_are_refused() {
        let legacy = skin(&png(64, 32, |x, y| [x as u8, y as u8, 9, 255])).unwrap();
        assert_eq!((legacy.w, legacy.h), (64, 64));
        // The left leg's front (20, 52) mirrors the right leg's front (4..8, 20..32).
        assert_eq!(legacy.texel(20, 52), [7, 20, 9, 255]);
        assert!(skin(&png(64, 48, |_, _| [0; 4])).is_none());
        assert!(skin(b"not a png").is_none());
        assert_eq!(skin(&png(128, 128, |_, _| [1, 2, 3, 255])).unwrap().w, 128);
    }

    #[test]
    fn capes_keep_their_layout_on_their_canvas() {
        for (w, h, k) in [
            (64, 32, 1),
            (46, 22, 1),
            (22, 17, 1),
            (92, 44, 2),
            (128, 64, 2),
        ] {
            let laid = cape(&png(w, h, |x, y| [x as u8, y as u8, 7, 255])).unwrap();
            assert_eq!((laid.w, laid.h), (64 * k, 32 * k), "{w}x{h}");
            let front = cape_front(&laid);
            assert_eq!((front.w, front.h), (10 * k, 16 * k));
            assert_eq!(front.texels[0], [k as u8, k as u8, 7, 255], "{w}x{h}");
        }
        assert!(cape(&png(8, 8, |_, _| [0; 4])).is_none());
    }

    #[test]
    fn slim_skins_are_told_by_their_clear_arm_column() {
        let classic = marked();
        assert!(!looks_slim(&classic));
        let slim = skin(&png(64, 64, |x, y| {
            if x == 54 && (20..32).contains(&y) {
                [0; 4]
            } else {
                [9, 9, 9, 255]
            }
        }))
        .unwrap();
        assert!(looks_slim(&slim));
    }
}
