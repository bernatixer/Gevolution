//! Minimal time-series and histogram charts drawn with the egui painter.

use bevy_egui::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Ui, vec2};

pub const PALETTE: [Color32; 8] = [
    Color32::from_rgb(66, 133, 244),
    Color32::from_rgb(234, 124, 42),
    Color32::from_rgb(52, 168, 83),
    Color32::from_rgb(171, 71, 188),
    Color32::from_rgb(219, 68, 55),
    Color32::from_rgb(0, 150, 150),
    Color32::from_rgb(150, 120, 40),
    Color32::from_rgb(120, 120, 120),
];

pub struct Series<'a> {
    pub label: &'a str,
    pub color: Color32,
    pub points: Vec<(f64, f64)>,
}

fn fmt(v: f64) -> String {
    let a = v.abs();
    if a >= 1e5 || (a < 1e-2 && a > 0.0) {
        format!("{v:.2e}")
    } else if a >= 100.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.2}")
    }
}

/// Line chart with shared x axis (seconds) and a y range from the data.
pub fn line_chart(ui: &mut Ui, title: &str, series: &[Series], height: f32, zero_based: bool) {
    ui.label(egui::RichText::new(title).strong());
    let (resp, painter) = ui.allocate_painter(vec2(ui.available_width(), height), Sense::hover());
    let r = resp.rect;
    painter.rect_stroke(
        r,
        2.0,
        Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
        egui::StrokeKind::Inside,
    );
    let pts: Vec<&(f64, f64)> = series.iter().flat_map(|s| s.points.iter()).filter(|p| p.1.is_finite()).collect();
    if pts.len() < 2 {
        painter.text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            "not enough samples yet",
            egui::FontId::proportional(11.0),
            ui.visuals().weak_text_color(),
        );
        return;
    }
    let (mut x0, mut x1, mut y0, mut y1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    for p in &pts {
        x0 = x0.min(p.0);
        x1 = x1.max(p.0);
        y0 = y0.min(p.1);
        y1 = y1.max(p.1);
    }
    if zero_based {
        y0 = y0.min(0.0);
    }
    if y1 - y0 < 1e-12 {
        y1 += 1.0;
        y0 -= 1.0;
    }
    let plot = Rect::from_min_max(r.min + vec2(4.0, 14.0), r.max - vec2(4.0, 14.0));
    let map = |x: f64, y: f64| {
        Pos2::new(
            plot.left() + ((x - x0) / (x1 - x0).max(1e-12)) as f32 * plot.width(),
            plot.bottom() - ((y - y0) / (y1 - y0)) as f32 * plot.height(),
        )
    };
    for s in series {
        let line: Vec<Pos2> = s.points.iter().filter(|p| p.1.is_finite()).map(|p| map(p.0, p.1)).collect();
        if line.len() > 1 {
            painter.add(egui::Shape::line(line, Stroke::new(1.6, s.color)));
        }
    }
    let small = egui::FontId::proportional(10.0);
    let weak = ui.visuals().weak_text_color();
    painter.text(r.left_top() + vec2(4.0, 1.0), egui::Align2::LEFT_TOP, fmt(y1), small.clone(), weak);
    painter.text(
        r.left_bottom() + vec2(4.0, -1.0),
        egui::Align2::LEFT_BOTTOM,
        format!("{}   t {:.0}–{:.0} s", fmt(y0), x0, x1),
        small.clone(),
        weak,
    );
    // Legend with direct labels.
    let mut x = r.right() - 4.0;
    for s in series.iter().rev() {
        let g = painter.layout_no_wrap(s.label.to_string(), small.clone(), s.color);
        x -= g.size().x;
        painter.galley(Pos2::new(x, r.top() + 1.0), g, s.color);
        x -= 10.0;
    }
    if let Some(h) = resp.hover_pos() {
        let t = x0 + ((h.x - plot.left()) / plot.width()) as f64 * (x1 - x0);
        painter.vline(h.x, plot.y_range(), Stroke::new(1.0, weak));
        let mut txt = format!("t = {t:.0} s");
        for s in series {
            if let Some(p) = s
                .points
                .iter()
                .min_by(|a, b| (a.0 - t).abs().partial_cmp(&(b.0 - t).abs()).unwrap())
            {
                txt.push_str(&format!("\n{}: {}", s.label, fmt(p.1)));
            }
        }
        resp.on_hover_text(txt);
    }
}

pub fn histogram(ui: &mut Ui, title: &str, bins: &[u32], lo: f64, hi: f64, color: Color32, height: f32) {
    ui.label(title);
    let (resp, painter) = ui.allocate_painter(vec2(ui.available_width(), height), Sense::hover());
    let r = resp.rect;
    let max = bins.iter().copied().max().unwrap_or(0).max(1) as f32;
    let bw = r.width() / bins.len() as f32;
    for (i, b) in bins.iter().enumerate() {
        let hgt = (*b as f32 / max) * (r.height() - 12.0);
        let x = r.left() + i as f32 * bw;
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(x + 1.0, r.bottom() - 12.0 - hgt),
                Pos2::new(x + bw - 1.0, r.bottom() - 12.0),
            ),
            1.0,
            color,
        );
    }
    let small = egui::FontId::proportional(10.0);
    let weak = ui.visuals().weak_text_color();
    painter.text(r.left_bottom(), egui::Align2::LEFT_BOTTOM, fmt(lo), small.clone(), weak);
    painter.text(r.right_bottom(), egui::Align2::RIGHT_BOTTOM, fmt(hi), small, weak);
    resp.on_hover_text(format!("{} organisms; counts per bin: {bins:?}", bins.iter().sum::<u32>()));
}
