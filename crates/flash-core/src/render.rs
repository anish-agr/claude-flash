//! What a flash looks like: its opacity over time and its shape on screen.
//!
//! Both platforms draw from these functions, so a flash on macOS and one on Windows
//! have the same envelope and the same falloff to the pixel.

use serde::{Deserialize, Serialize};

use crate::color::Rgb;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    /// A full-screen tint, stronger at the edges than the centre.
    #[default]
    Wash,
    /// A glow around the screen edges with a clear centre.
    Edge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    pub fade_in_ms: u32,
    pub hold_ms: u32,
    pub fade_out_ms: u32,
    pub dismiss_fade_ms: u32,
    pub min_visible_ms: u32,
}

pub fn ease_out_cubic(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    1.0 - (1.0 - x).powi(3)
}

pub fn ease_in_out_cubic(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    if x < 0.5 { 4.0 * x * x * x } else { 1.0 - (-2.0 * x + 2.0).powi(3) / 2.0 }
}

/// Opacity of one flash over its lifetime, including an early dismissal.
#[derive(Clone, Debug, PartialEq)]
pub struct Envelope {
    timing: Timing,
    peak: f32,
    dismissed: Option<(u64, f32)>,
}

/// With reduced motion, the tint cross-fades in and out quickly instead of easing,
/// and holds a little longer so it is still noticed.
const REDUCED_FADE_MS: u32 = 90;

impl Envelope {
    pub fn new(mut timing: Timing, peak: f32, reduce_motion: bool) -> Self {
        if reduce_motion {
            timing.hold_ms =
                timing.hold_ms.saturating_add(timing.fade_in_ms + timing.fade_out_ms) / 2 + timing.hold_ms / 2;
            timing.fade_in_ms = REDUCED_FADE_MS.min(timing.fade_in_ms.max(1));
            timing.fade_out_ms = REDUCED_FADE_MS;
        }
        Envelope { timing, peak: peak.clamp(0.0, 1.0), dismissed: None }
    }

    pub fn timing(&self) -> Timing {
        self.timing
    }

    pub fn total_ms(&self) -> u64 {
        match self.dismissed {
            Some((at, _)) => at + u64::from(self.timing.dismiss_fade_ms.max(1)),
            None => {
                u64::from(self.timing.fade_in_ms)
                    + u64::from(self.timing.hold_ms)
                    + u64::from(self.timing.fade_out_ms.max(1))
            }
        }
    }

    /// Opacity at `t` milliseconds after the flash started, or `None` once finished.
    pub fn alpha(&self, t: u64) -> Option<f32> {
        if let Some((at, from)) = self.dismissed {
            let d = (t.saturating_sub(at)) as f32 / self.timing.dismiss_fade_ms.max(1) as f32;
            return (d < 1.0).then(|| from * (1.0 - ease_out_cubic(d)));
        }
        let (fade_in, hold, fade_out) = (
            u64::from(self.timing.fade_in_ms),
            u64::from(self.timing.hold_ms),
            u64::from(self.timing.fade_out_ms.max(1)),
        );
        if t < fade_in {
            Some(self.peak * ease_out_cubic(t as f32 / fade_in as f32))
        } else if t < fade_in + hold {
            Some(self.peak)
        } else {
            let d = (t - fade_in - hold) as f32 / fade_out as f32;
            (d < 1.0).then(|| self.peak * (1.0 - ease_in_out_cubic(d)))
        }
    }

    /// Starts fading out early because the user pressed a key or clicked. Ignored
    /// before `min_visible_ms`, so a keystroke already in flight cannot hide a flash
    /// before it is seen. Returns whether the dismissal took effect.
    pub fn dismiss(&mut self, t: u64) -> bool {
        if self.dismissed.is_some() || t < u64::from(self.timing.min_visible_ms) {
            return false;
        }
        match self.alpha(t) {
            Some(a) => {
                self.dismissed = Some((t, a));
                true
            }
            None => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ByteOrder {
    /// Win32 DIB sections.
    Bgra,
    /// CoreGraphics `kCGImageAlphaPremultipliedLast`.
    Rgba,
}

/// Per-column factors, per-row factors, and how one of each combines into an alpha.
type Separable = (Vec<f32>, Vec<f32>, fn(f32, f32) -> f32);

/// Fills `buf` (`width * height * 4` bytes, top-down rows) with premultiplied pixels
/// of `color`, shaped by `style`. Per-pixel alpha here is the shape; the overall
/// fade is applied separately by the window system, so this runs once per flash.
pub fn paint(buf: &mut [u8], width: usize, height: usize, color: Rgb, style: Style, vignette: f32, order: ByteOrder) {
    assert_eq!(buf.len(), width * height * 4, "buffer does not match dimensions");
    if width == 0 || height == 0 {
        return;
    }
    // Every shape here is separable into a per-column and a per-row term, so the
    // inner loop is a multiply or a min rather than a distance calculation.
    let (cols, rows, combine): Separable = match style {
        Style::Wash => {
            let v = vignette.clamp(0.0, 0.9);
            let bowl = |i: usize, n: usize| {
                let d = 2.0 * (i as f32 + 0.5) / n as f32 - 1.0;
                1.0 - d * d
            };
            // 1 - v * fx * fy: `rows` carries the -v factor so combine stays a product.
            let cols = (0..width).map(|x| bowl(x, width)).collect();
            let rows = (0..height).map(|y| v * bowl(y, height)).collect();
            (cols, rows, |c, r| 1.0 - c * r)
        }
        Style::Edge => {
            let band = (width.min(height) as f32 * 0.14).max(1.0);
            let edge = |i: usize, n: usize| (i.min(n - 1 - i) as f32 / band).min(1.0);
            let cols = (0..width).map(|x| edge(x, width)).collect();
            let rows = (0..height).map(|y| edge(y, height)).collect();
            (cols, rows, |c, r| {
                let d = 1.0 - c.min(r);
                d * d
            })
        }
    };
    let (ci0, ci2) = match order {
        ByteOrder::Bgra => (2, 0),
        ByteOrder::Rgba => (0, 2),
    };
    let (r, g, b) = (f32::from(color.r), f32::from(color.g), f32::from(color.b));
    for (y, row) in buf.chunks_exact_mut(width * 4).enumerate() {
        let ry = rows[y];
        for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let a = combine(cols[x], ry).clamp(0.0, 1.0);
            px[ci0] = (r * a + 0.5) as u8;
            px[1] = (g * a + 0.5) as u8;
            px[ci2] = (b * a + 0.5) as u8;
            px[3] = (255.0 * a + 0.5) as u8;
        }
    }
}

/// Draws the Claude Flash icon, a shaded sphere, `size` pixels square in `color`,
/// with straight rather than premultiplied alpha. The tray and the menu bar show it
/// in the colour of the current state.
pub fn sphere(buf: &mut [u8], size: usize, color: Rgb, order: ByteOrder) {
    assert_eq!(buf.len(), size * size * 4, "buffer does not match dimensions");
    let (red, blue) = match order {
        ByteOrder::Bgra => (2, 0),
        ByteOrder::Rgba => (0, 2),
    };
    let radius = size as f32 / 2.0;
    // Light from the upper left, leaning towards the viewer.
    let (lx, ly, lz) = {
        let (x, y, z) = (-0.45f32, -0.55f32, 0.70f32);
        let n = (x * x + y * y + z * z).sqrt();
        (x / n, y / n, z / n)
    };
    let base = [f32::from(color.r), f32::from(color.g), f32::from(color.b)];
    for (i, px) in buf.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let x = ((i % size) as f32 + 0.5 - radius) / radius;
        let y = ((i / size) as f32 + 0.5 - radius) / radius;
        let r2 = x * x + y * y;
        // Anti-aliased rim: coverage falls from full to none across the edge pixel.
        let coverage = ((1.0 - r2.sqrt()) * radius + 0.5).clamp(0.0, 1.0);
        if coverage <= 0.0 {
            px.fill(0);
            continue;
        }
        let z = (1.0 - r2.min(1.0)).sqrt();
        let shade = 0.5 + 0.5 * (x * lx + y * ly + z * lz).max(0.0);
        let (hx, hy) = (x + 0.35, y + 0.4);
        let highlight = (1.0 - (hx * hx + hy * hy).sqrt() / 0.6).clamp(0.0, 1.0).powi(2) * 0.6;
        for (channel, value) in [red, 1, blue].into_iter().zip(base) {
            let lit = value * shade;
            px[channel] = (lit + (255.0 - lit) * highlight).round() as u8;
        }
        px[3] = (coverage * 255.0).round() as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sphere_is_solid_in_the_middle_clear_at_the_corners_and_lit_from_above_left() {
        let size = 32;
        let mut buf = vec![0u8; size * size * 4];
        sphere(&mut buf, size, Rgb::new(0x00, 0xFF, 0x5A), ByteOrder::Rgba);
        let at = |x: usize, y: usize| {
            let i = (y * size + x) * 4;
            [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
        };
        assert_eq!(at(16, 16)[3], 255);
        assert_eq!(at(0, 0)[3], 0);
        assert_eq!(at(31, 31)[3], 0);
        assert!(at(10, 9)[0] > at(22, 23)[0], "the highlight sits up and to the left");
    }

    const T: Timing =
        Timing { fade_in_ms: 70, hold_ms: 420, fade_out_ms: 560, dismiss_fade_ms: 110, min_visible_ms: 120 };

    #[test]
    fn envelope_rises_holds_and_falls() {
        let e = Envelope::new(T, 0.28, false);
        assert_eq!(e.alpha(0), Some(0.0));
        assert!(e.alpha(35).unwrap() > 0.0 && e.alpha(35).unwrap() < 0.28);
        assert_eq!(e.alpha(70), Some(0.28));
        assert_eq!(e.alpha(489), Some(0.28));
        assert!(e.alpha(800).unwrap() < 0.28);
        assert_eq!(e.alpha(1_050), None);
        assert_eq!(e.total_ms(), 1_050);
    }

    #[test]
    fn envelope_is_monotonic_in_each_phase() {
        let e = Envelope::new(T, 0.5, false);
        let samples: Vec<f32> = (0..1_050).map(|t| e.alpha(t).unwrap()).collect();
        assert!(samples[..70].windows(2).all(|w| w[1] >= w[0]));
        assert!(samples[490..].windows(2).all(|w| w[1] <= w[0]));
    }

    #[test]
    fn dismissal_respects_min_visible_and_fades_from_current_level() {
        let mut e = Envelope::new(T, 0.3, false);
        assert!(!e.dismiss(50), "too early");
        assert!(e.dismiss(200));
        assert!(!e.dismiss(210), "already dismissed");
        assert!((e.alpha(200).unwrap() - 0.3).abs() < 1e-6);
        assert!(e.alpha(260).unwrap() < 0.3);
        assert_eq!(e.alpha(310), None);
        assert_eq!(e.total_ms(), 310);
    }

    #[test]
    fn reduced_motion_shortens_fades_but_keeps_the_signal_visible() {
        let e = Envelope::new(T, 0.3, true);
        assert!(e.timing().fade_in_ms <= REDUCED_FADE_MS && e.timing().fade_out_ms == REDUCED_FADE_MS);
        assert!(e.timing().hold_ms >= T.hold_ms, "must not get shorter overall");
    }

    fn pixel(buf: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
        let i = (y * w + x) * 4;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    #[test]
    fn wash_is_heavier_at_edges_than_centre() {
        let (w, h) = (64, 40);
        let mut buf = vec![0; w * h * 4];
        paint(&mut buf, w, h, Rgb::new(0, 255, 90), Style::Wash, 0.32, ByteOrder::Rgba);
        let centre = pixel(&buf, w, w / 2, h / 2)[3];
        let corner = pixel(&buf, w, 0, 0)[3];
        assert!(corner > centre);
        // With vignette v the centre sits at roughly (1 - v) of full alpha.
        assert!((i32::from(centre) - (255.0 * 0.68) as i32).abs() <= 3, "centre alpha {centre}");
    }

    #[test]
    fn edge_is_clear_in_the_middle_and_solid_at_the_border() {
        let (w, h) = (100, 100);
        let mut buf = vec![0; w * h * 4];
        paint(&mut buf, w, h, Rgb::new(255, 0, 0), Style::Edge, 0.0, ByteOrder::Rgba);
        assert_eq!(pixel(&buf, w, 50, 50)[3], 0);
        assert_eq!(pixel(&buf, w, 0, 50)[3], 255);
    }

    #[test]
    fn output_is_premultiplied_and_channel_order_is_honoured() {
        let (w, h) = (8, 8);
        let (mut rgba, mut bgra) = (vec![0; w * h * 4], vec![0; w * h * 4]);
        let c = Rgb::new(200, 100, 10);
        paint(&mut rgba, w, h, c, Style::Wash, 0.5, ByteOrder::Rgba);
        paint(&mut bgra, w, h, c, Style::Wash, 0.5, ByteOrder::Bgra);
        for (p, q) in rgba.as_chunks::<4>().0.iter().zip(bgra.as_chunks::<4>().0) {
            assert!(p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3], "colour must not exceed alpha");
            assert_eq!((p[0], p[1], p[2], p[3]), (q[2], q[1], q[0], q[3]));
        }
    }

    #[test]
    fn degenerate_sizes_do_not_panic() {
        paint(&mut [], 0, 0, Rgb::new(1, 2, 3), Style::Edge, 0.3, ByteOrder::Bgra);
        let mut one = vec![0; 4];
        paint(&mut one, 1, 1, Rgb::new(1, 2, 3), Style::Edge, 0.3, ByteOrder::Bgra);
    }
}
