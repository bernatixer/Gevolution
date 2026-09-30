//! Gevolution's UI theme (Meadow): friendly, green, low-contrast chrome around the world.

use bevy::prelude::Resource;
use bevy_egui::egui::{self, Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Vec2};

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub dark: bool,
    pub bg: Color32,
    pub panel: Color32,
    pub card: Color32,
    pub field: Color32,
    pub text: Color32,
    pub weak: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub on_accent: Color32,
    pub soft: Color32,
    pub water: Color32,
    pub plants: Color32,
    pub animals: Color32,
    pub warn: Color32,
    pub border: Color32,
    /// Sky color for the 3D view, as linear-ish sRGB floats.
    pub sky: [f32; 3],
}

/// The Meadow palette: cream paper, leaf-green accents, water-blue details.
pub fn meadow() -> Palette {
    let c = Color32::from_rgb;
    Palette {
        dark: false,
        bg: c(244, 247, 236),
        panel: c(250, 252, 245),
        card: c(236, 243, 226),
        field: c(255, 255, 252),
        text: c(35, 58, 42),
        weak: c(110, 132, 112),
        accent: c(62, 155, 90),
        accent_hover: c(78, 175, 106),
        on_accent: c(255, 255, 255),
        soft: c(214, 234, 204),
        water: c(52, 140, 196),
        plants: c(76, 160, 70),
        animals: c(214, 128, 56),
        warn: c(206, 90, 60),
        border: c(210, 222, 198),
        sky: [0.70, 0.83, 0.92],
    }
}

#[derive(Resource, Clone, Copy)]
pub struct ActiveTheme(pub Palette);

impl Default for ActiveTheme {
    fn default() -> Self {
        ActiveTheme(meadow())
    }
}

pub fn apply(ctx: &egui::Context, p: &Palette) {
    ctx.set_theme(if p.dark { egui::Theme::Dark } else { egui::Theme::Light });
    ctx.all_styles_mut(|style| {
        let r = CornerRadius::same(10);
        let v = &mut style.visuals;
        v.dark_mode = p.dark;
        v.override_text_color = Some(p.text);
        v.weak_text_color = Some(p.weak);
        v.panel_fill = p.panel;
        v.window_fill = p.panel;
        v.window_stroke = Stroke::new(1.0, p.border);
        v.window_corner_radius = CornerRadius::same(14);
        v.menu_corner_radius = CornerRadius::same(10);
        v.window_shadow = Shadow {
            offset: [0, 6],
            blur: 18,
            spread: 0,
            color: Color32::from_black_alpha(if p.dark { 90 } else { 40 }),
        };
        v.popup_shadow = Shadow {
            offset: [0, 4],
            blur: 12,
            spread: 0,
            color: Color32::from_black_alpha(if p.dark { 80 } else { 30 }),
        };
        v.faint_bg_color = p.card;
        v.extreme_bg_color = p.field;
        v.text_edit_bg_color = Some(p.field);
        v.code_bg_color = p.card;
        v.hyperlink_color = p.water;
        v.warn_fg_color = p.warn;
        v.error_fg_color = p.warn;
        v.selection.bg_fill = p.accent;
        v.selection.stroke = Stroke::new(1.0, p.on_accent);
        v.striped = false;
        v.slider_trailing_fill = true;
        v.collapsing_header_frame = false;
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = p.panel;
        w.noninteractive.weak_bg_fill = p.card;
        w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
        w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
        w.noninteractive.corner_radius = r;
        w.inactive.bg_fill = p.soft;
        w.inactive.weak_bg_fill = p.card;
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0, p.text);
        w.inactive.corner_radius = r;
        w.hovered.bg_fill = p.accent_hover;
        w.hovered.weak_bg_fill = p.soft;
        w.hovered.bg_stroke = Stroke::new(1.0, p.accent);
        w.hovered.fg_stroke = Stroke::new(1.5, p.text);
        w.hovered.corner_radius = r;
        w.hovered.expansion = 1.0;
        w.active.bg_fill = p.accent;
        w.active.weak_bg_fill = p.accent;
        w.active.bg_stroke = Stroke::new(1.0, p.accent);
        // Strong text uses the active foreground; keep it readable (accent buttons set their own text color).
        w.active.fg_stroke = Stroke::new(1.5, p.text);
        w.active.corner_radius = r;
        w.open.bg_fill = p.soft;
        w.open.weak_bg_fill = p.soft;
        w.open.corner_radius = r;
        let s = &mut style.spacing;
        s.item_spacing = Vec2::new(8.0, 7.0);
        s.button_padding = Vec2::new(12.0, 6.0);
        s.window_margin = Margin::same(14);
        s.interact_size.y = 26.0;
        s.slider_width = 150.0;
        style.text_styles = [
            (TextStyle::Heading, FontId::new(21.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(14.5, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(14.5, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(13.5, FontFamily::Monospace)),
            (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
        ]
        .into();
    });
}
