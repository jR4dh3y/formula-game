//! A CPU-drawn heads-up display that the live renderer composites over the scene.
//!
//! Insert a [`Hud`] resource and draw into it from update systems. The live
//! renderer keeps it sized to the window and uploads it whenever it changes.

use crate::math::Color;
use std::collections::HashMap;

/// A rasterized TrueType font with a per-size glyph cache.
pub struct HudFont {
    font: fontdue::Font,
    glyphs: HashMap<(char, u32), Glyph>,
}

struct Glyph {
    metrics: fontdue::Metrics,
    coverage: Vec<u8>,
}

impl HudFont {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let font = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
            .map_err(str::to_string)?;
        Ok(Self {
            font,
            glyphs: HashMap::new(),
        })
    }

    fn glyph(&mut self, character: char, size: u32) -> &Glyph {
        self.glyphs.entry((character, size)).or_insert_with(|| {
            let (metrics, coverage) = self.font.rasterize(character, size as f32);
            Glyph { metrics, coverage }
        })
    }

    fn advance(&mut self, character: char, size: u32) -> f32 {
        self.glyph(character, size).metrics.advance_width
    }

    fn ascent(&self, size: u32) -> f32 {
        self.font
            .horizontal_line_metrics(size as f32)
            .map(|metrics| metrics.ascent)
            .unwrap_or(size as f32 * 0.8)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Straight-alpha sRGB canvas in window pixels, origin at the top-left.
pub struct Hud {
    width: u32,
    height: u32,
    pixels: Vec<[u8; 4]>,
    fonts: Vec<HudFont>,
    revision: u64,
}

/// Index of a font registered with [`Hud::add_font`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontId(usize);

impl Default for Hud {
    fn default() -> Self {
        Self::new()
    }
}

impl Hud {
    pub fn new() -> Self {
        Self {
            width: 0,
            height: 0,
            pixels: Vec::new(),
            fonts: Vec::new(),
            revision: 0,
        }
    }

    pub fn add_font(&mut self, font: HudFont) -> FontId {
        self.fonts.push(font);
        FontId(self.fonts.len() - 1)
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Changes whenever the canvas content changes.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn pixels(&self) -> &[[u8; 4]] {
        &self.pixels
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if (width, height) != (self.width, self.height) {
            self.width = width;
            self.height = height;
            self.pixels = vec![[0; 4]; (width * height) as usize];
            self.revision += 1;
        }
    }

    /// Clears the canvas to transparent. Call at the start of each HUD redraw.
    pub fn clear(&mut self) {
        self.pixels.fill([0; 4]);
        self.revision += 1;
    }

    pub fn fill_rect(&mut self, x: f32, y: f32, width: f32, height: f32, color: Color) {
        let x0 = x.max(0.0).round() as i64;
        let y0 = y.max(0.0).round() as i64;
        let x1 = ((x + width).round() as i64).min(self.width as i64);
        let y1 = ((y + height).round() as i64).min(self.height as i64);
        for row in y0..y1 {
            for column in x0..x1 {
                self.blend(column, row, color, 1.0);
            }
        }
    }

    pub fn stroke_rect(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        line: f32,
        color: Color,
    ) {
        self.fill_rect(x, y, width, line, color);
        self.fill_rect(x, y + height - line, width, line, color);
        self.fill_rect(x, y + line, line, height - line * 2.0, color);
        self.fill_rect(x + width - line, y + line, line, height - line * 2.0, color);
    }

    /// Anti-aliased line segment of the given thickness.
    pub fn line(&mut self, from: (f32, f32), to: (f32, f32), thickness: f32, color: Color) {
        let radius = thickness * 0.5;
        let x0 = (from.0.min(to.0) - radius - 1.0).floor().max(0.0) as i64;
        let y0 = (from.1.min(to.1) - radius - 1.0).floor().max(0.0) as i64;
        let x1 = ((from.0.max(to.0) + radius + 1.0).ceil() as i64).min(self.width as i64);
        let y1 = ((from.1.max(to.1) + radius + 1.0).ceil() as i64).min(self.height as i64);
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let length_squared = (dx * dx + dy * dy).max(1e-6);
        for row in y0..y1 {
            for column in x0..x1 {
                let (px, py) = (column as f32 + 0.5, row as f32 + 0.5);
                let t =
                    (((px - from.0) * dx + (py - from.1) * dy) / length_squared).clamp(0.0, 1.0);
                let (cx, cy) = (from.0 + dx * t, from.1 + dy * t);
                let distance = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
                let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    self.blend(column, row, color, coverage);
                }
            }
        }
    }

    /// Anti-aliased filled circle.
    pub fn circle(&mut self, center: (f32, f32), radius: f32, color: Color) {
        let x0 = (center.0 - radius - 1.0).floor().max(0.0) as i64;
        let y0 = (center.1 - radius - 1.0).floor().max(0.0) as i64;
        let x1 = ((center.0 + radius + 1.0).ceil() as i64).min(self.width as i64);
        let y1 = ((center.1 + radius + 1.0).ceil() as i64).min(self.height as i64);
        for row in y0..y1 {
            for column in x0..x1 {
                let distance = ((column as f32 + 0.5 - center.0).powi(2)
                    + (row as f32 + 0.5 - center.1).powi(2))
                .sqrt();
                let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    self.blend(column, row, color, coverage);
                }
            }
        }
    }

    /// Width in pixels of `text` at `size` pixels.
    pub fn text_width(&mut self, font: FontId, size: f32, text: &str) -> f32 {
        let size = size.round().max(1.0) as u32;
        let font = &mut self.fonts[font.0];
        text.chars()
            .map(|character| font.advance(character, size))
            .sum()
    }

    /// Draws one line of text; `y` is the top of the line box. Returns the width.
    pub fn text(
        &mut self,
        font: FontId,
        size: f32,
        position: (f32, f32),
        align: Align,
        color: Color,
        text: &str,
    ) -> f32 {
        let width = self.text_width(font, size, text);
        let size = size.round().max(1.0) as u32;
        let mut pen_x = match align {
            Align::Left => position.0,
            Align::Center => position.0 - width * 0.5,
            Align::Right => position.0 - width,
        };
        let Self {
            fonts,
            pixels,
            width: canvas_width,
            height: canvas_height,
            ..
        } = self;
        let font = &mut fonts[font.0];
        let baseline = (position.1 + font.ascent(size)).round() as i64;
        for character in text.chars() {
            let glyph = font.glyph(character, size);
            let left = pen_x.round() as i64 + glyph.metrics.xmin as i64;
            let top = baseline - glyph.metrics.height as i64 - glyph.metrics.ymin as i64;
            for row in 0..glyph.metrics.height {
                for column in 0..glyph.metrics.width {
                    let alpha = glyph.coverage[row * glyph.metrics.width + column];
                    if alpha > 0 {
                        blend_pixel(
                            pixels,
                            (*canvas_width, *canvas_height),
                            (left + column as i64, top + row as i64),
                            color,
                            alpha as f32 / 255.0,
                        );
                    }
                }
            }
            pen_x += glyph.metrics.advance_width;
        }
        width
    }

    fn blend(&mut self, x: i64, y: i64, color: Color, coverage: f32) {
        blend_pixel(
            &mut self.pixels,
            (self.width, self.height),
            (x, y),
            color,
            coverage,
        );
    }
}

fn blend_pixel(
    pixels: &mut [[u8; 4]],
    (width, height): (u32, u32),
    (x, y): (i64, i64),
    color: Color,
    coverage: f32,
) {
    if x < 0 || y < 0 || x >= width as i64 || y >= height as i64 {
        return;
    }
    let source_alpha = (color.a * coverage).clamp(0.0, 1.0);
    if source_alpha <= 0.0 {
        return;
    }
    let pixel = &mut pixels[(y as u32 * width + x as u32) as usize];
    // Most HUD pixels are drawn onto transparent canvas after clear(). Avoid
    // three divisions for those pixels and for fully opaque source colours.
    if pixel[3] == 0 || source_alpha >= 1.0 {
        *pixel = [
            (color.r.clamp(0.0, 1.0) * 255.0).round() as u8,
            (color.g.clamp(0.0, 1.0) * 255.0).round() as u8,
            (color.b.clamp(0.0, 1.0) * 255.0).round() as u8,
            (source_alpha * 255.0).round() as u8,
        ];
        return;
    }
    let destination_alpha = pixel[3] as f32 / 255.0;
    let out_alpha = source_alpha + destination_alpha * (1.0 - source_alpha);
    let channel = |source: f32, destination: u8| {
        let destination = destination as f32 / 255.0;
        let value = (source * source_alpha
            + destination * destination_alpha * (1.0 - source_alpha))
            / out_alpha.max(1e-6);
        (value.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    *pixel = [
        channel(color.r, pixel[0]),
        channel(color.g, pixel[1]),
        channel(color.b, pixel[2]),
        (out_alpha * 255.0).round() as u8,
    ];
}
