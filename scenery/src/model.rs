//! The player model drawn in software, for the launcher's Skins tab: Minecraft's player model
//! (head, body, arms and legs, their outer layer, and the cape), so the launcher needs no 3D view
//! of its own. Each part is a box whose six faces take the skin's standard box layout; the boxes
//! are turned by a yaw and a pitch, drawn with a depth buffer, nearest texel, and a light from the
//! front so their sides read. It lives here, with the other pixel loops, to be optimised in debug
//! builds too: unoptimised, one 560 by 840 frame took 65 ms, too slow to turn smoothly.
//!
//! Model units are skin pixels: the feet at y 0, the top of the head at y 32, the player facing
//! +Z (toward the viewer at yaw 0), their right side at −X.

/// A picture as RGBA texels, row by row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Texture {
    pub texels: Vec<[u8; 4]>,
    pub w: usize,
    pub h: usize,
}

impl Texture {
    /// The texel at `x`, `y`, the nearest edge one outside the picture.
    pub fn texel(&self, x: usize, y: usize) -> [u8; 4] {
        self.texels[y.min(self.h - 1) * self.w + x.min(self.w - 1)]
    }
}

/// What the model wears and how it is turned.
pub struct Pose<'a> {
    pub skin: &'a Texture,
    /// Slim (3 pixel) arms instead of classic (4 pixel) ones.
    pub slim: bool,
    pub cape: Option<&'a Texture>,
    /// Turned about the vertical, in radians: 0 faces the viewer.
    pub yaw: f32,
    /// Tipped toward the viewer, in radians.
    pub pitch: f32,
}

type V3 = [f32; 3];
/// A part: its box's corner and size, its base and outer layers' texels, and how much the outer
/// layer grows.
type Part = (V3, V3, (f32, f32), (f32, f32), f32);

#[derive(Clone, Copy, PartialEq)]
enum Sheet {
    Skin,
    Cape,
}

/// One face: its corners top left, top right, bottom right, bottom left (as its texels lie), the
/// texels it takes (in 64 by 64 layout units), and its outward normal.
struct Quad {
    corners: [V3; 4],
    uv: [f32; 4],
    normal: V3,
    sheet: Sheet,
}

/// A box from `min` of `size`, its faces from the box layout at `uv`, grown by `grow` on every
/// side (an outer layer) without changing its texels.
fn cuboid(min: V3, size: V3, uv: (f32, f32), grow: f32, sheet: Sheet) -> Vec<Quad> {
    let [w, h, d] = size;
    let (u, v) = uv;
    let [x0, y0, z0] = [min[0] - grow, min[1] - grow, min[2] - grow];
    let [x1, y1, z1] = [min[0] + w + grow, min[1] + h + grow, min[2] + d + grow];
    let quad = |corners: [V3; 4], uv: [f32; 4], normal: V3| Quad {
        corners,
        uv,
        normal,
        sheet,
    };
    vec![
        // Front, facing +Z.
        quad(
            [[x0, y1, z1], [x1, y1, z1], [x1, y0, z1], [x0, y0, z1]],
            [u + d, v + d, u + d + w, v + d + h],
            [0., 0., 1.],
        ),
        // Back: seen from behind, its texels run toward −X.
        quad(
            [[x1, y1, z0], [x0, y1, z0], [x0, y0, z0], [x1, y0, z0]],
            [u + 2. * d + w, v + d, u + 2. * d + 2. * w, v + d + h],
            [0., 0., -1.],
        ),
        // The model's right side (−X): its texels run from the back to the front.
        quad(
            [[x0, y1, z0], [x0, y1, z1], [x0, y0, z1], [x0, y0, z0]],
            [u, v + d, u + d, v + d + h],
            [-1., 0., 0.],
        ),
        // Its left side (+X): from the front to the back.
        quad(
            [[x1, y1, z1], [x1, y1, z0], [x1, y0, z0], [x1, y0, z1]],
            [u + d + w, v + d, u + 2. * d + w, v + d + h],
            [1., 0., 0.],
        ),
        // Top: its far edge at the back.
        quad(
            [[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]],
            [u + d, v, u + d + w, v + d],
            [0., 1., 0.],
        ),
        // Bottom.
        quad(
            [[x0, y0, z1], [x1, y0, z1], [x1, y0, z0], [x0, y0, z0]],
            [u + d + w, v, u + d + 2. * w, v + d],
            [0., -1., 0.],
        ),
    ]
}

/// Every face of the model in `pose`, the cape's included.
fn faces(slim: bool, cape: bool) -> Vec<Quad> {
    let arm = if slim { 3. } else { 4. };
    let mut all = Vec::new();
    // (min, size, base layer's texels, outer layer's texels, how much the outer layer grows)
    let parts: [Part; 6] = [
        ([-4., 24., -4.], [8., 8., 8.], (0., 0.), (32., 0.), 0.5),
        ([-4., 12., -2.], [8., 12., 4.], (16., 16.), (16., 32.), 0.25),
        (
            [-4. - arm, 12., -2.],
            [arm, 12., 4.],
            (40., 16.),
            (40., 32.),
            0.25,
        ),
        ([4., 12., -2.], [arm, 12., 4.], (32., 48.), (48., 48.), 0.25),
        ([-4., 0., -2.], [4., 12., 4.], (0., 16.), (0., 32.), 0.25),
        ([0., 0., -2.], [4., 12., 4.], (16., 48.), (0., 48.), 0.25),
    ];
    for (min, size, base, outer, grow) in parts {
        all.extend(cuboid(min, size, base, 0., Sheet::Skin));
        all.extend(cuboid(min, size, outer, grow, Sheet::Skin));
    }
    if cape {
        // The cape box (10 by 16 by 1) hangs from the shoulders behind the back, turned half
        // round so its outer face looks backward, as the game's model is, and swung back a
        // little from its top edge.
        let tilt = 8f32.to_radians();
        let (top, back) = (24., -2.);
        for mut q in cuboid([-5., 8., -3.], [10., 16., 1.], (0., 0.), 0., Sheet::Cape) {
            let turn = |p: V3| [-p[0], p[1], 2. * -2.5 - p[2]];
            let swing = |p: V3| {
                let (dy, dz) = (p[1] - top, p[2] - back);
                [
                    p[0],
                    top + dy * tilt.cos() - dz * tilt.sin(),
                    back + dz * tilt.cos() + dy * tilt.sin(),
                ]
            };
            q.corners = q.corners.map(|p| swing(turn(p)));
            let n = q.normal;
            let n = [-n[0], n[1], -n[2]];
            q.normal = [
                n[0],
                n[1] * tilt.cos() - n[2] * tilt.sin(),
                n[2] * tilt.cos() + n[1] * tilt.sin(),
            ];
            all.push(q);
        }
    }
    all
}

/// The model in `pose`, `width` by `height` texels, row by row as RGBA; transparent around it.
pub fn render(pose: &Pose, width: usize, height: usize) -> Vec<[u8; 4]> {
    let mut out = vec![[0u8; 4]; width * height];
    let mut depth = vec![f32::NEG_INFINITY; width * height];
    if width == 0 || height == 0 {
        return out;
    }
    let (sy, cy) = pose.yaw.sin_cos();
    let (sp, cp) = pose.pitch.sin_cos();
    let turn = |p: V3| {
        let x = p[0] * cy + p[2] * sy;
        let z = -p[0] * sy + p[2] * cy;
        let y = p[1] - 16.;
        [x, y * cp - z * sp, y * sp + z * cp]
    };
    // The model is 32 units tall (36 with its outer layer, the hat and room around it), and as
    // wide as its turn makes it.
    let scale = (height as f32 / 38.).min(width as f32 / 24.);
    let (mid_x, mid_y) = (width as f32 / 2., height as f32 / 2.);
    let to_screen = |p: V3| [mid_x + p[0] * scale, mid_y - p[1] * scale, p[2]];
    let light = {
        let l: V3 = [-0.35, 0.55, 0.75];
        let n = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
        [l[0] / n, l[1] / n, l[2] / n]
    };
    for quad in faces(pose.slim, pose.cape.is_some()) {
        let normal = turn([quad.normal[0], quad.normal[1] + 16., quad.normal[2]]);
        // Faces turned away aren't drawn.
        if normal[2] <= 0. {
            continue;
        }
        let texture = match quad.sheet {
            Sheet::Skin => pose.skin,
            Sheet::Cape => match pose.cape {
                Some(cape) => cape,
                None => continue,
            },
        };
        let lit = 0.55
            + 0.45 * (normal[0] * light[0] + normal[1] * light[1] + normal[2] * light[2]).max(0.);
        let k = texture.w as f32 / 64.;
        let corners = quad.corners.map(|p| to_screen(turn(p)));
        let [u0, v0, u1, v1] = quad.uv;
        let uvs = [[u0, v0], [u1, v0], [u1, v1], [u0, v1]];
        // The texel a point of the face takes, kept inside the face's own texels.
        let sample = |u: f32, v: f32| {
            let x = (u.clamp(u0.min(u1), u0.max(u1) - 0.001) * k) as usize;
            let y = (v.clamp(v0.min(v1), v0.max(v1) - 0.001) * k) as usize;
            texture.texel(x, y)
        };
        for [a, b, c] in [[0, 1, 2], [0, 2, 3]] {
            let (pa, pb, pc) = (corners[a], corners[b], corners[c]);
            let area = (pb[0] - pa[0]) * (pc[1] - pa[1]) - (pb[1] - pa[1]) * (pc[0] - pa[0]);
            if area.abs() < 1e-6 {
                continue;
            }
            let lo_x = pa[0].min(pb[0]).min(pc[0]).floor().max(0.) as usize;
            let hi_x = (pa[0].max(pb[0]).max(pc[0]).ceil() as usize).min(width);
            let lo_y = pa[1].min(pb[1]).min(pc[1]).floor().max(0.) as usize;
            let hi_y = (pa[1].max(pb[1]).max(pc[1]).ceil() as usize).min(height);
            for y in lo_y..hi_y {
                for x in lo_x..hi_x {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let wa = ((pb[0] - px) * (pc[1] - py) - (pb[1] - py) * (pc[0] - px)) / area;
                    let wb = ((pc[0] - px) * (pa[1] - py) - (pc[1] - py) * (pa[0] - px)) / area;
                    let wc = 1. - wa - wb;
                    if wa < 0. || wb < 0. || wc < 0. {
                        continue;
                    }
                    let z = wa * pa[2] + wb * pb[2] + wc * pc[2];
                    let i = y * width + x;
                    if z <= depth[i] {
                        continue;
                    }
                    let u = wa * uvs[a][0] + wb * uvs[b][0] + wc * uvs[c][0];
                    let v = wa * uvs[a][1] + wb * uvs[b][1] + wc * uvs[c][1];
                    let [r, g, bl, al] = sample(u, v);
                    // Clear texels (the outer layer's gaps, a cape's corners) let what is behind
                    // show, as the game's cutout does.
                    if al < 128 {
                        continue;
                    }
                    let shade = |c: u8| (c as f32 * lit).round().min(255.) as u8;
                    out[i] = [shade(r), shade(g), shade(bl), 255];
                    depth[i] = z;
                }
            }
        }
    }
    out
}
