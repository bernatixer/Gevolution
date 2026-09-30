//! Player interface: controls, budgets, inspection, lineage, experiments, tools, guide.

use crate::charts::{self, PALETTE, Series};
use crate::editor::{self, EditorState};
use crate::sim::{Snapshot, Speed, ToSim};
use crate::view::{Overlay, Tool, ViewMode, ViewState};
use crate::{ClientState, SimLink};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::egui::{self, Color32, RichText};
use bevy_egui::{EguiContexts, egui::Ui};
use sim_core::commands::CommandKind;
use sim_core::state::Origin;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    World,
    Lineage,
    Laws,
    Tools,
}

#[derive(Resource)]
pub struct UiState {
    pub tab: Tab,
    pub new_region: String,
    pub param_filter: String,
    pub set_field: String,
    pub set_field_value: f64,
    pub inspector_open: bool,
    last_selection: (Option<usize>, Option<u64>),
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            tab: Tab::World,
            new_region: "my_region".into(),
            param_filter: String::new(),
            set_field: "conductance_factor".into(),
            set_field_value: 0.0,
            inspector_open: false,
            last_selection: (None, None),
        }
    }
}

fn fmt_time(t: f64) -> String {
    let t = t.max(0.0) as u64;
    if t >= 3600 {
        format!("{}h {:02}m {:02}s", t / 3600, t / 60 % 60, t % 60)
    } else {
        format!("{:02}m {:02}s", t / 60, t % 60)
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
        format!("{v:.4}")
    }
}

#[allow(clippy::too_many_arguments)]
pub fn ui_system(
    mut contexts: EguiContexts,
    mut state: ResMut<ClientState>,
    mut view: ResMut<ViewState>,
    mut uis: ResMut<UiState>,
    mut editor: ResMut<EditorState>,
    link: Res<SimLink>,
    window: Single<&Window, With<PrimaryWindow>>,
) -> Result {
    let ctx = contexts.ctx_mut()?.clone();
    let Some(s) = state.snapshot.clone() else {
        egui::Window::new("Loading").show(&ctx, |ui| ui.label("Building the world…"));
        return Ok(());
    };
    if view.region.is_empty()
        && let Some(r) = s.plan.regions.first()
    {
        view.region = r.clone();
    }
    // Terrain always shows the natural look; region painting reveals the mask being painted.
    view.overlay = if matches!(view.tool, Tool::PaintRegion | Tool::EraseRegion) {
        Overlay::Region
    } else {
        Overlay::Natural
    };
    if (view.selected_cell, view.selected_entity) != uis.last_selection {
        uis.last_selection = (view.selected_cell, view.selected_entity);
        if view.selected_cell.is_some() {
            uis.inspector_open = true;
        }
    }
    let tx = link.0.tx.clone();
    let send = |m: ToSim| {
        let _ = tx.send(m);
    };
    let mut root = Ui::new(
        ctx.clone(),
        "root".into(),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(ctx.viewport_rect()),
    );

    // ---- Top bar ----
    egui::Panel::top("top").show(&mut root, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.heading("Evolving Worlds");
            ui.separator();
            ui.monospace(format!("{}", fmt_time(s.time)));
            ui.separator();
            for (sp, label, tip) in [
                (Speed::Paused, "⏸", "Pause"),
                (Speed::X1, "▶ 1×", "Real time"),
                (Speed::X10, "⏩ 10×", "Ten times faster"),
                (Speed::Max, "⏭ Max", "As fast as possible"),
            ] {
                if ui.selectable_label(s.speed == sp, label).on_hover_text(tip).clicked() {
                    send(ToSim::Speed(sp));
                }
            }
            if ui.button("Step").on_hover_text("Advance one tick (0.25 s)").clicked() {
                send(ToSim::Step(1));
            }
            ui.separator();
            ui.selectable_value(&mut view.mode, ViewMode::Orbit, "3D");
            ui.selectable_value(&mut view.mode, ViewMode::TopDown, "Map");
            ui.separator();
            ui.label(format!("population {}", s.population));
            if s.pending > 0 {
                ui.label(RichText::new(format!("{} queued command(s)", s.pending)).color(Color32::YELLOW));
            }
        });
        if let Some(f) = &s.failure {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!("⚠ Paused: {f}"))
                        .color(Color32::from_rgb(255, 110, 90))
                        .strong(),
                );
                if ui
                    .button("Resume after fixing")
                    .on_hover_text("Clear the failure; the failed tick was not committed")
                    .clicked()
                {
                    send(ToSim::ClearFailure);
                }
            });
        }
        if let Some(m) = state.messages.last() {
            ui.label(RichText::new(m).small().weak());
        }
    });

    // ---- Left panel ----
    egui::Panel::left("left").resizable(true).default_size(330.0).show(&mut root, |ui| {
        ui.horizontal_wrapped(|ui| {
            for (t, l) in [
                (Tab::World, "World"),
                (Tab::Lineage, "Lineage"),
                (Tab::Laws, "Laws"),
                (Tab::Tools, "Tools"),
            ] {
                ui.selectable_value(&mut uis.tab, t, l);
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| match uis.tab {
            Tab::World => world_tab(ui, &s),
            Tab::Lineage => lineage_tab(ui, &s),
            Tab::Laws => laws_tab(ui, &s, &mut uis, &mut editor, &send),
            Tab::Tools => tools_tab(ui, &s, &mut view, &mut uis, &send),
        });
    });

    // ---- Right panel: inspector (collapsible) ----
    if uis.inspector_open {
        egui::Panel::right("right")
            .resizable(true)
            .default_size(340.0)
            .show(&mut root, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Inspector");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("▶").on_hover_text("Minimize").clicked() {
                            uis.inspector_open = false;
                        }
                    });
                });
                egui::ScrollArea::vertical().show(ui, |ui| inspector(ui, &s, &mut view, &send));
            });
    } else {
        egui::Panel::right("right_min")
            .resizable(false)
            .exact_size(34.0)
            .show(&mut root, |ui| {
                if ui.button("◀").on_hover_text("Show inspector").clicked() {
                    uis.inspector_open = true;
                }
                ui.label(RichText::new("I\nn\ns\np\ne\nc\nt").small().weak());
            });
    }

    // Remaining area is the 3D viewport.
    let central = root.available_rect_before_wrap();
    let sf = window.scale_factor();
    view.viewport = Some((
        (central.left() * sf) as u32,
        (central.top() * sf) as u32,
        (central.width() * sf) as u32,
        (central.height() * sf) as u32,
    ));

    editor::editor_window(&ctx, &mut editor, &s, &send);

    state.pointer_over_ui = ctx.is_pointer_over_egui() || ctx.egui_wants_pointer_input();
    state.keyboard_captured = ctx.egui_wants_keyboard_input();
    Ok(())
}

fn world_tab(ui: &mut Ui, s: &Snapshot) {
    ui.heading("World");
    let alive: usize = s.entities.iter().map(|e| e.len()).sum();
    ui.label(format!(
        "{alive} organisms alive · {} born · {} died",
        s.stats.births, s.stats.deaths
    ));
    let h = &s.history;
    let series = |idx: usize| -> Vec<(f64, f64)> {
        h.iter()
            .map(|x| (x.tick as f64 * s.dt, x.field_totals.get(idx).copied().unwrap_or(f64::NAN)))
            .collect()
    };
    let fi = |name: &str| s.plan.cell_fields.iter().position(|f| f.0 == name);
    if let (Some(a), Some(b), Some(c)) = (fi("surface_water"), fi("soil_water"), fi("groundwater")) {
        charts::line_chart(
            ui,
            "Water inventories (kg)",
            &[
                Series {
                    label: "surface",
                    color: PALETTE[0],
                    points: series(a),
                },
                Series {
                    label: "soil",
                    color: PALETTE[6],
                    points: series(b),
                },
                Series {
                    label: "ground",
                    color: PALETTE[5],
                    points: series(c),
                },
            ],
            110.0,
            true,
        );
    }
    if let Some(v) = fi("vegetation_biomass") {
        charts::line_chart(
            ui,
            "Vegetation biomass (kg)",
            &[Series {
                label: "vegetation",
                color: PALETTE[2],
                points: series(v),
            }],
            90.0,
            true,
        );
    }
    let pop: Vec<(f64, f64)> = h
        .iter()
        .map(|x| (x.tick as f64 * s.dt, x.population.iter().sum::<usize>() as f64))
        .collect();
    charts::line_chart(
        ui,
        "Population",
        &[Series {
            label: "organisms",
            color: PALETTE[1],
            points: pop,
        }],
        90.0,
        true,
    );
    if !s.stats.deaths_by_reason.is_empty() {
        ui.label(format!(
            "death reasons: {}",
            s.stats
                .deaths_by_reason
                .iter()
                .map(|(k, v)| format!("{k} {v}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for (f, n) in &s.stats.range_violations {
        if *n > 0 {
            ui.label(RichText::new(format!("range diagnostic: {n} cells of {f} outside declared range")).color(Color32::YELLOW));
        }
    }
    ui.collapsing("Resource budgets", |ui| {
        ui.label(
            RichText::new(
                "Tracked totals follow total_next = total_now + explicit external inputs − outputs. Errors are unexplained drift.",
            )
            .small(),
        );
        egui::Grid::new("ledger").striped(true).show(ui, |ui| {
            ui.strong("resource");
            ui.strong("total");
            ui.strong("tick error / tol.");
            ui.strong("cumulative err.");
            ui.end_row();
            for l in &s.ledger {
                ui.label(&l.resource);
                ui.monospace(fmt(l.total));
                ui.monospace(format!("{} / {}", fmt(l.last_tick_error), fmt(l.last_tolerance)));
                ui.monospace(fmt(l.cumulative_error));
                ui.end_row();
            }
        });
        ui.collapsing("External accounts (open boundaries and toy exchanges)", |ui| {
            egui::Grid::new("accounts").striped(true).show(ui, |ui| {
                ui.strong("account");
                ui.strong("in");
                ui.strong("out");
                ui.end_row();
                for (id, res, label, i, o) in &s.accounts {
                    ui.label(format!("{id} ({res})")).on_hover_text(label);
                    ui.monospace(fmt(*i));
                    ui.monospace(fmt(*o));
                    ui.end_row();
                }
                for (r, i, o, ro) in &s.interventions {
                    if *i != 0.0 || *o != 0.0 {
                        ui.label(format!("interventions ({r})"));
                        ui.monospace(fmt(*i));
                        ui.monospace(fmt(*o));
                        ui.end_row();
                    }
                    if *ro != 0.0 {
                        ui.label(format!("roundoff corrections ({r})"));
                        ui.monospace(fmt(*ro));
                        ui.label("");
                        ui.end_row();
                    }
                }
            });
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
            "tick {:.2} ms\n snapshot {:.2}  eval {:.2}  resolve {:.2}\n receipts {:.2}  integrate {:.2}  lifecycle {:.2}\n validate {:.2}  commit {:.2}",
            t.total, t.snapshot, t.evaluation, t.resolution, t.receipts, t.integration, t.lifecycle, t.validation, t.commit
        ));
        ui.monospace(format!("plan: {} instructions, ~{:.1} M element-ops/tick", s.plan.instructions, s.plan.element_ops as f64 / 1e6));
    });
}

fn lineage_tab(ui: &mut Ui, s: &Snapshot) {
    for (ai, (arch, traits)) in s.plan.archetypes.iter().enumerate() {
        ui.heading(arch);
        let ents = &s.entities[ai];
        let (mut seeded, mut born, mut intervened, mut authored) = (0, 0, 0, 0);
        for e in ents {
            match e.origin {
                Origin::Random => seeded += 1,
                Origin::Authored => authored += 1,
                Origin::Born => born += 1,
                Origin::Intervention => intervened += 1,
            }
        }
        ui.label(format!(
            "alive {}: born in-world {born}, random seed genomes {seeded}, authored control genomes {authored}, interventions {intervened}",
            ents.len()
        ));
        if authored > 0 {
            ui.label(
                RichText::new("Authored genomes are labeled control fixtures, not evolved results.")
                    .small()
                    .color(Color32::YELLOW),
            );
        }
        let generation: Vec<(f64, f64)> = s
            .history
            .iter()
            .map(|x| (x.tick as f64 * s.dt, x.mean_generation.get(ai).copied().unwrap_or(f64::NAN)))
            .collect();
        charts::line_chart(
            ui,
            "Mean generation",
            &[Series {
                label: "generation",
                color: PALETTE[3],
                points: generation,
            }],
            70.0,
            true,
        );
        ui.label(RichText::new("Inherited trait distributions: seed generation vs. born later").strong());
        for (t, (name, lo, hi)) in traits.iter().enumerate() {
            let mut first = [0u32; 10];
            let mut later = [0u32; 10];
            for e in ents {
                let v = e.traits[t] as f64;
                let b = (((v - lo) / (hi - lo).max(1e-12)) * 10.0).floor().clamp(0.0, 9.0) as usize;
                if e.generation == 0 { first[b] += 1 } else { later[b] += 1 }
            }
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width() / 2.0 - 4.0);
                    charts::histogram(ui, &format!("{name} (gen 0)"), &first, *lo, *hi, PALETTE[7], 44.0);
                });
                ui.vertical(|ui| charts::histogram(ui, &format!("{name} (born)"), &later, *lo, *hi, PALETTE[t % 6], 44.0));
            });
            let means: Vec<(f64, f64)> = s
                .history
                .iter()
                .map(|x| {
                    (
                        x.tick as f64 * s.dt,
                        x.trait_means.get(ai).and_then(|m| m.get(t)).copied().unwrap_or(f64::NAN),
                    )
                })
                .collect();
            charts::line_chart(
                ui,
                &format!("mean {name}"),
                &[Series {
                    label: name,
                    color: PALETTE[t % 6],
                    points: means,
                }],
                50.0,
                false,
            );
        }
    }
}

fn laws_tab(ui: &mut Ui, s: &Snapshot, uis: &mut UiState, editor: &mut EditorState, send: &dyn Fn(ToSim)) {
    ui.heading("Laws");
    ui.label(
        RichText::new("Laws are typed graphs. Parameters here change a value inside its validated range; the editor changes formulas.")
            .small(),
    );
    if ui.button("Open law editor").clicked() {
        editor.open(s);
    }
    ui.label(format!(
        "{} instructions, {} effects in the active plan",
        s.plan.instructions,
        s.plan.effects.len()
    ));
    for w in &s.plan.warnings {
        ui.label(RichText::new(w).small().color(Color32::YELLOW));
    }
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("filter");
        ui.text_edit_singleline(&mut uis.param_filter);
    });
    for p in &s.plan.params {
        if !uis.param_filter.is_empty() && !p.name.contains(&uis.param_filter) && !p.label.contains(&uis.param_filter) {
            continue;
        }
        let mut v = p.value;
        ui.horizontal(|ui| {
            let resp = ui.add(
                egui::Slider::new(&mut v, p.min..=p.max)
                    .logarithmic(p.min >= 0.0 && p.max / p.min.max(1e-12) > 100.0)
                    .text(&p.unit),
            );
            ui.label(RichText::new(if p.label.is_empty() { &p.name } else { &p.label }).small())
                .on_hover_text(&p.name);
            if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                send(ToSim::Submit(CommandKind::SetParam {
                    name: p.name.clone(),
                    value: v,
                }));
            }
        });
    }
}

fn tools_tab(ui: &mut Ui, s: &Snapshot, view: &mut ViewState, uis: &mut UiState, send: &dyn Fn(ToSim)) {
    ui.heading("Tools");
    ui.label(
        RichText::new("Interventions are explicit sources or sinks recorded in the ledger; they never masquerade as law edits.").small(),
    );
    for (t, l) in [
        (Tool::Inspect, "Inspect (click a cell or organism)"),
        (Tool::PaintRegion, "Paint region"),
        (Tool::EraseRegion, "Erase region"),
        (Tool::AddWater, "Add water (intervention)"),
        (Tool::SpawnOrganisms, "Spawn organisms (intervention)"),
    ] {
        ui.radio_value(&mut view.tool, t, l);
    }
    ui.add(egui::Slider::new(&mut view.brush_radius, 1.0..=40.0).text("brush radius (cells)"));
    ui.separator();
    ui.label("Region");
    egui::ComboBox::from_id_salt("region")
        .selected_text(view.region.clone())
        .show_ui(ui, |ui| {
            for r in &s.plan.regions {
                ui.selectable_value(&mut view.region, r.clone(), r);
            }
        });
    ui.horizontal(|ui| {
        ui.text_edit_singleline(&mut uis.new_region);
        if ui.button("Create region").clicked() {
            send(ToSim::Submit(CommandKind::CreateRegion {
                id: uis.new_region.clone(),
            }));
            view.region = uis.new_region.clone();
        }
    });
    ui.label(
        RichText::new("Region membership is a mask, not a biome. Rules scoped to it still conserve resources across its edge.").small(),
    );
    ui.separator();
    ui.add(
        egui::DragValue::new(&mut view.water_amount)
            .prefix("water per cell: ")
            .suffix(" kg")
            .range(-1e6..=1e6),
    );
    ui.add(
        egui::DragValue::new(&mut view.spawn_count)
            .prefix("organisms per click: ")
            .range(1..=500),
    );
    ui.separator();
    ui.label("Edit a parameter field in the brush (e.g. make ground impermeable)");
    egui::ComboBox::from_id_salt("setfield")
        .selected_text(uis.set_field.clone())
        .show_ui(ui, |ui| {
            for (f, _, pol) in &s.plan.cell_fields {
                if pol == "Parameter" && f != "elevation" {
                    ui.selectable_value(&mut uis.set_field, f.clone(), f);
                }
            }
        });
    ui.add(egui::DragValue::new(&mut uis.set_field_value).prefix("value: ").speed(0.01));
    if let Some(c) = view.selected_cell {
        if ui.button("Apply to brush at selected cell").clicked() {
            let (cx, cz) = ((c % s.width) as f64 + 0.5, (c / s.width) as f64 + 0.5);
            send(ToSim::Submit(CommandKind::SetField {
                field: uis.set_field.clone(),
                shape: sim_core::schema::RegionShape::Circle {
                    cx,
                    cz,
                    radius: view.brush_radius as f64,
                    feather: 0.0,
                },
                value: uis.set_field_value,
            }));
        }
    } else {
        ui.label(RichText::new("select a cell first").small());
    }
    ui.separator();
}

fn inspector(ui: &mut Ui, s: &Snapshot, view: &mut ViewState, send: &dyn Fn(ToSim)) {
    let Some(c) = view.selected_cell else {
        ui.label("Select a cell or organism with the Inspect tool.");
        return;
    };
    let (x, z) = (c % s.width, c / s.width);
    ui.label(format!("cell ({x}, {z})  #{c}  elevation {:.1} m", s.elevation[c]));
    egui::Grid::new("cellvals").striped(true).show(ui, |ui| {
        for (i, (f, unit, pol)) in s.plan.cell_fields.iter().enumerate() {
            if ui
                .selectable_label(view.inspect_field == *f, f)
                .on_hover_text(format!("{pol} field"))
                .clicked()
            {
                view.inspect_field = f.clone();
                send(ToSim::SelectCell(Some(c), f.clone()));
            }
            ui.monospace(format!("{} {unit}", fmt(s.fields[i][c] as f64)));
            ui.end_row();
        }
        for (i, r) in s.plan.regions.iter().enumerate() {
            let v = s.regions[i][c];
            if v > 0.0 {
                ui.label(format!("region {r}"));
                ui.monospace(format!("coverage {v:.2}"));
                ui.end_row();
            }
        }
    });
    if let Some(e) = &s.cell {
        ui.separator();
        ui.label(RichText::new(format!("Explain {} ({}), last tick", e.field, e.policy)).strong());
        egui::Grid::new("explain").striped(true).show(ui, |ui| {
            ui.label("previous tick");
            ui.monospace(format!("{} {}", fmt(e.previous), e.unit));
            ui.label("");
            ui.end_row();
            for k in &e.contributions {
                let short: String = if k.label.chars().count() > 30 {
                    format!("{}…", k.label.chars().take(30).collect::<String>())
                } else {
                    k.label.clone()
                };
                ui.label(short)
                    .on_hover_text(format!("{}\neffect {} of rule {}", k.label, k.effect, k.rule));
                ui.monospace(format!("{:+}", Fmt(k.accepted)));
                let lim = if k.min_factor < 1.0 {
                    format!(
                        "requested {:+} ({:.0}% accepted){}",
                        Fmt(k.requested),
                        k.min_factor * 100.0,
                        k.limited_by.as_ref().map(|l| format!("; limited by {l}")).unwrap_or_default()
                    )
                } else {
                    "fully accepted".into()
                };
                ui.label(RichText::new(lim).small());
                ui.end_row();
            }
            if e.other != 0.0 {
                ui.label("lifecycle deposits");
                ui.monospace(format!("{:+}", Fmt(e.other)));
                ui.label("");
                ui.end_row();
            }
            ui.label(RichText::new("this tick").strong());
            ui.monospace(format!("{} {}", fmt(e.current), e.unit));
            ui.label("");
            ui.end_row();
        });
    }
    if s.cell_history.len() > 1 {
        let h = &s.cell_history;
        charts::line_chart(
            ui,
            &format!("{} at this cell, recent ticks", view.inspect_field),
            &[Series {
                label: "value",
                color: PALETTE[0],
                points: h.iter().map(|x| (x.0, x.1)).collect(),
            }],
            70.0,
            false,
        );
        charts::line_chart(
            ui,
            "accepted in / out per tick",
            &[
                Series {
                    label: "in",
                    color: PALETTE[2],
                    points: h.iter().map(|x| (x.0, x.2)).collect(),
                },
                Series {
                    label: "out",
                    color: PALETTE[4],
                    points: h.iter().map(|x| (x.0, x.3)).collect(),
                },
            ],
            70.0,
            true,
        );
    }
    if let Some(o) = &s.entity {
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Organism {}", o.id)).strong());
            if !o.alive {
                ui.label(RichText::new("died this tick").color(Color32::from_rgb(255, 110, 90)));
            }
            if ui.small_button("deselect").clicked() {
                view.selected_entity = None;
                send(ToSim::SelectEntity(None));
            }
        });
        ui.label(format!(
            "generation {} · parent {} · lineage root {} · origin {:?} · offspring {} · age {:.0} s",
            o.lineage.generation, o.lineage.parent, o.lineage.root, o.lineage.origin, o.lineage.offspring, o.age
        ));
        ui.label(RichText::new("Current condition (transient state)").strong());
        egui::Grid::new("ofields").striped(true).show(ui, |ui| {
            for (f, now, prev) in &o.fields {
                ui.label(f);
                ui.monospace(fmt(*now));
                ui.label(RichText::new(format!("{:+}", Fmt(now - prev))).small());
                ui.end_row();
            }
        });
        ui.label(RichText::new("Inherited traits (genome)").strong());
        egui::Grid::new("otraits").striped(true).show(ui, |ui| {
            for (t, v) in &o.traits {
                ui.label(t);
                ui.monospace(fmt(*v));
                ui.end_row();
            }
        });
        ui.collapsing("Sensed inputs and brain outputs", |ui| {
            for (n, v) in &o.inputs {
                ui.monospace(format!("{:>34} {:+.3}", n.rsplit('/').next().unwrap_or(n), v));
            }
            let names = ["turn", "forward", "eat", "drink", "reproduce", "rest"];
            for (i, v) in o.outputs.iter().enumerate() {
                ui.monospace(format!("{:>34} {:+.3}", names.get(i).copied().unwrap_or("out"), v));
            }
        });
        ui.label(format!(
            "attempted speed {:.2} m/s → actual {:.2} m/s",
            o.attempted_speed, o.actual_speed
        ));
        ui.collapsing("Processes (requested vs accepted)", |ui| {
            for p in &o.processes {
                ui.label(RichText::new(format!("{} — {:.0}%", p.label, p.factor * 100.0)).strong())
                    .on_hover_text(&p.effect);
                for (r, q, a) in &p.legs {
                    ui.monospace(format!("   {r}: requested {} accepted {}", fmt(*q), fmt(*a)));
                }
                if let Some(l) = &p.limited_by {
                    ui.label(RichText::new(format!("   limited by {l}")).small());
                }
            }
        });
    }
}

struct Fmt(f64);
impl std::fmt::Display for Fmt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = fmt(self.0.abs());
        if f.sign_plus() {
            write!(f, "{}{s}", if self.0 < 0.0 { "−" } else { "+" })
        } else {
            write!(f, "{s}")
        }
    }
}
