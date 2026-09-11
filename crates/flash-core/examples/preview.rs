//! Draws the pictures in the README with the same code that draws flashes on screen:
//!
//! ```text
//! cargo run -p flash-core --example preview
//! ```
//!
//! `docs/images/signals.png` shows each signal's wash over a desktop at its default
//! colour and opacity; `docs/images/icon.png` is the sphere the tray and menu bar use.

use std::error::Error;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::Path;

use flash_core::color::Rgb;
use flash_core::config;
use flash_core::event::Attention;
use flash_core::render::{self, ByteOrder, Style};

const PANEL_W: usize = 640;
const PANEL_H: usize = 360;
const GAP: usize = 16;

const SIGNALS: [(Attention, &str); 4] = [
    (Attention::Done, "DONE"),
    (Attention::Question, "QUESTION"),
    (Attention::Approval, "APPROVAL"),
    (Attention::Error, "ERROR"),
];

fn main() -> Result<(), Box<dyn Error>> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/images");
    fs::create_dir_all(&dir)?;

    let width = 2 * PANEL_W + 3 * GAP;
    let height = 2 * PANEL_H + 3 * GAP;
    let mut sheet = Canvas::new(width, height, Rgb::new(0x0B, 0x0D, 0x12));
    for (i, (kind, word)) in SIGNALS.into_iter().enumerate() {
        let panel = panel(kind, word);
        sheet.blit(&panel, GAP + (i % 2) * (PANEL_W + GAP), GAP + (i / 2) * (PANEL_H + GAP));
    }
    sheet.save(&dir.join("signals.png"))?;

    let size = 128;
    let mut icon = vec![0u8; size * size * 4];
    render::sphere(&mut icon, size, config::builtin(Attention::Done).color, ByteOrder::Rgba);
    save_rgba(&dir.join("icon.png"), &icon, size, size)?;
    println!("wrote {}", dir.display());
    Ok(())
}

/// A rectangle: left, top, width, height.
type Rect = (usize, usize, usize, usize);

/// A desktop with a terminal window, washed in the signal's colour.
fn panel(kind: Attention, word: &str) -> Canvas {
    let mut c = Canvas::new(PANEL_W, PANEL_H, Rgb::new(0, 0, 0));
    for y in 0..PANEL_H {
        let t = y as f32 / PANEL_H as f32;
        c.fill((0, y, PANEL_W, 1), mix(Rgb::new(0x26, 0x2B, 0x3B), Rgb::new(0x12, 0x15, 0x1C), t), 1.0);
    }

    // The terminal window.
    let (wx, wy, ww, wh) = (56, 44, 528, 236);
    let panel_colour = Rgb::new(0x16, 0x1B, 0x22);
    c.round((wx - 1, wy - 1, ww + 2, wh + 2), 11.0, Rgb::new(0x30, 0x36, 0x3D), 1.0);
    c.round((wx, wy, ww, wh), 10.0, Rgb::new(0x0D, 0x11, 0x17), 1.0);
    c.round((wx, wy, ww, 30), 10.0, panel_colour, 1.0);
    c.fill((wx, wy + 20, ww, 10), panel_colour, 1.0);
    for (i, dot) in [0xFF5F57, 0xFEBC2E, 0x28C840].into_iter().enumerate() {
        c.circle(wx as f32 + 18.0 + i as f32 * 18.0, wy as f32 + 15.0, 5.5, hex(dot));
    }

    let text = Rgb::new(0xC9, 0xD1, 0xD9);
    let muted = Rgb::new(0x8B, 0x94, 0x9E);
    let (lx, mut ly) = (wx + 20, wy + 50);
    c.fill((lx, ly, 8, 10), hex(0x58A6FF), 1.0);
    c.round((lx + 16, ly + 1, 250, 8), 4.0, text, 0.85);
    for width in [384, 330, 356] {
        ly += 24;
        c.round((lx + 16, ly + 1, width, 8), 4.0, muted, 0.7);
    }
    ly += 34;
    let accent = config::builtin(kind).color;
    match kind {
        Attention::Done => c.round((lx + 16, ly + 1, 170, 8), 4.0, hex(0x3FB950), 0.9),
        Attention::Error => c.round((lx + 16, ly + 1, 240, 8), 4.0, hex(0xF85149), 0.9),
        Attention::Question | Attention::Approval => {
            c.round((lx + 15, ly - 13, 334, 44), 8.0, accent, 0.9);
            c.round((lx + 17, ly - 11, 330, 40), 7.0, Rgb::new(0x0D, 0x11, 0x17), 1.0);
            c.round((lx + 32, ly + 1, 150, 8), 4.0, text, 0.85);
            if kind == Attention::Approval {
                c.round((lx + 228, ly - 3, 50, 16), 5.0, accent, 0.95);
                c.round((lx + 286, ly - 3, 50, 16), 5.0, muted, 0.5);
            }
        }
    }

    // The flash, exactly as the agent paints it.
    let signal = config::builtin(kind);
    let mut wash = vec![0u8; PANEL_W * PANEL_H * 4];
    render::paint(&mut wash, PANEL_W, PANEL_H, signal.color, Style::Wash, 0.32, ByteOrder::Rgba);
    let opacity = signal.opacity as f32;
    for (i, px) in wash.as_chunks::<4>().0.iter().enumerate() {
        c.blend(i, signal.color, f32::from(px[3]) / 255.0 * opacity);
    }

    let label_y = PANEL_H - 50;
    c.circle(62.0, label_y as f32 + 14.0, 9.0, accent);
    c.text(84, label_y, 4, word, Rgb::new(0xFF, 0xFF, 0xFF));
    c
}

struct Canvas {
    w: usize,
    h: usize,
    px: Vec<[f32; 3]>,
}

impl Canvas {
    fn new(w: usize, h: usize, fill: Rgb) -> Canvas {
        Canvas { w, h, px: vec![rgb(fill); w * h] }
    }

    fn blend(&mut self, i: usize, color: Rgb, alpha: f32) {
        let alpha = alpha.clamp(0.0, 1.0);
        for (d, s) in self.px[i].iter_mut().zip(rgb(color)) {
            *d = *d * (1.0 - alpha) + s * alpha;
        }
    }

    fn plot(&mut self, x: usize, y: usize, color: Rgb, alpha: f32) {
        if x < self.w && y < self.h {
            self.blend(y * self.w + x, color, alpha);
        }
    }

    fn fill(&mut self, (x, y, w, h): Rect, color: Rgb, alpha: f32) {
        for yy in y..(y + h).min(self.h) {
            for xx in x..(x + w).min(self.w) {
                self.plot(xx, yy, color, alpha);
            }
        }
    }

    /// A rectangle with anti-aliased round corners of radius `r`.
    fn round(&mut self, (x, y, w, h): Rect, r: f32, color: Rgb, alpha: f32) {
        let r = r.min(w as f32 / 2.0).min(h as f32 / 2.0);
        for yy in y..y + h {
            for xx in x..x + w {
                let (px, py) = (xx as f32 + 0.5 - x as f32, yy as f32 + 0.5 - y as f32);
                let dx = (r - px).max(px - (w as f32 - r)).max(0.0);
                let dy = (r - py).max(py - (h as f32 - r)).max(0.0);
                let coverage = (r - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    self.plot(xx, yy, color, alpha * coverage);
                }
            }
        }
    }

    fn circle(&mut self, cx: f32, cy: f32, r: f32, color: Rgb) {
        let (x0, y0) = ((cx - r - 1.0).max(0.0) as usize, (cy - r - 1.0).max(0.0) as usize);
        for yy in y0..(cy + r + 2.0) as usize {
            for xx in x0..(cx + r + 2.0) as usize {
                let d = ((xx as f32 + 0.5 - cx).powi(2) + (yy as f32 + 0.5 - cy).powi(2)).sqrt();
                let coverage = (r - d + 0.5).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    self.plot(xx, yy, color, coverage);
                }
            }
        }
    }

    /// Capital letters from a 5 by 7 bitmap font, each dot `scale` pixels square.
    fn text(&mut self, x: usize, y: usize, scale: usize, word: &str, color: Rgb) {
        for (i, letter) in word.chars().enumerate() {
            let Some(rows) = glyph(letter) else { continue };
            let left = x + i * 6 * scale;
            for (row, bits) in rows.iter().enumerate() {
                for col in 0..5 {
                    if bits & (0b10000 >> col) != 0 {
                        self.fill((left + col * scale, y + row * scale, scale, scale), color, 0.95);
                    }
                }
            }
        }
    }

    fn blit(&mut self, other: &Canvas, x: usize, y: usize) {
        for row in 0..other.h {
            let start = (y + row) * self.w + x;
            self.px[start..start + other.w].copy_from_slice(&other.px[row * other.w..(row + 1) * other.w]);
        }
    }

    fn save(&self, path: &Path) -> Result<(), Box<dyn Error>> {
        let bytes: Vec<u8> = self.px.iter().flat_map(|p| p.map(|v| v.round().clamp(0.0, 255.0) as u8)).collect();
        let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), self.w as u32, self.h as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&bytes)?;
        Ok(())
    }
}

fn save_rgba(path: &Path, rgba: &[u8], w: usize, h: usize) -> Result<(), Box<dyn Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), w as u32, h as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(rgba)?;
    Ok(())
}

fn glyph(letter: char) -> Option<[u8; 7]> {
    Some(match letter {
        'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'D' => [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'I' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'N' => [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'Q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'S' => [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        _ => return None,
    })
}

fn rgb(c: Rgb) -> [f32; 3] {
    [f32::from(c.r), f32::from(c.g), f32::from(c.b)]
}

fn hex(value: u32) -> Rgb {
    Rgb::new((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let lerp = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Rgb::new(lerp(a.r, b.r), lerp(a.g, b.g), lerp(a.b, b.b))
}
