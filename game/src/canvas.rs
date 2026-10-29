//! Immediate HUD geometry submitted to egui's Bevy render pass.

use bevy_egui::egui::{
    self, Align2, Color32, FontFamily, FontId, Painter, Pos2, Rect, Stroke, Vec2,
};

pub enum Align {
    Left,
    Center,
    Right,
}

pub struct Hud {
    pub painter: Painter,
    pub size: Vec2,
    pub finish: Finish,
}

#[derive(Clone, Copy)]
pub enum Finish {
    Digital,
    Analog,
}

pub struct SurfaceColors {
    pub top: Color32,
    pub bottom: Color32,
    pub edge: Color32,
    pub ink: Color32,
    pub muted: Color32,
    pub accent: Color32,
}

impl Finish {
    pub fn colors(self) -> SurfaceColors {
        match self {
            Self::Digital => SurfaceColors {
                top: Color32::from_rgb(34, 37, 36),
                bottom: Color32::from_rgb(10, 13, 13),
                edge: Color32::from_rgb(84, 87, 79),
                ink: Color32::from_rgb(234, 240, 213),
                muted: Color32::from_rgb(163, 172, 153),
                accent: Color32::from_rgb(246, 61, 49),
            },
            Self::Analog => SurfaceColors {
                top: Color32::from_rgb(34, 35, 36),
                bottom: Color32::from_rgb(12, 14, 16),
                edge: Color32::from_rgb(85, 88, 89),
                ink: Color32::from_rgb(235, 233, 219),
                muted: Color32::from_rgb(174, 177, 171),
                accent: Color32::from_rgb(223, 225, 211),
            },
        }
    }
}

pub fn rgba(r: f32, g: f32, b: f32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        (r * 255.0) as u8,
        (g * 255.0) as u8,
        (b * 255.0) as u8,
        (a * 255.0) as u8,
    )
}

impl Hud {
    /// A chamfered instrument plate with a directional material highlight.
    pub fn plate(&self, rect: Rect, cut: f32, top: Color32, bottom: Color32, edge: Color32) {
        let cut = cut.min(rect.height() * 0.5).min(rect.width() * 0.5);
        let points = vec![
            Pos2::new(rect.left() + cut, rect.top()),
            Pos2::new(rect.right() - cut, rect.top()),
            Pos2::new(rect.right(), rect.top() + cut),
            Pos2::new(rect.right(), rect.bottom() - cut),
            Pos2::new(rect.right() - cut, rect.bottom()),
            Pos2::new(rect.left() + cut, rect.bottom()),
            Pos2::new(rect.left(), rect.bottom() - cut),
            Pos2::new(rect.left(), rect.top() + cut),
        ];
        let mut mesh = egui::Mesh::default();
        for point in &points {
            mesh.colored_vertex(
                *point,
                top.lerp_to_gamma(bottom, (point.y - rect.top()) / rect.height()),
            );
        }
        for index in 1..7 {
            mesh.add_triangle(0, index, index + 1);
        }
        self.painter.add(egui::Shape::mesh(mesh));
        self.painter
            .add(egui::Shape::closed_line(points, Stroke::new(1., edge)));
    }

    /// A radial material ramp for dial faces, turned hubs and lamp lenses.
    pub fn disc(&self, center: (f32, f32), radius: f32, inner: Color32, outer: Color32) {
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(Pos2::new(center.0, center.1), inner);
        for index in 0..=96 {
            let angle = index as f32 / 96. * std::f32::consts::TAU;
            mesh.colored_vertex(
                Pos2::new(
                    center.0 + radius * angle.cos(),
                    center.1 + radius * angle.sin(),
                ),
                outer,
            );
        }
        for index in 1..=96 {
            mesh.add_triangle(0, index, index + 1);
        }
        self.painter.add(egui::Shape::mesh(mesh));
    }
    /// Seven illuminated bars plus a decimal point, drawn without a font atlas.
    pub fn segments(
        &self,
        height: f32,
        position: (f32, f32),
        align: Align,
        color: Color32,
        text: &str,
    ) {
        let advance = |ch| match ch {
            '.' | ':' | '°' => height * 0.28,
            '/' => height * 0.42,
            _ => height * 0.72,
        };
        let width = text.chars().map(advance).sum::<f32>();
        let mut x = position.0
            - match align {
                Align::Left => 0.,
                Align::Center => width * 0.5,
                Align::Right => width,
            };
        let y = position.1;
        let off = Color32::from_rgb(28, 29, 28);
        for ch in text.chars() {
            let mask = segment_mask(ch);
            let w = height * 0.58;
            let t = height * 0.095;
            let mid = height * 0.5;
            let horizontal = |cy: f32| {
                vec![
                    Pos2::new(x + t * 0.55, cy),
                    Pos2::new(x + t * 1.35, cy - t * 0.5),
                    Pos2::new(x + w - t * 1.35, cy - t * 0.5),
                    Pos2::new(x + w - t * 0.55, cy),
                    Pos2::new(x + w - t * 1.35, cy + t * 0.5),
                    Pos2::new(x + t * 1.35, cy + t * 0.5),
                ]
            };
            let vertical = |cx: f32, top: f32, bottom: f32| {
                vec![
                    Pos2::new(cx, top),
                    Pos2::new(cx + t * 0.5, top + t * 0.65),
                    Pos2::new(cx + t * 0.5, bottom - t * 0.65),
                    Pos2::new(cx, bottom),
                    Pos2::new(cx - t * 0.5, bottom - t * 0.65),
                    Pos2::new(cx - t * 0.5, top + t * 0.65),
                ]
            };
            if mask != 0 {
                for (bit, polygon) in [
                    horizontal(y + t * 0.5),
                    vertical(x + w - t * 0.5, y + t * 0.8, y + mid - t * 0.25),
                    vertical(x + w - t * 0.5, y + mid + t * 0.25, y + height - t * 0.8),
                    horizontal(y + height - t * 0.5),
                    vertical(x + t * 0.5, y + mid + t * 0.25, y + height - t * 0.8),
                    vertical(x + t * 0.5, y + t * 0.8, y + mid - t * 0.25),
                    horizontal(y + mid),
                ]
                .into_iter()
                .enumerate()
                {
                    self.painter.add(egui::Shape::convex_polygon(
                        polygon,
                        if mask & (1 << bit) != 0 { color } else { off },
                        Stroke::NONE,
                    ));
                }
            } else {
                match ch {
                    '.' => self.circle((x + height * 0.07, y + height - t), t * 0.6, color),
                    ':' => {
                        self.circle((x + height * 0.07, y + height * 0.3), t * 0.6, color);
                        self.circle((x + height * 0.07, y + height * 0.72), t * 0.6, color);
                    }
                    '/' => self.line(
                        (x + height * 0.02, y + height),
                        (x + height * 0.28, y),
                        t * 0.7,
                        color,
                    ),
                    '%' => {
                        self.line(
                            (x + height * 0.05, y + height * 0.9),
                            (x + height * 0.53, y + height * 0.1),
                            t * 0.7,
                            color,
                        );
                        for (cx, cy) in [
                            (x + height * 0.13, y + height * 0.18),
                            (x + height * 0.45, y + height * 0.82),
                        ] {
                            self.painter.circle_stroke(
                                Pos2::new(cx, cy),
                                height * 0.09,
                                Stroke::new(t * 0.5, color),
                            );
                        }
                    }
                    '°' => {
                        self.painter.circle_stroke(
                            Pos2::new(x + t, y + t),
                            t,
                            Stroke::new(t * 0.5, color),
                        );
                    }
                    '+' => {
                        self.line(
                            (x + w * 0.5, y + height * 0.3),
                            (x + w * 0.5, y + height * 0.7),
                            t,
                            color,
                        );
                        self.line((x, y + mid), (x + w, y + mid), t, color);
                    }
                    _ => {}
                }
            }
            x += advance(ch);
        }
    }
    /// Whether every character has a seven-segment form.
    pub fn can_segment(text: &str) -> bool {
        text.chars().all(|ch| {
            segment_mask(ch) != 0 || matches!(ch, '.' | ':' | '/' | '%' | '°' | '+' | ' ')
        })
    }

    pub fn width(&self) -> f32 {
        self.size.x
    }
    pub fn height(&self) -> f32 {
        self.size.y
    }

    pub fn fill_rect(&self, x: f32, y: f32, w: f32, h: f32, color: Color32) {
        self.painter.rect_filled(
            Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h)),
            0.0,
            color,
        );
    }

    pub fn fill_round(&self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: Color32) {
        self.painter.rect_filled(
            Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h)),
            radius,
            color,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn stroke_round(
        &self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        thickness: f32,
        color: Color32,
    ) {
        self.painter.rect_stroke(
            Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h)),
            radius,
            Stroke::new(thickness, color),
            egui::StrokeKind::Inside,
        );
    }

    pub fn circle(&self, center: (f32, f32), radius: f32, color: Color32) {
        self.painter
            .circle_filled(Pos2::new(center.0, center.1), radius, color);
    }

    pub fn line(&self, a: (f32, f32), b: (f32, f32), thickness: f32, color: Color32) {
        self.painter.line_segment(
            [Pos2::new(a.0, a.1), Pos2::new(b.0, b.1)],
            Stroke::new(thickness, color),
        );
    }

    pub fn text(
        &self,
        family: &FontFamily,
        size: f32,
        position: (f32, f32),
        align: Align,
        color: Color32,
        text: &str,
    ) {
        let anchor = match align {
            Align::Left => Align2::LEFT_TOP,
            Align::Center => Align2::CENTER_TOP,
            Align::Right => Align2::RIGHT_TOP,
        };
        self.painter.text(
            Pos2::new(position.0, position.1),
            anchor,
            text,
            FontId::new(size, family.clone()),
            color,
        );
    }

    /// Text whose capitals are centred on `position.1`, so words line up with
    /// segment digits and the middle of their row regardless of font padding.
    pub fn text_mid(
        &self,
        family: &FontFamily,
        size: f32,
        position: (f32, f32),
        align: Align,
        color: Color32,
        text: &str,
    ) {
        let galley =
            self.painter
                .layout_no_wrap(text.to_owned(), FontId::new(size, family.clone()), color);
        let baseline = galley
            .rows
            .first()
            .and_then(|row| row.glyphs.first().map(|glyph| row.pos.y + glyph.pos.y))
            .unwrap_or(size);
        let x = position.0
            - galley.size().x
                * match align {
                    Align::Left => 0.,
                    Align::Center => 0.5,
                    Align::Right => 1.,
                };
        let top = position.1 - baseline + CAP_HEIGHT * size * 0.5;
        self.painter.galley(Pos2::new(x, top), galley, color);
    }

    pub fn text_width(&self, family: &FontFamily, size: f32, text: &str) -> f32 {
        self.painter
            .layout_no_wrap(
                text.to_owned(),
                FontId::new(size, family.clone()),
                Color32::WHITE,
            )
            .size()
            .x
    }
}

/// Titillium Web cap height as a fraction of the em.
const CAP_HEIGHT: f32 = 0.685;

fn segment_mask(ch: char) -> u8 {
    match ch {
        '0' => 0x3f,
        '1' => 0x06,
        '2' => 0x5b,
        '3' => 0x4f,
        '4' => 0x66,
        '5' => 0x6d,
        '6' => 0x7d,
        '7' => 0x07,
        '8' => 0x7f,
        '9' => 0x6f,
        'N' => 0x54,
        'R' => 0x50,
        'C' => 0x39,
        'P' => 0x73,
        '-' => 0x40,
        _ => 0,
    }
}
