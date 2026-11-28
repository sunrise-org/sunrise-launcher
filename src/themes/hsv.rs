//! The maths of a colour picker, with nothing drawn: a colour as hue, saturation and value, the
//! way back to `0xRRGGBB`, and where a pointer on a bar is as a share of it.

/// A colour as the picker moves it: a hue in degrees (0 up to 360), and saturation and value
/// from 0 to 1. A grey keeps its hue here, which an `0xRRGGBB` can't.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsv {
    pub h: f32,
    pub s: f32,
    pub v: f32,
}

impl Hsv {
    /// The same colour as `rgb` (`0xRRGGBB`). A grey has hue 0.
    pub fn from_rgb(rgb: u32) -> Hsv {
        let channel = |shift: u32| (rgb >> shift & 0xff) as f32 / 255.;
        let (r, g, b) = (channel(16), channel(8), channel(0));
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let spread = max - min;
        let h = if spread <= f32::EPSILON {
            0.
        } else if max == r {
            60. * ((g - b) / spread).rem_euclid(6.)
        } else if max == g {
            60. * ((b - r) / spread + 2.)
        } else {
            60. * ((r - g) / spread + 4.)
        };
        let s = if max <= f32::EPSILON { 0. } else { spread / max };
        Hsv { h, s, v: max }
    }

    /// This colour as `0xRRGGBB`. Out-of-range parts are taken as the nearest in range.
    pub fn to_rgb(self) -> u32 {
        let s = self.s.clamp(0., 1.);
        let v = self.v.clamp(0., 1.);
        let sector = self.h.rem_euclid(360.) / 60.;
        let chroma = v * s;
        let x = chroma * (1. - (sector % 2. - 1.).abs());
        let (r, g, b) = match sector as u32 {
            0 => (chroma, x, 0.),
            1 => (x, chroma, 0.),
            2 => (0., chroma, x),
            3 => (0., x, chroma),
            4 => (x, 0., chroma),
            _ => (chroma, 0., x),
        };
        let m = v - chroma;
        let byte = |part: f32| ((part + m) * 255.).round().clamp(0., 255.) as u32;
        byte(r) << 16 | byte(g) << 8 | byte(b)
    }

    /// The fully saturated, full-value colour of hue `h`: what the picker's square is filled
    /// with, and a stop of its hue bar.
    pub fn pure(h: f32) -> u32 {
        Hsv { h, s: 1., v: 1. }.to_rgb()
    }
}

/// Where `pos` is on a bar that starts at `start` and is `len` long, from 0 (at its start) to 1
/// (at its end), held to that range.
pub fn fraction(pos: f32, start: f32, len: f32) -> f32 {
    if len <= 0. {
        return 0.;
    }
    ((pos - start) / len).clamp(0., 1.)
}

/// The colour a pointer at (`x`, `y`) picks on a saturation and value square whose top-left is
/// (`left`, `top`): saturation grows to the right, value falls going down. `hue` is kept.
pub fn square_pick(
    hue: f32,
    (x, y): (f32, f32),
    (left, top): (f32, f32),
    (width, height): (f32, f32),
) -> Hsv {
    Hsv {
        h: hue,
        s: fraction(x, left, width),
        v: 1. - fraction(y, top, height),
    }
}

/// The hue (0 up to 360) a pointer at `x` picks on a hue bar that starts at `left` and is
/// `width` long. The bar's far end is held just short of 360, which is red again.
pub fn hue_pick(x: f32, left: f32, width: f32) -> f32 {
    (fraction(x, left, width) * 360.).min(359.99)
}
