//! Gevolution's UI themes: friendly, green, low-contrast chrome around the world.

use bevy::prelude::Resource;
use bevy_egui::egui::{self, Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    /// Light and airy: cream paper, leaf-green accents, water-blue details.
    Meadow,
    /// Calm teal-green mid tones, like a shaded pond.
    Lagoon,
    /// Deep forest greens with fresh lime accents.
    Grove,
}

impl Theme {
    pub const ALL: [Theme; 3] = [Theme::Meadow, Theme::Lagoon, Theme::Grove];
    pub fn name(&self) -> &'static str {
        match self {
            Theme::Meadow => "Meadow",
            Theme::Lagoon => "Lagoon",
            Theme::Grove => "Grove",
        }
    }
    pub fn from_name(s: &str) -> Option<Theme> {
        Theme::ALL.into_iter().find(|t| t.name().eq_ignore_ascii_case(s))
    }
}

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

pub fn palette(t: Theme) -> Palette {
    let c = Color32::from_rgb;
    match t {
        Theme::Meadow => Palette {
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
        },
        Theme::Lagoon => Palette {
            dark: true,
            bg: c(24, 58, 58),
            panel: c(30, 70, 68),
            card: c(38, 84, 80),
            field: c(22, 52, 52),
            text: c(232, 246, 238),
            weak: c(160, 196, 184),
            accent: c(94, 208, 150),
            accent_hover: c(120, 224, 170),
            on_accent: c(16, 48, 40),
            soft: c(48, 104, 96),
            water: c(110, 190, 232),
            plants: c(140, 214, 110),
            animals: c(244, 176, 102),
            warn: c(255, 140, 110),
            border: c(56, 112, 104),
            sky: [0.60, 0.78, 0.84],
        },
        Theme::Grove => Palette {
            dark: true,
            bg: c(20, 36, 26),
            panel: c(26, 46, 33),
            card: c(34, 58, 42),
            field: c(18, 32, 23),
            text: c(230, 242, 224),
            weak: c(150, 178, 148),
            accent: c(140, 212, 104),
            accent_hover: c(164, 228, 126),
            on_accent: c(20, 40, 22),
            soft: c(44, 78, 54),
            water: c(104, 176, 222),
            plants: c(150, 214, 104),
            animals: c(236, 170, 92),
            warn: c(250, 130, 100),
            border: c(50, 82, 58),
            sky: [0.62, 0.75, 0.84],
        },
    }
}

#[derive(Resource, Clone, Copy)]
pub struct ActiveTheme(pub Theme, pub Palette);

impl Default for ActiveTheme {
    fn default() -> Self {
        let t = std::env::var("GEVOLUTION_THEME")
            .ok()
            .and_then(|s| Theme::from_name(&s))
            .unwrap_or(Theme::Meadow);
        ActiveTheme(t, palette(t))
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
