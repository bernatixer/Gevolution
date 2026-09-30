//! Player interface: the world fills the window; a light HUD floats over it
//! (glance stats, time, views, tool dock) with Overview, Laws and Inspector as
//! floating windows and the under-the-hood details in a settings modal.

use crate::charts::{self, Series};
use crate::editor::{self, EditorState};
use crate::sim::{Snapshot, Speed, ToSim};
use crate::theme::{self, ActiveTheme, Palette};
use crate::view::{Tool, ViewMode, ViewState};
use crate::{ClientState, SimLink};
use bevy::prelude::*;
use bevy_egui::egui::{self, Color32, CornerRadius, RichText, Stroke};
use bevy_egui::{EguiContexts, egui::Ui};
use std::collections::BTreeMap;

#[derive(Resource)]
pub struct UiState {
    pub overview_open: bool,
    pub laws_open: bool,
    pub settings_open: bool,
    pub theme_applied: bool,
}

impl Default for UiState {
    fn default() -> Self {
        UiState { overview_open: true, laws_open: false, settings_open: false, theme_applied: false }
    }
}

fn fmt_time(t: f64) -> String {
    let t = t.max(0.0) as u64;
    if t >= 3600 {
        format!("{}h {:02}m", t / 3600, t / 60 % 60)
    } else {
        format!("{:02}m {:02}s", t / 60, t % 60)
    }
}

/// Compact human numbers: 1234 -> "1.2k", 3.4e6 -> "3.4M".
fn human(v: f64) -> String {
    let a = v.abs();
    if !v.is_finite() {
        "—".into()
    } else if a >= 1e9 {
        format!("{:.1}B", v / 1e9)
    } else if a >= 1e6 {
        format!("{:.1}M", v / 1e6)
    } else if a >= 1e4 {
        format!("{:.1}k", v / 1e3)
    } else if a >= 100.0 {
        format!("{v:.0}")
    } else if a >= 1.0 {
        format!("{v:.1}")
    } else if a == 0.0 {
        "0".into()
    } else {
        format!("{v:.2}")
    }
}

fn fmt(v: f64) -> String {
    let a = v.abs();
    if !v.is_finite() {
        "—".into()
    } else if a == 0.0 {
        "0".into()
    } else if !(1e-3..1e6).contains(&a) {
        format!("{v:.3e}")
    } else if a >= 100.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.3}")
    }
}

fn card<R>(ui: &mut Ui, p: &Palette, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(p.card)
        .corner_radius(CornerRadius::same(12))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A pill-shaped toggle button.
fn pill(ui: &mut Ui, p: &Palette, on: bool, label: &str) -> egui::Response {
    let text = RichText::new(label).color(if on { p.on_accent } else { p.text });
    let b = egui::Button::new(text)
        .fill(if on { p.accent } else { p.soft })
        .corner_radius(CornerRadius::same(16));
    ui.add(b)
}

fn field_of<'a>(s: &'a Snapshot, name: &str) -> Option<&'a [f32]> {
    s.plan.cell_fields.iter().position(|f| f.0 == name).map(|i| &s.fields[i][..])
}

/// Floating HUD surface: the world stays visible around it.
fn hud(p: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.panel.gamma_multiply(0.94))
        .corner_radius(CornerRadius::same(18))
        .inner_margin(egui::Margin::symmetric(10, 6))
        .stroke(Stroke::new(1.0, p.border))
        .shadow(egui::Shadow { offset: [0, 3], blur: 12, spread: 0, color: Color32::from_black_alpha(30) })
}

fn chip(ui: &mut Ui, color: Color32, value: String, tip: &str) -> egui::Response {
    let b = egui::Button::new(RichText::new(value).size(15.0).strong().color(color)).frame(false);
    ui.add(b).on_hover_text(tip)
}

pub fn ui_system(
    mut contexts: EguiContexts,
    mut state: ResMut<ClientState>,
    mut view: ResMut<ViewState>,
    mut uis: ResMut<UiState>,
    mut editor: ResMut<EditorState>,
    active: Res<ActiveTheme>,
    link: Res<SimLink>,
) -> Result {
    let ctx = contexts.ctx_mut()?.clone();
    if !uis.theme_applied {
        theme::apply(&ctx, &active.0);
        uis.theme_applied = true;
    }
    let p = active.0;
    let Some(s) = state.snapshot.clone() else {
        egui::Window::new("Gevolution").show(&ctx, |ui| ui.label("Growing a world…"));
        return Ok(());
    };
    let tx = link.0.tx.clone();
    let send = |m: ToSim| {
        let _ = tx.send(m);
    };
    let screen = ctx.viewport_rect();
    let paused = s.speed == Speed::Paused;

    // ---- Top left: name and the world at a glance ----
    egui::Area::new("hud_left".into()).fixed_pos(screen.min + egui::vec2(12.0, 12.0)).show(&ctx, |ui| {
        hud(&p).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("🌱 Gevolution").size(19.0).strong().color(p.accent));
                ui.add_space(8.0);
                let total = |name: &str| field_of(&s, name).map(|v| v.iter().map(|x| *x as f64).sum::<f64>()).unwrap_or(0.0);
                let cover = field_of(&s, "vegetation_biomass")
                    .map(|v| v.iter().map(|b| (*b / (*b + 500.0)) as f64).sum::<f64>() / (s.width * s.height) as f64)
                    .unwrap_or(0.0);
                let chips = [
                    (p.animals, format!("🐾 {}", s.population), "Animals alive"),
                    (p.plants, format!("🌿 {:.0}%", cover * 100.0), "Share of the land covered by plants"),
                    (p.water, format!("💧 {} m³", human(total("surface_water") / 1000.0)), "Open water in lakes and rivers"),
                ];
                for (c, v, tip) in chips {
                    if chip(ui, c, v, &format!("{tip}. Click for trends.")).clicked() {
                        uis.overview_open = !uis.overview_open;
                    }
                }
            });
        });
    });

    // ---- Top centre: time ----
    egui::Area::new("hud_time".into())
        .pivot(egui::Align2::CENTER_TOP)
        .fixed_pos(egui::pos2(screen.center().x, screen.min.y + 12.0))
        .show(&ctx, |ui| {
            hud(&p).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(fmt_time(s.time)).monospace().color(p.weak));
                    ui.add_space(4.0);
                    for (sp, label, tip) in
                        [(Speed::Paused, "⏸", "Pause"), (Speed::X1, "▶", "Play"), (Speed::X10, "⏩", "Fast"), (Speed::Max, "⏭", "Fastest")]
                    {
                        if pill(ui, &p, s.speed == sp, label).on_hover_text(tip).clicked() {
                            send(ToSim::Speed(sp));
                        }
                    }
                });
            });
            if let Some(f) = &s.failure {
                hud(&p).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(format!("The world paused: {}", f.message)).color(p.warn).strong());
                        if ui.button("Resume").clicked() {
                            send(ToSim::ClearFailure);
                        }
                    });
                });
            }
        });

    // ---- Top right: views, panels, settings ----
    egui::Area::new("hud_right".into())
        .pivot(egui::Align2::RIGHT_TOP)
        .fixed_pos(egui::pos2(screen.max.x - 12.0, screen.min.y + 12.0))
        .show(&ctx, |ui| {
            hud(&p).show(ui, |ui| {
                ui.horizontal(|ui| {
                    if pill(ui, &p, uis.overview_open, "📊 Overview").clicked() {
                        uis.overview_open = !uis.overview_open;
                    }
                    if pill(ui, &p, uis.laws_open, "📜 Laws").clicked() {
                        uis.laws_open = !uis.laws_open;
                    }
                    ui.separator();
                    if pill(ui, &p, view.mode == ViewMode::Orbit, "3D").clicked() {
                        view.mode = ViewMode::Orbit;
                    }
                    if pill(ui, &p, view.mode == ViewMode::TopDown, "Map").clicked() {
                        view.mode = ViewMode::TopDown;
                    }
                    ui.separator();
                    if pill(ui, &p, uis.settings_open, "⚙").on_hover_text("Under the hood: budgets, events, performance").clicked() {
                        uis.settings_open = !uis.settings_open;
                    }
                });
            });
        });

    // ---- Bottom centre: tool dock, with a hint above it ----
    let hint: Option<String> = match view.tool {
        _ if paused && s.pending > 0 => Some(format!("{} change(s) waiting: they happen when the world runs", s.pending)),
        Tool::Inspect if view.selected_cell.is_none() => Some("Click the land or an animal to look closer".into()),
        Tool::Inspect => None,
        Tool::AddWater if paused => Some("Paused: pour water now, it arrives when you press Play".into()),
        Tool::SpawnOrganisms if paused => Some("Paused: place a herd now, it appears when you press Play".into()),
        Tool::AddWater => Some("Hold the mouse on the land to pour water".into()),
        Tool::SpawnOrganisms => Some("Click the land to release a small herd".into()),
    };
    egui::Area::new("dock".into())
        .pivot(egui::Align2::CENTER_BOTTOM)
        .fixed_pos(egui::pos2(screen.center().x, screen.max.y - 16.0))
        .show(&ctx, |ui| {
            ui.vertical_centered(|ui| {
                if let Some(h) = hint {
                    hud(&p).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(h).color(p.text));
                            if paused {
                                let b = egui::Button::new(RichText::new("▶ Play").color(p.on_accent)).fill(p.accent).corner_radius(CornerRadius::same(14));
                                if ui.add(b).clicked() {
                                    send(ToSim::Speed(Speed::X1));
                                }
                            }
                        });
                    });
                    ui.add_space(6.0);
                }
                hud(&p).corner_radius(CornerRadius::same(24)).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (t, label, tip) in [
                            (Tool::Inspect, "🔍  Look", "Click the land or an animal to see what is happening there"),
                            (Tool::AddWater, "💧  Water", "Hold the mouse on the land to pour water"),
                            (Tool::SpawnOrganisms, "🐾  Animals", "Click the land to release a small herd"),
                        ] {
                            let on = view.tool == t;
                            let b = egui::Button::new(RichText::new(label).size(16.0).color(if on { p.on_accent } else { p.text }))
                                .fill(if on { p.accent } else { p.soft })
                                .corner_radius(CornerRadius::same(18))
                                .min_size(egui::vec2(112.0, 40.0));
                            if ui.add(b).on_hover_text(tip).clicked() {
                                view.tool = t;
                            }
                        }
                    });
                });
            });
        });

    // ---- Floating windows ----
    let max_h = (screen.height() - 190.0).max(200.0);
    egui::Window::new("📊 Overview")
        .open(&mut uis.overview_open)
        .default_pos(screen.min + egui::vec2(12.0, 70.0))
        .default_width(300.0)
        .resizable(false)
        .show(&ctx, |ui| egui::ScrollArea::vertical().max_height(max_h).min_scrolled_height(max_h.min(640.0)).show(ui, |ui| overview(ui, &s, &p)));
    egui::Window::new("📜 Laws")
        .open(&mut uis.laws_open)
        .default_pos(screen.min + egui::vec2(330.0, 70.0))
        .default_width(320.0)
        .resizable(false)
        .show(&ctx, |ui| egui::ScrollArea::vertical().max_height(max_h).min_scrolled_height(max_h.min(640.0)).show(ui, |ui| laws(ui, &s, &p, &mut editor, &send)));
    // The inspector follows the selection; closing it clears the selection, collapsing keeps it.
    let mut inspecting = view.selected_cell.is_some();
    if inspecting {
        egui::Window::new("🔍 Inspector")
            .open(&mut inspecting)
            .default_pos(egui::pos2(screen.max.x - 342.0, screen.min.y + 70.0))
            .default_width(320.0)
            .resizable(false)
            .show(&ctx, |ui| egui::ScrollArea::vertical().max_height(max_h).min_scrolled_height(max_h.min(640.0)).show(ui, |ui| inspector(ui, &s, &p, &mut view)));
        if !inspecting {
            view.selected_cell = None;
            view.selected_entity = None;
            send(ToSim::SelectCell(None, String::new()));
            send(ToSim::SelectEntity(None));
        }
    }
    if uis.settings_open {
        let m = egui::Modal::new("settings".into()).show(&ctx, |ui| {
            ui.set_width(480.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("⚙ Under the hood").size(18.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Close").clicked() {
                        uis.settings_open = false;
                    }
                });
            });
            ui.label(RichText::new("What the simulation keeps track of behind the scenes.").color(p.weak));
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(max_h).show(ui, |ui| details(ui, &s));
        });
        if m.should_close() {
            uis.settings_open = false;
        }
    }

    editor::editor_window(&ctx, &mut editor, &s, &send);

    state.pointer_over_ui = ctx.is_pointer_over_egui() || ctx.egui_wants_pointer_input();
    state.keyboard_captured = ctx.egui_wants_keyboard_input();
    Ok(())
}

fn overview(ui: &mut Ui, s: &Snapshot, p: &Palette) {
    let empty = if s.speed == Speed::Paused { "Press ▶ Play to start recording" } else { "Recording…" };
    let h = &s.history;
    let t = |x: &sim_core::world::Sample| x.tick as f64 * s.dt;
    card(ui, p, |ui| {
        charts::line_chart(
            ui,
            &format!("Animals · {:.0} born, {:.0} died", s.stats.births as f64, s.stats.deaths as f64),
            &[Series {
                label: "",
                color: p.animals,
                points: h.iter().map(|x| (t(x), x.population.iter().sum::<usize>() as f64)).collect(),
            }],
            90.0,
            true,
            empty,
        );
    });
    let fi = |name: &str| s.plan.cell_fields.iter().position(|f| f.0 == name);
    if let (Some(w), Some(v)) = (fi("surface_water"), fi("vegetation_biomass")) {
        card(ui, p, |ui| {
            charts::line_chart(
                ui,
                "Open water (m³)",
                &[Series {
                    label: "",
                    color: p.water,
                    points: h
                        .iter()
                        .map(|x| (t(x), x.field_totals.get(w).copied().unwrap_or(f64::NAN) / 1000.0))
                        .collect(),
                }],
                70.0,
                true,
                empty,
            );
            charts::line_chart(
                ui,
                "Plants (tonnes)",
                &[Series {
                    label: "",
                    color: p.plants,
                    points: h
                        .iter()
                        .map(|x| (t(x), x.field_totals.get(v).copied().unwrap_or(f64::NAN) / 1000.0))
                        .collect(),
                }],
                70.0,
                true,
                empty,
            );
        });
    }
    // Evolution, in plain words.
    card(ui, p, |ui| {
        ui.label(RichText::new("Evolution").strong());
        let (Some(first), Some(last)) = (h.first(), h.last()) else { return };
        let generation = last.mean_generation.first().copied().unwrap_or(0.0);
        if generation < 0.05 {
            ui.label(
                RichText::new(
                    "Evolution begins once animals are born in the world. Offspring inherit their parents' traits with small mutations.",
                )
                .color(p.weak),
            );
            return;
        }
        ui.label(RichText::new(format!("Average generation {generation:.1}")).color(p.weak));
        ui.add_space(4.0);
        let labels = trait_labels(s);
        for (k, (name, label)) in labels.iter().enumerate() {
            let (a, b) = (
                first.trait_means.first().and_then(|m| m.get(k)).copied(),
                last.trait_means.first().and_then(|m| m.get(k)).copied(),
            );
            let (Some(a), Some(b)) = (a, b) else { continue };
            if !a.is_finite() || !b.is_finite() {
                continue;
            }
            let change = if a.abs() > 1e-9 { (b - a) / a.abs() * 100.0 } else { 0.0 };
            let (tag, col) = if change > 1.0 {
                (format!("+{change:.0}%"), p.accent)
            } else if change < -1.0 {
                (format!("{change:.0}%"), p.animals)
            } else {
                ("steady".to_string(), p.weak)
            };
            ui.horizontal(|ui| {
                ui.label(label);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(tag).color(col).strong());
                    ui.label(RichText::new(trait_value(name, b)).color(p.weak));
                });
            });
        }
    });
}

/// (trait id, friendly label) for the first archetype.
fn trait_labels(s: &Snapshot) -> Vec<(String, String)> {
    let Some((arch, traits)) = s.plan.archetypes.first() else {
        return vec![];
    };
    let friendly = |id: &str| -> Option<&'static str> {
        Some(match id {
            "body_size" => "Body size",
            "preferred_temperature" => "Favourite temperature",
            "tolerance_breadth" => "Heat tolerance",
            "water_conservation" => "Saves water",
            "locomotion_efficiency" => "Walking efficiency",
            _ => return None,
        })
    };
    let decl = s.plan.packages.iter().flat_map(|p| &p.archetypes).find(|a| &a.id == arch);
    traits
        .iter()
        .map(|(id, _, _)| {
            let label = friendly(id)
                .map(|s| s.to_string())
                .or_else(|| {
                    decl.and_then(|d| d.traits.iter().find(|t| &t.id == id))
                        .map(|t| t.label.clone())
                        .filter(|l| !l.is_empty())
                })
                .unwrap_or_else(|| id.replace('_', " "));
            (id.clone(), label)
        })
        .collect()
}

fn trait_value(id: &str, v: f64) -> String {
    match id {
        "preferred_temperature" => format!("{:.1}°C", v - 273.15),
        "tolerance_breadth" => format!("±{v:.1}°"),
        "water_conservation" | "locomotion_efficiency" => format!("{:.0}%", v * 100.0),
        _ => format!("{v:.2}"),
    }
}

fn details(ui: &mut Ui, s: &Snapshot) {
    if !s.stats.deaths_by_reason.is_empty() {
        ui.label(RichText::new("Causes of death").strong());
        for (k, v) in &s.stats.deaths_by_reason {
            ui.label(format!("{k}: {v}"));
        }
        ui.add_space(4.0);
    }
    ui.label(RichText::new("Resource budgets").strong());
    ui.label(RichText::new("Every resource is conserved: totals change only through declared inputs and outputs.").small());
    egui::Grid::new("ledger").show(ui, |ui| {
        for l in &s.ledger {
            ui.label(&l.resource);
            ui.monospace(fmt(l.total));
            ui.label(RichText::new(format!("drift {}", fmt(l.cumulative_error))).small());
            ui.end_row();
        }
    });
    ui.collapsing("Inputs and outputs", |ui| {
        egui::Grid::new("accounts").show(ui, |ui| {
            for (id, res, label, i, o) in &s.accounts {
                ui.label(if label.is_empty() { id.as_str() } else { label.as_str() })
                    .on_hover_text(format!("{id} ({res})"));
                ui.monospace(format!("+{} −{}", fmt(*i), fmt(*o)));
                ui.end_row();
            }
        });
    });
    ui.collapsing("Recent events", |ui| {
        for e in &s.events {
            ui.label(RichText::new(format!("{:>7} {:?}", e.tick, e.kind)).small().monospace());
        }
    });
    ui.collapsing("Performance", |ui| {
        let t = &s.timings;
        ui.monospace(format!(
            "tick {:.2} ms  ({:.0} ticks/s)\nlaws {:.2} · resources {:.2} · life {:.2}",
            t.total, s.ticks_per_second, t.evaluation, t.resolution, t.lifecycle
        ));
    });
}

fn laws(ui: &mut Ui, s: &Snapshot, p: &Palette, editor: &mut EditorState, send: &dyn Fn(ToSim)) {
    card(ui, p, |ui| {
        ui.label(RichText::new("How the world works").strong());
        ui.label(
            RichText::new("Every rule of nature here is editable. Nudge the numbers below, or open the editor to rewrite a rule.")
                .color(p.weak),
        );
        ui.add_space(4.0);
        let b = egui::Button::new(RichText::new("✏ Open law editor").color(p.on_accent))
            .fill(p.accent)
            .corner_radius(CornerRadius::same(16));
        if ui.add(b).clicked() {
            editor.open(s);
        }
    });
    // Group laws by what they are about, with short names.
    let topic = |rule: &str| -> &'static str {
        if rule.starts_with("core.bio") {
            "🐾 Animals"
        } else if rule.contains("vegetation") || rule.contains("decomposition") || rule.contains("dispersal") {
            "🌿 Plants"
        } else if rule.contains("temperature") {
            "☀ Weather"
        } else {
            "💧 Water"
        }
    };
    let mut labels: BTreeMap<String, String> = BTreeMap::new();
    for pk in &s.plan.packages {
        for r in &pk.rules {
            let l = if r.label.is_empty() { r.rule_id.clone() } else { r.label.clone() };
            let short = l.split(" (").next().unwrap_or(&l).split(" with ").next().unwrap_or(&l).to_string();
            labels.insert(r.rule_id.clone(), short);
        }
    }
    let mut topics: BTreeMap<&str, BTreeMap<String, Vec<&crate::sim::ParamView>>> = BTreeMap::new();
    for pv in &s.plan.params {
        let owner = pv.name.split(':').next().unwrap_or("");
        let Some(law) = labels.get(owner) else { continue };
        topics.entry(topic(owner)).or_default().entry(law.clone()).or_default().push(pv);
    }
    let order = ["💧 Water", "🌿 Plants", "🐾 Animals", "☀ Weather"];
    let mut topics: Vec<(&str, BTreeMap<String, Vec<&crate::sim::ParamView>>)> = topics.into_iter().collect();
    topics.sort_by_key(|(t, _)| order.iter().position(|o| o == t).unwrap_or(9));
    for (t, laws) in topics {
        egui::CollapsingHeader::new(RichText::new(t).strong().size(16.0))
            .id_salt(("topic", t))
            .show(ui, |ui| {
                for (law, params) in laws {
                    egui::CollapsingHeader::new(&law).id_salt(("law", &law)).show(ui, |ui| {
                        for pv in params {
                            let mut v = pv.value;
                            ui.add(
                                egui::Label::new(
                                    RichText::new(if pv.label.is_empty() { &pv.name } else { &pv.label })
                                        .small()
                                        .color(p.weak),
                                )
                                .wrap(),
                            )
                            .on_hover_text(&pv.name);
                            let log = pv.min >= 0.0 && pv.max / pv.min.max(1e-12) > 100.0;
                            let resp = ui.add(
                                egui::Slider::new(&mut v, pv.min..=pv.max)
                                    .logarithmic(log)
                                    .suffix(format!(" {}", pv.unit)),
                            );
                            if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                                send(ToSim::Submit(sim_core::commands::CommandKind::SetParam {
                                    name: pv.name.clone(),
                                    value: v,
                                }));
                            }
                        }
                    });
                }
            });
    }
}

fn bar(ui: &mut Ui, p: &Palette, label: &str, frac: f64, color: Color32, text: String) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(p.weak));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(text));
    });
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 8.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(4), p.soft);
    let mut fill = rect;
    fill.set_width(rect.width() * frac.clamp(0.0, 1.0) as f32);
    ui.painter().rect_filled(fill, CornerRadius::same(4), color);
}

fn row(ui: &mut Ui, p: &Palette, label: &str, value: String) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(p.weak));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(value));
    });
}

fn inspector(ui: &mut Ui, s: &Snapshot, p: &Palette, view: &mut ViewState) {
    let Some(c) = view.selected_cell else { return };
    if let Some(o) = &s.entity {
        card(ui, p, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("🐾 Animal #{}", o.id)).strong().color(p.animals));
                if !o.alive {
                    ui.label(RichText::new("died").color(p.warn));
                }
            });
            ui.label(
                RichText::new(format!(
                    "generation {} · {} offspring · {:.0} s old",
                    o.lineage.generation, o.lineage.offspring, o.age
                ))
                .small()
                .color(p.weak),
            );
            let get = |k: &str| o.fields.iter().find(|f| f.0 == k).map(|f| f.1).unwrap_or(0.0);
            let size = o.traits.iter().find(|t| t.0 == "body_size").map(|t| t.1).unwrap_or(1.0);
            bar(ui, p, "Health", get("health"), p.accent, format!("{:.0}%", get("health") * 100.0));
            bar(
                ui,
                p,
                "Energy",
                get("energy") / (300_000.0 * size),
                p.animals,
                format!("{:.0} kJ", get("energy") / 1000.0),
            );
            bar(
                ui,
                p,
                "Water",
                get("body_water") / (20.0 * size),
                p.water,
                format!("{:.1} L", get("body_water")),
            );
            let names = ["turning", "moving", "eating", "drinking", "wanting to breed", "resting"];
            let doing: Vec<&str> = o
                .outputs
                .iter()
                .enumerate()
                .filter(|(i, v)| *i >= 1 && **v > 0.3)
                .map(|(i, _)| names[i])
                .collect();
            if !doing.is_empty() {
                ui.label(RichText::new(format!("Right now: {}", doing.join(", "))).small());
            }
            ui.add_space(4.0);
            ui.label(RichText::new("Inherited traits").strong());
            let labels = trait_labels(s);
            for (id, v) in &o.traits {
                let l = labels.iter().find(|x| &x.0 == id).map(|x| x.1.clone()).unwrap_or(id.clone());
                row(ui, p, &l, trait_value(id, *v));
            }
            egui::CollapsingHeader::new(RichText::new("What it senses and does").small())
                .id_salt("brain")
                .show(ui, |ui| {
                    for (n, v) in &o.inputs {
                        ui.label(
                            RichText::new(format!("{}: {:+.2}", n.rsplit('/').next().unwrap_or(n).replace('_', " "), v))
                                .small()
                                .monospace(),
                        );
                    }
                    for p_ in &o.processes {
                        ui.label(RichText::new(format!("{} — {:.0}% of what it asked for", p_.label, p_.factor * 100.0)).small());
                    }
                });
        });
    }
    let (x, z) = (c % s.width, c / s.width);
    card(ui, p, |ui| {
        ui.label(RichText::new("🌿 The land here").strong().color(p.plants));
        ui.label(
            RichText::new(format!("spot ({x}, {z}) · {:.0} m high", s.elevation[c]))
                .small()
                .color(p.weak),
        );
        let get = |k: &str| field_of(s, k).map(|v| v[c] as f64).unwrap_or(0.0);
        let area = s.cell_size * s.cell_size;
        let depth_mm = get("surface_water") / area;
        row(ui, p, "Temperature", format!("{:.1} °C", get("temperature") - 273.15));
        row(
            ui,
            p,
            "Water on the surface",
            if depth_mm < 0.1 {
                "dry".into()
            } else {
                format!("{depth_mm:.0} mm deep")
            },
        );
        let cap = get("soil_water_capacity");
        bar(
            ui,
            p,
            "Soil moisture",
            if cap > 0.0 { get("soil_water") / cap } else { 0.0 },
            p.water,
            format!("{:.0}%", if cap > 0.0 { get("soil_water") / cap * 100.0 } else { 0.0 }),
        );
        let veg = get("vegetation_biomass");
        bar(ui, p, "Plants", veg / 3000.0, p.plants, format!("{:.0} kg", veg));
        row(ui, p, "Groundwater", format!("{:.1} m³", get("groundwater") / 1000.0));
        row(ui, p, "Soil nutrients", format!("{:.0} kg", get("soil_nutrients")));
    });
    card(ui, p, |ui| {
        ui.label(RichText::new("History of this spot").strong());
        let h = &s.cell_history;
        if h.len() < 2 {
            ui.label(
                RichText::new(if s.speed == Speed::Paused {
                    "The world is paused. Press ▶ Play to watch how this spot changes over time."
                } else {
                    "Recording… the history fills in as time passes."
                })
                .color(p.weak),
            );
            return;
        }
        for (k, title, color) in [
            (1, "Water on the surface (mm)", p.water),
            (2, "Soil moisture (0–1)", p.water),
            (3, "Plants (kg)", p.plants),
            (4, "Temperature (°C)", p.animals),
        ] {
            charts::line_chart(
                ui,
                title,
                &[Series {
                    label: "",
                    color,
                    points: h.iter().map(|x| (x[0], x[k])).collect(),
                }],
                48.0,
                false,
                "",
            );
        }
    });
}
