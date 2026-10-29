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
