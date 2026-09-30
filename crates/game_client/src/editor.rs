//! Law editor: a typed node-graph editor over the canonical JSON rule documents.
//!
//! Drafts never change the running experiment. Every edit is validated by the
//! real compiler; a valid draft is applied atomically at the next tick boundary.

use crate::sim::{Snapshot, ToSim};
use bevy::prelude::Resource;
use bevy_egui::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};
use sim_core::catalog::{self, ArgKind, Inputs};
use sim_core::commands::CommandKind;
use sim_core::compiler::{self, Diagnostic, NodeTypes, Severity};
use sim_core::graph_edit::{insert_blend, is_effect, rename_refs, set_input, sync_effects};
use sim_core::schema::{DomainDecl, Node, Package, ParamDecl, Rule, Scope};
use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

pub const PLAYER_PACKAGE: &str = "player.laws";
const NODE_W: f32 = 200.0;
const ROW: f32 = 17.0;

pub struct Report {
    pub ok: bool,
    pub diagnostics: Vec<Diagnostic>,
    pub types: NodeTypes,
    pub summary: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Resource)]
pub struct EditorState {
    pub open: bool,
    pub draft: Vec<Package>,
    pub base: Vec<Package>,
    pub base_hash: u64,
    pub sel: Option<(usize, usize)>,
    pub sel_node: Option<String>,
    undo: Vec<Vec<Package>>,
    redo: Vec<Vec<Package>>,
    pub report: Option<Report>,
    needs_check: Option<Instant>,
    scene_rect: Rect,
    drag_from: Option<String>,
    moving: Option<String>,
    inputs_pos: Vec<(String, usize, Pos2)>,
    outputs_pos: HashMap<String, Pos2>,
    pub file_path: String,
    pub message: String,
    new_rule_id: String,
    new_rule_kind: String,
    new_rule_template: String,
    blend_region: String,
    blend_port: usize,
    new_param: String,
    new_field: String,
    new_field_unit: String,
    expanded: bool,
    /// Read (plain-language) or Graph (node editor) view.
    pub graph_mode: bool,
    pending_select: Option<String>,
    last_scenario_regions: Vec<String>,
    fit_for: Option<(usize, usize)>,
}

impl Default for EditorState {
    fn default() -> Self {
        EditorState {
            open: false,
            draft: vec![],
            base: vec![],
            base_hash: 0,
            sel: None,
            sel_node: None,
            undo: vec![],
            redo: vec![],
            report: None,
            needs_check: None,
            scene_rect: Rect::from_min_size(pos2(-40.0, -40.0), vec2(1200.0, 700.0)),
            drag_from: None,
            moving: None,
            inputs_pos: vec![],
            outputs_pos: HashMap::new(),
            file_path: "saves/laws/player.laws.json".into(),
            message: String::new(),
            new_rule_id: "player.my_law".into(),
            new_rule_kind: "cells".into(),
            new_rule_template: String::new(),
            blend_region: String::new(),
            blend_port: 0,
            new_param: "k".into(),
            new_field: "toxin".into(),
            new_field_unit: "kg".into(),
            expanded: true,
            graph_mode: false,
            pending_select: None,
            last_scenario_regions: vec![],
            fit_for: None,
        }
    }
}

fn default_args(op: &str, s: &Snapshot) -> BTreeMap<String, Value> {
    let mut m = BTreeMap::new();
    let first_field = s.plan.cell_fields.first().map(|f| f.0.clone()).unwrap_or_default();
    match op {
        "const" => {
            m.insert("value".into(), json!(1.0));
            m.insert("unit".into(), json!("1"));
        }
        "parameter" => {
            m.insert("name".into(), json!(""));
        }
        "read_state" | "candidate_state" => {
            m.insert("field".into(), json!(first_field));
        }
        "read_forcing" => {
            m.insert("forcing".into(), json!("rain_flux"));
        }
        "region" | "region_mean" | "region_sum" => {
            m.insert("region".into(), json!(s.plan.regions.first().cloned().unwrap_or_default()));
        }
        "curve" => {
            m.insert("in_unit".into(), json!("1"));
            m.insert("out_unit".into(), json!("1"));
            m.insert("points".into(), json!([[0.0, 0.0], [1.0, 1.0]]));
        }
        "neighbor_sum" | "neighbor_mean" | "crowding" => {
            m.insert("radius".into(), json!(1));
        }
        "sample_offset" => {
            m.insert("forward".into(), json!(10.0));
            m.insert("lateral".into(), json!(0.0));
        }
        "trait" | "builtin" => {
            m.insert("name".into(), json!(if op == "builtin" { "age" } else { "body_size" }));
        }
        "transfer" => {
            m.insert("resource".into(), json!("water"));
            m.insert("from".into(), json!("surface_water"));
            m.insert("to".into(), json!("soil_water"));
            m.insert("rate".into(), json!(""));
        }
        "external_source" => {
            m.insert("resource".into(), json!("water"));
            m.insert("account".into(), json!("external.weather"));
            m.insert("to".into(), json!("surface_water"));
            m.insert("rate".into(), json!(""));
        }
        "external_sink" => {
            m.insert("resource".into(), json!("water"));
            m.insert("from".into(), json!("surface_water"));
            m.insert("account".into(), json!("external.atmosphere"));
            m.insert("rate".into(), json!(""));
        }
        "edge_transfer" => {
            m.insert("resource".into(), json!("water"));
            m.insert("field".into(), json!("surface_water"));
            m.insert("rate".into(), json!(""));
        }
        "reaction" => {
            m.insert(
                "legs".into(),
                json!([{ "resource": "water", "from": "surface_water", "to": "soil_water", "rate": "" }]),
            );
        }
        "rate_contribution" | "next_value" => {
            m.insert("field".into(), json!("temperature"));
            m.insert(if op == "next_value" { "value" } else { "rate" }.into(), json!(""));
        }
        "move" => {
            m.insert("turn".into(), json!(""));
            m.insert("speed".into(), json!(""));
        }
        "death" => {
            m.insert("condition".into(), json!(""));
            m.insert("reason".into(), json!("custom law"));
        }
        "birth" => {
            m.insert("condition".into(), json!(""));
            m.insert("legs".into(), json!([]));
        }
        _ => {}
    }
    if let Some(info) = catalog::info(op)
        && let Inputs::Positional(p) = info.inputs
        && !p.is_empty()
    {
        m.insert("inputs".into(), Value::Array(p.iter().map(|_| json!("")).collect()));
    }
    m
}

fn node_pos(n: &Node) -> Option<Pos2> {
    n.args
        .get("pos")
        .and_then(|p| p.as_array())
        .and_then(|a| Some(pos2(a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32)))
}

/// Topological-depth layout for nodes without stored positions.
fn layout(rule: &Rule) -> HashMap<String, Pos2> {
    let ids: HashMap<&str, usize> = rule.nodes.iter().enumerate().map(|(i, n)| (n.id.as_str(), i)).collect();
    let mut depth = vec![usize::MAX; rule.nodes.len()];
    fn d(i: usize, rule: &Rule, ids: &HashMap<&str, usize>, depth: &mut Vec<usize>, guard: usize) -> usize {
        if depth[i] != usize::MAX {
            return depth[i];
        }
        if guard > 256 {
            return 0;
        }
        let mut best = 0;
        for (_, r) in catalog::node_refs(&rule.nodes[i]) {
            let local = r.split('.').next().unwrap_or("");
            if !r.contains('/')
                && let Some(&j) = ids.get(local)
                && j != i
            {
                best = best.max(d(j, rule, ids, depth, guard + 1) + 1);
            }
        }
        depth[i] = best;
        best
    }
    for i in 0..rule.nodes.len() {
        d(i, rule, &ids, &mut depth, 0);
    }
    let mut rows: HashMap<usize, usize> = HashMap::new();
    let mut out = HashMap::new();
    for (i, n) in rule.nodes.iter().enumerate() {
        let p = node_pos(n).unwrap_or_else(|| {
            let r = rows.entry(depth[i]).or_insert(0);
            *r += 1;
            pos2(depth[i] as f32 * (NODE_W + 50.0), (*r - 1) as f32 * 120.0)
        });
        out.insert(n.id.clone(), p);
    }
    out
}

fn node_height(n: &Node) -> f32 {
    let refs = catalog::node_refs(n).len().max(match catalog::info(&n.op).map(|i| i.inputs) {
        Some(Inputs::Positional(p)) => p.len(),
        Some(Inputs::Named(p)) => p.len(),
        _ => 0,
    });
    let args = n
        .args
        .keys()
        .filter(|k| {
            !matches!(
                k.as_str(),
                "inputs" | "pos" | "legs" | "rate" | "amount" | "value" | "turn" | "speed" | "condition"
            ) || (k.as_str() == "value" && n.op == "const")
        })
        .count();
    let outs = catalog::info(&n.op).map(|i| i.outputs.len()).unwrap_or(1).max(1);
    22.0 + ROW * (refs + args.min(4) + outs) as f32 + 8.0
}

impl EditorState {
    pub fn open(&mut self, s: &Snapshot) {
        let pristine = self.undo.is_empty();
        if !self.open && (pristine || self.base_hash != s.plan.source_hash) || self.draft.is_empty() {
            self.draft = s.plan.packages.clone();
            self.base = s.plan.packages.clone();
            self.base_hash = s.plan.source_hash;
            self.undo.clear();
            self.redo.clear();
            self.needs_check = Some(Instant::now());
        }
        self.open = true;
        if self.sel.is_none() {
            self.pending_select = Some("core.env.rain".into());
        }
    }

    pub fn select_rule(&mut self, id: &str) {
        self.pending_select = Some(id.to_string());
    }

    fn edit(&mut self) {
        self.undo.push(self.draft.clone());
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.needs_check = Some(Instant::now());
    }

    fn rule(&self) -> Option<&Rule> {
        self.sel.and_then(|(p, r)| self.draft.get(p)?.rules.get(r))
    }

    fn rule_mut(&mut self) -> Option<&mut Rule> {
        let (p, r) = self.sel?;
        self.draft.get_mut(p)?.rules.get_mut(r)
    }

    fn player_package(&mut self) -> usize {
        if let Some(i) = self.draft.iter().position(|p| p.package_id == PLAYER_PACKAGE) {
            return i;
        }
        let requires = self.draft.iter().map(|p| p.package_id.clone()).collect();
        let pkg: Package = serde_json::from_value(json!({
            "schema_version": 1,
            "package_id": PLAYER_PACKAGE,
            "label": "Player-authored laws",
            "capabilities": ["core.cells.v1", "core.edges.v1", "core.entities.v1"],
            "rules": []
        }))
        .unwrap();
        let mut pkg = pkg;
        pkg.requires = requires;
        self.draft.push(pkg);
        self.draft.len() - 1
    }

    fn check(&mut self, s: &Snapshot) {
        let env = sim_core::world::compile_env(&s.plan.scenario, s.plan.regions.clone());
        let (res, types) = compiler::compile_with_types(&self.draft, &env);
        let mut summary = vec![];
        // Changed rules and what they touch.
        let index = |ps: &[Package]| -> BTreeMap<String, Rule> {
            ps.iter()
                .flat_map(|p| p.rules.iter().map(|r| (r.rule_id.clone(), r.clone())))
                .collect()
        };
        let (before, after) = (index(&self.base), index(&self.draft));
        for (id, r) in &after {
            let changed = before.get(id) != Some(r);
            if !changed {
                continue;
            }
            summary.push(format!("{} rule {id}", if before.contains_key(id) { "changed" } else { "new" }));
            for n in r.nodes.iter().filter(|n| is_effect(&n.op)) {
                let get = |k: &str| n.args.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
                let line = match n.op.as_str() {
                    "external_source" => format!("  introduces source {} → {}", get("account"), get("to")),
                    "external_sink" => format!("  introduces sink {} → {}", get("from"), get("account")),
                    "transfer" => format!("  transfers {} → {}", get("from"), get("to")),
                    "edge_transfer" => format!("  transports {} along edges", get("field")),
                    "rate_contribution" => format!("  integrates {}", get("field")),
                    "next_value" => format!("  writes next value of {}", get("field")),
                    "reaction" => "  coupled reaction".to_string(),
                    op => format!("  {op}"),
                };
                summary.push(line);
            }
        }
        for id in before.keys() {
            if !after.contains_key(id) {
                summary.push(format!("removed rule {id}"));
            }
        }
        if summary.is_empty() {
            summary.push("no changes from the active laws".into());
        }
        self.report = Some(match res {
            Ok(plan) => {
                let plan = sim_core::optimize::optimize(plan);
                Report {
                    ok: true,
                    diagnostics: vec![],
                    types,
                    summary,
                    warnings: plan.warnings.iter().map(|w| w.to_string()).collect(),
                }
            }
            Err(d) => Report {
                ok: false,
                diagnostics: d,
                types,
                summary,
                warnings: vec![],
            },
        });
    }
}

fn node_diags<'a>(rep: Option<&'a Report>, rule: &str, node: &str) -> Vec<&'a Diagnostic> {
    rep.map(|r| {
        r.diagnostics
            .iter()
            .filter(|d| d.rule.as_deref() == Some(rule) && d.node.as_deref() == Some(node))
            .collect()
    })
    .unwrap_or_default()
}

fn category_color(op: &str) -> Color32 {
    match catalog::info(op).map(|i| i.category).unwrap_or("") {
        "Values" => Color32::from_rgb(70, 90, 120),
        "Arithmetic" => Color32::from_rgb(70, 100, 80),
        "Logic" => Color32::from_rgb(110, 90, 60),
        "Spatial" => Color32::from_rgb(60, 105, 110),
        "Organisms" => Color32::from_rgb(110, 70, 110),
        "Effects" => Color32::from_rgb(140, 70, 60),
        _ => Color32::from_rgb(90, 90, 90),
    }
}

pub fn editor_window(ctx: &egui::Context, ed: &mut EditorState, s: &Snapshot, send: &dyn Fn(ToSim)) {
    if !ed.open {
        return;
    }
    if let Some(id) = ed.pending_select.take() {
        for (pi, p) in ed.draft.iter().enumerate() {
            if let Some(ri) = p.rules.iter().position(|r| r.rule_id == id) {
                ed.sel = Some((pi, ri));
                ed.sel_node = None;
            }
        }
    }
    if ed.last_scenario_regions != s.plan.regions {
        ed.last_scenario_regions = s.plan.regions.clone();
        ed.needs_check = Some(Instant::now());
    }
    if let Some(t) = ed.needs_check
        && t.elapsed().as_millis() > 200
    {
        ed.needs_check = None;
        ed.check(s);
    }
    let mut open = true;
    egui::Window::new("Laws of nature")
        .open(&mut open)
        .default_size([1300.0, 820.0])
        .resizable(true)
        .show(ctx, |ui| {
            toolbar(ui, ed, s, send);
            ui.add_space(4.0);
            egui::Panel::left("ed_rules")
                .resizable(true)
                .default_size(250.0)
                .max_size(420.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt("rules_scroll").show(ui, |ui| {
                        if ed.graph_mode {
                            rules_panel(ui, ed, s)
                        } else {
                            topic_list(ui, ed)
                        }
                    });
                });
            if ed.graph_mode {
                egui::Panel::right("ed_props").resizable(true).default_size(330.0).show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("props_scroll")
                        .show(ui, |ui| properties(ui, ed, s));
                });
                egui::CentralPanel::default().show(ui, |ui| canvas(ui, ed, s));
            } else {
                egui::CentralPanel::default().show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("read_scroll")
                        .show(ui, |ui| read_view(ui, ed, s));
                });
            }
        });
    ed.open = open;
}

fn toolbar(ui: &mut Ui, ed: &mut EditorState, s: &Snapshot, send: &dyn Fn(ToSim)) {
    let accent = ui.visuals().selection.bg_fill;
    ui.horizontal(|ui| {
        if ui.add_enabled(!ed.undo.is_empty(), egui::Button::new("Undo")).clicked()
            && let Some(prev) = ed.undo.pop()
        {
            ed.redo.push(std::mem::replace(&mut ed.draft, prev));
            ed.needs_check = Some(Instant::now());
        }
        if ui.add_enabled(!ed.redo.is_empty(), egui::Button::new("Redo")).clicked()
            && let Some(next) = ed.redo.pop()
        {
            ed.undo.push(std::mem::replace(&mut ed.draft, next));
            ed.needs_check = Some(Instant::now());
        }
        ui.separator();
        let changed = ed.draft != ed.base;
        let ok = ed.report.as_ref().is_some_and(|r| r.ok) && ed.needs_check.is_none();
        let apply = egui::Button::new(RichText::new("✔ Apply changes").color(Color32::WHITE))
            .fill(accent)
            .corner_radius(egui::CornerRadius::same(16));
        if ui
            .add_enabled(ok && changed, apply)
            .on_hover_text("Your changes take effect in the running world")
            .clicked()
        {
            send(ToSim::Submit(CommandKind::ApplyPackages {
                packages: ed.draft.clone(),
            }));
            ed.base = ed.draft.clone();
            ed.message = "Applied. Press ▶ Play if the world is paused.".into();
        }
        if ui.add_enabled(changed, egui::Button::new("Discard changes")).clicked() {
            ed.edit();
            ed.draft = s.plan.packages.clone();
            ed.base = s.plan.packages.clone();
            ed.base_hash = s.plan.source_hash;
        }
        ui.separator();
        let status = match &ed.report {
            _ if ed.needs_check.is_some() => RichText::new("checking…").weak(),
            Some(r) if !r.ok => {
                RichText::new(format!("⚠ {} problem(s) to fix before applying", r.diagnostics.len())).color(Color32::from_rgb(200, 90, 60))
            }
            _ if changed => RichText::new("Ready to apply").color(accent),
            _ => RichText::new("No changes").weak(),
        };
        ui.label(status);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button("Files", |ui| {
                ui.label(RichText::new("Save or load laws").strong());
                ui.add(egui::TextEdit::singleline(&mut ed.file_path).desired_width(240.0));
                if ui.button("Save selected law's package").clicked() {
                    let pi = ed
                        .sel
                        .map(|x| x.0)
                        .or_else(|| ed.draft.iter().position(|p| p.package_id == PLAYER_PACKAGE));
                    ed.message = match pi.and_then(|i| ed.draft.get(i)) {
                        Some(p) => {
                            let path = std::path::PathBuf::from(&ed.file_path);
                            if let Some(d) = path.parent() {
                                let _ = std::fs::create_dir_all(d);
                            }
                            match std::fs::write(&path, serde_json::to_string_pretty(p).unwrap()) {
                                Ok(()) => format!("saved to {}", path.display()),
                                Err(e) => format!("save failed: {e}"),
                            }
                        }
                        None => "select a law first".into(),
                    };
                }
                if ui.button("Load package").clicked() {
                    ed.message = match sim_core::assets::load_package(std::path::Path::new(&ed.file_path)) {
                        Ok(p) => {
                            ed.edit();
                            let id = p.package_id.clone();
                            match ed.draft.iter().position(|x| x.package_id == id) {
                                Some(i) => ed.draft[i] = p,
                                None => ed.draft.push(p),
                            }
                            format!("loaded {id}")
                        }
                        Err(e) => format!("load failed: {e}"),
                    };
                }
            });
            let label = if ed.graph_mode {
                "📖 Simple view"
            } else {
                "🔧 Graph view (advanced)"
            };
            if ui.button(label).clicked() {
                ed.graph_mode = !ed.graph_mode;
                ed.expanded = true;
            }
        });
    });
    if !ed.message.is_empty() {
        ui.label(RichText::new(&ed.message).small().weak());
    }
}

/// Short, friendly name of a rule.
pub fn short_label(r: &Rule) -> String {
    let l = if r.label.is_empty() { r.rule_id.clone() } else { r.label.clone() };
    l.split(" (").next().unwrap_or(&l).split(" with ").next().unwrap_or(&l).to_string()
}

pub fn topic_of(rule_id: &str) -> &'static str {
    if rule_id.starts_with("core.bio") {
        "🐾 Animals"
    } else if rule_id.contains("vegetation") || rule_id.contains("decomposition") || rule_id.contains("dispersal") {
        "🌿 Plants"
    } else if rule_id.contains("temperature") {
        "☀ Weather"
    } else if rule_id.starts_with("core.env") {
        "💧 Water"
    } else {
        "✨ Your laws"
    }
}

fn topic_list(ui: &mut Ui, ed: &mut EditorState) {
    let mut select = None;
    for topic in ["💧 Water", "🌿 Plants", "🐾 Animals", "☀ Weather", "✨ Your laws"] {
        let items: Vec<(usize, usize, String)> = ed
            .draft
            .iter()
            .enumerate()
            .flat_map(|(pi, p)| p.rules.iter().enumerate().map(move |(ri, r)| (pi, ri, r)))
            .filter(|(_, _, r)| topic_of(&r.rule_id) == topic)
            .map(|(pi, ri, r)| (pi, ri, short_label(r)))
            .collect();
        if items.is_empty() {
            continue;
        }
        ui.label(RichText::new(topic).strong());
        for (pi, ri, l) in items {
            if ui.selectable_label(ed.sel == Some((pi, ri)), format!("   {l}")).clicked() {
                select = Some((pi, ri));
            }
        }
        ui.add_space(6.0);
    }
    if let Some(x) = select {
        ed.sel = Some(x);
        ed.sel_node = None;
    }
}

fn humanize(id: &str) -> String {
    let id = id
        .strip_prefix("self.")
        .map(|x| format!("its {x}"))
        .unwrap_or_else(|| id.strip_prefix("cell.").unwrap_or(id).to_string());
    id.replace('_', " ")
}

/// Plain-language reading of a node reference, bounded in depth.
fn formula(rule: &Rule, all: &[Package], r: &str, depth: u32) -> String {
    if r.is_empty() {
        return "(nothing connected)".into();
    }
    if depth > 5 {
        return "…".into();
    }
    let (node_part, port) = match r.rsplit_once('.') {
        Some((n, p)) if !p.contains('/') => (n, p),
        _ => (r, "value"),
    };
    let (rule, id) = match node_part.split_once('/') {
        Some((rid, nid)) => match all.iter().flat_map(|p| &p.rules).find(|x| x.rule_id == rid) {
            Some(other) => (other, nid),
            None => return humanize(nid),
        },
        None => (rule, node_part),
    };
    let Some(n) = rule.nodes.iter().find(|n| n.id == id) else {
        return humanize(id);
    };
    if is_effect(&n.op) {
        return match port {
            "fraction" => format!("the share of \"{}\" that was possible", humanize(id)),
            _ => format!("what \"{}\" actually moved", humanize(id)),
        };
    }
    if n.op == "brain" {
        let urges = ["turning", "moving", "eating", "drinking", "breeding", "resting"];
        let k: usize = port.strip_prefix("out").and_then(|x| x.parse().ok()).unwrap_or(0);
        return format!("its urge for {}", urges.get(k).unwrap_or(&"acting"));
    }
    let arg = |k: &str| n.args.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let ins: Vec<String> = n
        .args
        .get("inputs")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().map(|x| x.as_str().unwrap_or("").to_string()).collect())
        .unwrap_or_default();
    let child = |k: usize| -> String {
        let r = ins.get(k).cloned().unwrap_or_default();
        let t = formula(rule, all, &r, depth + 1);
        let op = rule
            .nodes
            .iter()
            .find(|x| Some(x.id.as_str()) == r.split('.').next())
            .map(|x| x.op.as_str())
            .unwrap_or("");
        if matches!(op, "add" | "sub") { format!("({t})") } else { t }
    };
    match n.op.as_str() {
        "const" => {
            let v = n.args.get("value").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let u = arg("unit");
            if u == "1" || u.is_empty() {
                trim_num(v)
            } else {
                format!("{} {u}", trim_num(v))
            }
        }
        "parameter" => {
            let name = arg("name");
            let val = rule
                .parameters
                .get(&name)
                .map(|p| format!(" [{} {}]", trim_num(p.value), p.unit))
                .unwrap_or_default();
            format!("{}{val}", humanize(&name))
        }
        "read_state" => humanize(&arg("field")),
        "candidate_state" => format!("{} after this moment", humanize(&arg("field"))),
        "read_forcing" => humanize(&arg("forcing").replace("rain_flux", "rainfall")),
        "region" => format!("how much of the spot is inside {}", arg("region")),
        "cell_area" => "the area of the spot".into(),
        "cell_size" => "the size of the spot".into(),
        "is_boundary" => "being at the edge of the world".into(),
        "random" => "a random number".into(),
        "trait" => format!("its inherited {}", humanize(&arg("name"))),
        "builtin" => format!("its {}", arg("name")),
        "crowding" => "how many neighbours are close".into(),
        "add" => format!("{} + {}", child(0), child(1)),
        "sub" => format!("{} − {}", child(0), child(1)),
        "mul" | "multiply" => format!("{} × {}", child(0), child(1)),
        "safe_divide" => format!("{} ÷ {}", child(0), child(1)),
        "neg" => format!("−{}", child(0)),
        "abs" => format!("the size of {}", child(0)),
        "min" => format!("the smaller of {} and {}", child(0), child(1)),
        "max" => format!("the larger of {} and {}", child(0), child(1)),
        "clamp" => format!("{} kept between {} and {}", child(0), child(1), child(2)),
        "lerp" => format!("a blend from {} to {} by {}", child(0), child(1), child(2)),
        "pow" => format!("{} to the power {}", child(0), child(1)),
        "exp" | "ln" | "sin" | "cos" | "tanh" => format!("{}({})", n.op, child(0)),
        "curve" => format!("a response curve of {}", child(0)),
        "as_quantity" => child(0),
        "lt" => format!("{} < {}", child(0), child(1)),
        "le" => format!("{} ≤ {}", child(0), child(1)),
        "gt" => format!("{} > {}", child(0), child(1)),
        "ge" => format!("{} ≥ {}", child(0), child(1)),
        "and" => format!("{} and {}", child(0), child(1)),
        "or" => format!("{} or {}", child(0), child(1)),
        "not" => format!("not {}", child(0)),
        "select" => format!("if {} then {} otherwise {}", child(0), child(1), child(2)),
        "laplacian" => format!("how much {} differs from its neighbours", child(0)),
        "gradient_x" | "gradient_z" => format!("the slope of {}", child(0)),
        "neighbor_sum" => format!("{} summed over nearby spots", child(0)),
        "neighbor_mean" => format!("{} averaged over nearby spots", child(0)),
        "region_mean" | "region_sum" => format!("{} over region {}", child(0), arg("region")),
        "edge_from" => format!("{} on one side", child(0)),
        "edge_to" => format!("{} on the other side", child(0)),
        "sample" => format!("{} where it stands", child(0)),
        "sample_offset" => {
            let f = n.args.get("forward").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let l = n.args.get("lateral").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let dir = if f > 0.0 {
                "ahead"
            } else if f < 0.0 {
                "behind"
            } else if l > 0.0 {
                "to its left"
            } else {
                "to its right"
            };
            format!("{} {dir}", child(0))
        }
        op => format!("{op}(…)"),
    }
}

/// Things in the world a law reads (state, weather, traits), in plain words.
fn depends_on(rule: &Rule, all: &[Package], r: &str, out: &mut Vec<String>, depth: u32) {
    if r.is_empty() || depth > 40 {
        return;
    }
    let node_part = match r.rsplit_once('.') {
        Some((n, p)) if !p.contains('/') => n,
        _ => r,
    };
    let (rule, id) = match node_part.split_once('/') {
        Some((rid, nid)) => match all.iter().flat_map(|p| &p.rules).find(|x| x.rule_id == rid) {
            Some(o) => (o, nid),
            None => return,
        },
        None => (rule, node_part),
    };
    let Some(n) = rule.nodes.iter().find(|n| n.id == id) else { return };
    let arg = |k: &str| n.args.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let mut push = |x: String| {
        if !out.contains(&x) {
            out.push(x);
        }
    };
    match n.op.as_str() {
        "read_state" | "candidate_state" => push(humanize(&arg("field"))),
        "read_forcing" => push(humanize(&arg("forcing").replace("rain_flux", "rainfall"))),
        "trait" => push(format!("inherited {}", humanize(&arg("name")))),
        "builtin" => push(format!("its {}", arg("name"))),
        "crowding" => push("nearby animals".into()),
        "region" => push(format!("region {}", arg("region"))),
        "brain" => push("its brain's decisions".into()),
        "random" => push("chance".into()),
        _ => {}
    }
    if is_effect(&n.op) || n.op == "brain" {
        return;
    }
    for (_, child) in catalog::node_refs(n) {
        depends_on(rule, all, &child, out, depth + 1);
    }
}

fn trim_num(v: f64) -> String {
    let s = if v.abs() >= 1000.0 || v == v.trunc() {
        format!("{v}")
    } else {
        format!("{v:.4}")
    };
    
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

fn place(ep: &str, s: &Snapshot) -> String {
    if let Some(a) = s.plan.accounts.iter().find(|a| a.0 == ep) {
        return format!("outside the world ({})", a.2.to_lowercase());
    }
    humanize(ep.trim_end_matches("@from").trim_end_matches("@to"))
}

/// Plain-language view of the selected law: what it does, and the numbers to tune.
fn read_view(ui: &mut Ui, ed: &mut EditorState, s: &Snapshot) {
    let Some((pi, ri)) = ed.sel else {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            ui.heading("Pick a law on the left");
            ui.label(RichText::new("Each law describes one process of nature: rain falling, plants growing, animals eating. Read what it does, then change its numbers.").weak());
        });
        return;
    };
    let rule = ed.draft[pi].rules[ri].clone();
    let all = ed.draft.clone();
    ui.heading(short_label(&rule));
    ui.label(RichText::new(topic_of(&rule.rule_id)).weak());
    if let Some(r) = &ed.report {
        for d in r.diagnostics.iter().filter(|d| d.rule.as_deref() == Some(&rule.rule_id)) {
            ui.label(RichText::new(format!("⚠ {}", d.message)).color(Color32::from_rgb(200, 90, 60)));
        }
    }
    ui.add_space(8.0);
    ui.label(RichText::new("What it does").strong().size(16.0));
    let get = |n: &Node, k: &str| n.args.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let effects: Vec<Node> = rule.nodes.iter().filter(|n| is_effect(&n.op)).cloned().collect();
    if effects.is_empty() {
        ui.label(RichText::new("This law only works things out for other laws to use (it does not change the world by itself).").weak());
    }
    for n in &effects {
        egui::Frame::new()
            .fill(ui.visuals().faint_bg_color)
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let refs: Vec<String> = catalog::node_refs(n).into_iter().map(|x| x.1).collect();
                let headline = match n.op.as_str() {
                    "transfer" | "external_source" | "external_sink" => {
                        let from = if n.op == "external_source" {
                            get(n, "account")
                        } else {
                            get(n, "from")
                        };
                        let to = if n.op == "external_sink" { get(n, "account") } else { get(n, "to") };
                        format!("Moves {} from {} to {}", get(n, "resource"), place(&from, s), place(&to, s))
                    }
                    "edge_transfer" => format!("Moves {} between neighbouring spots", get(n, "resource")),
                    "reaction" => {
                        let legs = n.args.get("legs").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                        let parts: Vec<String> = legs
                            .iter()
                            .map(|l| {
                                let g = |k: &str| l.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
                                format!("{} from {} to {}", g("resource"), place(&g("from"), s), place(&g("to"), s))
                            })
                            .collect();
                        format!("Together, moves {}", parts.join("; "))
                    }
                    "rate_contribution" => format!("Changes {}", humanize(&get(n, "field"))),
                    "next_value" => format!("Sets {}", humanize(&get(n, "field"))),
                    "move" => "Moves the animal".into(),
                    "death" => format!("The animal dies ({})", get(n, "reason")),
                    "birth" => "A young animal is born".into(),
                    _ => n.op.clone(),
                };
                ui.label(RichText::new(headline).strong());
                let mut deps = vec![];
                for r in &refs {
                    depends_on(&rule, &all, r, &mut deps, 0);
                }
                if !deps.is_empty() {
                    ui.label(RichText::new(format!("Depends on: {}", deps.join(", "))).weak());
                }
                egui::CollapsingHeader::new(RichText::new("Formula").small())
                    .id_salt(("formula", &n.id))
                    .show(ui, |ui| {
                        for (port, r) in catalog::node_refs(n) {
                            let name = match port.as_str() {
                                "rate" => "per second".to_string(),
                                "condition" => "when".to_string(),
                                p if p.starts_with("legs[") => "per second".to_string(),
                                p => p.to_string(),
                            };
                            ui.label(RichText::new(format!("{name}: {}", formula(&rule, &all, &r, 0))).small());
                        }
                    });
            });
    }
    ui.add_space(8.0);
    ui.label(RichText::new("Numbers you can change").strong().size(16.0));
    if rule.parameters.is_empty() {
        ui.label(RichText::new("This law has no adjustable numbers. Use the graph view to change its formula.").weak());
    }
    let mut changed = None;
    for (name, pd) in &rule.parameters {
        let mut v = pd.value;
        ui.label(RichText::new(if pd.label.is_empty() { humanize(name) } else { pd.label.clone() }).weak());
        let log = pd.min >= 0.0 && pd.max / pd.min.max(1e-12) > 100.0;
        if ui
            .add(
                egui::Slider::new(&mut v, pd.min..=pd.max)
                    .logarithmic(log)
                    .suffix(format!(" {}", pd.unit)),
            )
            .changed()
        {
            changed = Some((name.clone(), v));
        }
    }
    if let Some((name, v)) = changed {
        ed.edit();
        if let Some(pd) = ed.draft[pi].rules[ri].parameters.get_mut(&name) {
            pd.value = v;
        }
    }
    ui.add_space(12.0);
    if ui.button("🔧 Change the formula in the graph view").clicked() {
        ed.graph_mode = true;
        ed.expanded = true;
    }
}

fn rules_panel(ui: &mut Ui, ed: &mut EditorState, s: &Snapshot) {
    ui.strong("Rules");
    let mut select = None;
    let mut toggle = None;
    for (pi, p) in ed.draft.iter().enumerate() {
        egui::CollapsingHeader::new(RichText::new(&p.package_id).strong())
            .id_salt(("pkg", pi))
            .default_open(true)
            .show(ui, |ui| {
                for (ri, r) in p.rules.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let mut en = r.enabled;
                        if ui.checkbox(&mut en, "").on_hover_text("enabled").changed() {
                            toggle = Some((pi, ri, en));
                        }
                        let label = if r.label.is_empty() { r.rule_id.clone() } else { r.label.clone() };
                        let tag = r.template.as_ref().map(|t| format!(" [{t}]")).unwrap_or_default();
                        let errors = ed
                            .report
                            .as_ref()
                            .map(|rep| rep.diagnostics.iter().filter(|d| d.rule.as_deref() == Some(&r.rule_id)).count())
                            .unwrap_or(0);
                        let mut text = RichText::new(truncate(&format!("{label}{tag}"), 34));
                        if errors > 0 {
                            text = text.color(Color32::from_rgb(255, 120, 100));
                        }
                        if ui
                            .selectable_label(ed.sel == Some((pi, ri)), text)
                            .on_hover_text(&r.rule_id)
                            .clicked()
                        {
                            select = Some((pi, ri));
                        }
                    });
                }
            });
    }
    if let Some((pi, ri, en)) = toggle {
        ed.edit();
        ed.draft[pi].rules[ri].enabled = en;
    }
    if let Some(x) = select {
        ed.sel = Some(x);
        ed.sel_node = None;
        ed.expanded = ed.draft[x.0].rules[x.1].template.is_none();
    }
    ui.separator();
    ui.strong("New law");
    ui.horizontal(|ui| {
        ui.label("id");
        ui.text_edit_singleline(&mut ed.new_rule_id);
    });
    egui::ComboBox::from_id_salt("newkind")
        .selected_text(ed.new_rule_kind.clone())
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut ed.new_rule_kind, "cells".into(), "cells");
            ui.selectable_value(&mut ed.new_rule_kind, "edges".into(), "edges");
            for (a, _) in &s.plan.archetypes {
                ui.selectable_value(&mut ed.new_rule_kind, format!("entities:{a}"), format!("organisms ({a})"));
            }
        });
    let templates: Vec<(String, String)> = ed
        .draft
        .iter()
        .flat_map(|p| {
            p.rules
                .iter()
                .filter(|r| r.template.is_some())
                .map(|r| (r.rule_id.clone(), format!("{} ({})", r.template.clone().unwrap(), r.rule_id)))
        })
        .collect();
    egui::ComboBox::from_id_salt("tmpl")
        .selected_text(if ed.new_rule_template.is_empty() {
            "empty rule".to_string()
        } else {
            ed.new_rule_template.clone()
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut ed.new_rule_template, String::new(), "empty rule");
            for (id, l) in &templates {
                ui.selectable_value(&mut ed.new_rule_template, id.clone(), l);
            }
        });
    if ui.button("Create in player package").clicked() {
        let id = ed.new_rule_id.clone();
        let exists = ed.draft.iter().any(|p| p.rules.iter().any(|r| r.rule_id == id));
        if exists || !compiler::valid_id(&id) || id.contains('/') {
            ed.message = format!("rule id {id} is invalid or already used");
        } else {
            ed.edit();
            let rule = if ed.new_rule_template.is_empty() {
                let (kind, arch) = match ed.new_rule_kind.split_once(':') {
                    Some((k, a)) => (k.to_string(), Some(a.to_string())),
                    None => (ed.new_rule_kind.clone(), None),
                };
                Rule {
                    rule_id: id.clone(),
                    label: "New law".into(),
                    domain: DomainDecl {
                        kind,
                        grid: None,
                        archetype: arch,
                    },
                    scope: None,
                    parameters: BTreeMap::new(),
                    nodes: vec![],
                    effects: vec![],
                    template: None,
                    enabled: true,
                }
            } else {
                let src = ed
                    .draft
                    .iter()
                    .flat_map(|p| p.rules.iter())
                    .find(|r| r.rule_id == ed.new_rule_template)
                    .cloned()
                    .unwrap();
                let mut r = src.clone();
                // Cross-rule references stay qualified; local references stay local.
                r.rule_id = id.clone();
                r.label = format!("{} (copy)", src.label);
                // Parameters referenced by name resolve to this rule's own copies; package parameters stay shared.
                r
            };
            let pi = ed.player_package();
            ed.draft[pi].rules.push(rule);
            ed.sel = Some((pi, ed.draft[pi].rules.len() - 1));
            ed.expanded = true;
            ed.message = format!("created {id}; an exact copy of a law adds to the original (e.g. two rainfall rules add rain)");
        }
    }
    if let Some((pi, ri)) = ed.sel {
        ui.horizontal(|ui| {
            if ui.button("Duplicate").clicked() {
                ed.edit();
                let mut r = ed.draft[pi].rules[ri].clone();
                let mut n = 2;
                while ed
                    .draft
                    .iter()
                    .any(|p| p.rules.iter().any(|x| x.rule_id == format!("{}_{n}", r.rule_id)))
                {
                    n += 1;
                }
                r.rule_id = format!("{}_{n}", r.rule_id);
                let pp = ed.player_package();
                ed.draft[pp].rules.push(r);
                ed.sel = Some((pp, ed.draft[pp].rules.len() - 1));
            }
            if ui.button("Delete").clicked() {
                ed.edit();
                ed.draft[pi].rules.remove(ri);
                ed.sel = None;
            }
        });
    }
    ui.separator();
    ui.strong("Custom field (player package)");
    ui.horizontal(|ui| {
        ui.text_edit_singleline(&mut ed.new_field);
        ui.add(egui::TextEdit::singleline(&mut ed.new_field_unit).desired_width(50.0));
    });
    if ui
        .button("Add integrated cell field")
        .on_hover_text("A bounded, non-conserved scalar state (e.g. a toxin level) with its own unit")
        .clicked()
    {
        let (id, unit) = (ed.new_field.clone(), ed.new_field_unit.clone());
        if sim_core::units::Unit::parse(&unit).is_err() {
            ed.message = format!("invalid unit {unit}");
        } else {
            ed.edit();
            let pi = ed.player_package();
            ed.draft[pi].fields.push(
                serde_json::from_value(json!({ "id": id, "unit": unit, "policy": { "kind": "integrated" }, "label": "player field" }))
                    .unwrap(),
            );
        }
    }
    if let Some(pi) = ed.draft.iter().position(|p| p.package_id == PLAYER_PACKAGE) {
        for f in ed.draft[pi].fields.clone() {
            ui.label(RichText::new(format!("field {} [{}] {:?}", f.id, f.unit, f.policy)).small());
        }
    }
    ui.separator();
    ui.strong("Palette");
    ui.label(RichText::new("Click to add a node to the selected rule.").small());
    let mut add = None;
    for cat in catalog::categories() {
        egui::CollapsingHeader::new(cat)
            .id_salt(("cat", cat))
            .default_open(false)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for o in catalog::OPS.iter().filter(|o| o.category == cat) {
                        if ui.small_button(o.op).on_hover_text(o.description).clicked() {
                            add = Some(o.op);
                        }
                    }
                });
            });
    }
    if let (Some(op), Some(_)) = (add, ed.sel) {
        ed.edit();
        let args = default_args(op, s);
        let center = ed.scene_rect.center();
        let rule = ed.rule_mut().unwrap();
        let mut k = rule.nodes.len();
        while rule.nodes.iter().any(|n| n.id == format!("{op}_{k}")) {
            k += 1;
        }
        let id = format!("{op}_{k}");
        let mut args = args;
        args.insert("pos".into(), json!([center.x - NODE_W / 2.0, center.y - 30.0]));
        rule.nodes.push(Node {
            id: id.clone(),
            op: op.to_string(),
            args,
        });
        sync_effects(rule);
        ed.sel_node = Some(id);
        ed.expanded = true;
    }
}

fn canvas(ui: &mut Ui, ed: &mut EditorState, _s: &Snapshot) {
    let Some(rule) = ed.rule().cloned() else {
        ui.centered_and_justified(|ui| {
            ui.label("Select a rule to see its graph. Templates open collapsed; expand to edit their actual nodes.")
        });
        return;
    };
    if !ed.expanded {
        ui.vertical_centered(|ui| {
            ui.add_space(30.0);
            ui.heading(format!("{} [template: {}]", rule.label, rule.template.clone().unwrap_or_default()));
            ui.label(format!("{} nodes, effects: {}", rule.nodes.len(), rule.effects.join(", ")));
            ui.label(
                "Templates are ordinary graphs, not privileged systems. Edit parameters on the right, or expand to change the formula.",
            );
            if ui.button("Expand graph").clicked() {
                ed.expanded = true;
            }
        });
        return;
    }
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!(
                "{}  ·  domain: {}{}",
                rule.rule_id,
                rule.domain.kind,
                rule.domain.archetype.as_ref().map(|a| format!(" ({a})")).unwrap_or_default()
            ))
            .strong(),
        );
        if rule.template.is_some() && ui.small_button("collapse").clicked() {
            ed.expanded = false;
        }
        ui.label(
            RichText::new("drag from an output ● to an input ● to connect · drag a header to move · right-drag to pan · wheel to zoom")
                .small()
                .weak(),
        );
    });
    let positions = layout(&rule);
    if ed.fit_for != ed.sel && !positions.is_empty() {
        // Frame the whole graph when a rule is first shown.
        let mut bb = Rect::NOTHING;
        for (id, p) in &positions {
            let h = rule.nodes.iter().find(|n| &n.id == id).map(node_height).unwrap_or(60.0);
            bb = bb.union(Rect::from_min_size(*p, vec2(NODE_W, h)));
        }
        ed.scene_rect = bb.expand(40.0);
        ed.fit_for = ed.sel;
    }
    let rep = ed.report.as_ref();
    let mut scene_rect = ed.scene_rect;
    let mut new_pos: Option<(String, Pos2)> = None;
    let mut clicked_node: Option<String> = None;
    let mut connect: Option<(String, usize, String)> = None;
    let mut delete: Option<String> = None;
    let mut dup: Option<String> = None;
    let mut started_move = None;
    let mut inputs_pos = vec![];
    let mut outputs_pos = HashMap::new();
    let drag_from = ed.drag_from.clone();
    let mut new_drag_from = drag_from.clone();
    let sel_node = ed.sel_node.clone();
    // Manual pan/zoom over a painter clipped to the canvas, so nothing spills into side panels.
    let canvas_rect = ui.available_rect_before_wrap();
    let bg = ui.interact(canvas_rect, ui.id().with("canvas_bg"), Sense::click_and_drag());
    {
        // Keep the visible world rect at the canvas aspect ratio.
        let aspect = canvas_rect.width() / canvas_rect.height().max(1.0);
        let c = scene_rect.center();
        let w = scene_rect.width().max(scene_rect.height() * aspect);
        scene_rect = Rect::from_center_size(c, vec2(w, w / aspect));
    }
    let mut zoom = canvas_rect.width() / scene_rect.width().max(1.0);
    if bg.dragged_by(egui::PointerButton::Secondary) || bg.dragged_by(egui::PointerButton::Middle) {
        scene_rect = scene_rect.translate(-bg.drag_delta() / zoom);
    }
    if let Some(hp) = bg.hover_pos() {
        let scroll = ui.ctx().input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            let f = (1.0 - scroll * 0.002).clamp(0.5, 2.0);
            let new_w = (scene_rect.width() * f).clamp(200.0, 20000.0);
            let k = new_w / scene_rect.width();
            let anchor = scene_rect.min + (hp - canvas_rect.min) / zoom;
            scene_rect = Rect::from_min_size(anchor + (scene_rect.min - anchor) * k, scene_rect.size() * k);
            zoom = canvas_rect.width() / scene_rect.width();
        }
    }
    let origin = scene_rect.min;
    let cmin = canvas_rect.min;
    let tf = move |p: Pos2| -> Pos2 { cmin + (p - origin) * zoom };
    let z = zoom;
    {
        let painter = ui.painter_at(canvas_rect);
        let text = ui.visuals().text_color();
        let weak = ui.visuals().weak_text_color();
        let small = egui::FontId::proportional((11.0 * z).max(5.0));
        // Nodes.
        for n in &rule.nodes {
            let wp = positions[&n.id];
            let p = tf(wp);
            let h = node_height(n) * z;
            let rect = Rect::from_min_size(p, vec2(NODE_W * z, h));
            let diags = node_diags(rep, &rule.rule_id, &n.id);
            let fill = ui.visuals().extreme_bg_color;
            let border = if !diags.is_empty() {
                Stroke::new(2.0, Color32::from_rgb(255, 90, 80))
            } else if sel_node.as_deref() == Some(&n.id) {
                Stroke::new(2.0, Color32::from_rgb(255, 210, 80))
            } else {
                Stroke::new(1.0, Color32::from_gray(90))
            };
            painter.rect(rect, 5.0 * z, fill, border, egui::StrokeKind::Inside);
            let header = Rect::from_min_size(p, vec2(NODE_W * z, 20.0 * z));
            painter.rect_filled(
                header,
                egui::CornerRadius {
                    nw: 5,
                    ne: 5,
                    sw: 0,
                    se: 0,
                },
                category_color(&n.op),
            );
            painter.text(
                header.left_center() + vec2(6.0 * z, 0.0),
                egui::Align2::LEFT_CENTER,
                format!("{}  {}", n.op, n.id),
                egui::FontId::proportional((12.0 * z).max(5.0)),
                Color32::WHITE,
            );
            let hresp = ui.interact(header, ui.id().with(("hdr", &n.id)), Sense::click_and_drag());
            if hresp.drag_started() {
                started_move = Some(n.id.clone());
            }
            if hresp.dragged() {
                new_pos = Some((n.id.clone(), wp + hresp.drag_delta() / z));
            }
            if hresp.clicked() {
                clicked_node = Some(n.id.clone());
            }
            let body = ui.interact(rect, ui.id().with(("body", &n.id)), Sense::click());
            if body.clicked() {
                clicked_node = Some(n.id.clone());
            }
            let nid = n.id.clone();
            body.context_menu(|ui| {
                if ui.button("Duplicate").clicked() {
                    dup = Some(nid.clone());
                    ui.close();
                }
                if ui.button("Delete").clicked() {
                    delete = Some(nid.clone());
                    ui.close();
                }
            });
            if !diags.is_empty() {
                body.on_hover_text(diags.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("\n"));
            }
            let row = ROW * z;
            let mut y = p.y + 22.0 * z;
            // Input ports.
            let port_names: Vec<String> = match catalog::info(&n.op).map(|i| i.inputs) {
                Some(Inputs::Positional(names)) => names.iter().map(|s| s.to_string()).collect(),
                Some(Inputs::Named(names)) => names.iter().map(|s| s.to_string()).collect(),
                _ => vec![],
            };
            let refs = catalog::node_refs(n);
            let mut rows: Vec<(String, String)> = port_names
                .iter()
                .map(|pn| {
                    (
                        pn.clone(),
                        refs.iter().find(|r| &r.0 == pn).map(|r| r.1.clone()).unwrap_or_default(),
                    )
                })
                .collect();
            for r in &refs {
                if !rows.iter().any(|x| x.0 == r.0) {
                    rows.push(r.clone());
                }
            }
            for (k, (pname, target)) in rows.iter().enumerate() {
                let pp = pos2(p.x, y + row / 2.0);
                let port_diag = diags
                    .iter()
                    .any(|d| d.port.as_deref().is_some_and(|x| x == pname || x == format!("inputs[{k}]")));
                painter.circle_filled(
                    pp,
                    4.5 * z.max(0.6),
                    if port_diag {
                        Color32::from_rgb(255, 90, 80)
                    } else {
                        Color32::from_gray(170)
                    },
                );
                let shown = if target.is_empty() { "—".to_string() } else { target.clone() };
                painter.text(
                    pp + vec2(8.0 * z, 0.0),
                    egui::Align2::LEFT_CENTER,
                    format!("{pname}: {shown}"),
                    small.clone(),
                    if target.is_empty() { Color32::from_rgb(230, 150, 90) } else { text },
                );
                inputs_pos.push((n.id.clone(), k, pp));
                y += row;
            }
            // Scalar args (compact).
            let mut shown_args = 0;
            for (k, v) in &n.args {
                if matches!(
                    k.as_str(),
                    "inputs" | "pos" | "legs" | "rate" | "amount" | "turn" | "speed" | "condition"
                ) || (k == "value" && n.op != "const")
                {
                    continue;
                }
                if shown_args >= 4 {
                    break;
                }
                let vs = match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                painter.text(
                    pos2(p.x + 8.0 * z, y + row / 2.0),
                    egui::Align2::LEFT_CENTER,
                    format!("{k} = {}", truncate(&vs, 26)),
                    small.clone(),
                    weak,
                );
                y += row;
                shown_args += 1;
            }
            if let Some(Value::Array(legs)) = n.args.get("legs") {
                painter.text(
                    pos2(p.x + 8.0 * z, y - row / 2.0 + 2.0 * z),
                    egui::Align2::LEFT_CENTER,
                    format!("{} leg(s)", legs.len()),
                    small.clone(),
                    weak,
                );
            }
            // Output ports with type labels.
            let outs: Vec<&str> = catalog::info(&n.op).map(|i| i.outputs.to_vec()).unwrap_or(vec!["value"]);
            let types = rep.and_then(|r| r.types.get(&format!("{}/{}", rule.rule_id, n.id)));
            for (k, o) in outs.iter().enumerate() {
                let pp = pos2(p.x + NODE_W * z, y + row / 2.0);
                let refname = if *o == "value" { n.id.clone() } else { format!("{}.{o}", n.id) };
                painter.circle_filled(pp, 4.5 * z.max(0.6), Color32::from_rgb(120, 200, 255));
                let label = if is_effect(&n.op) {
                    o.to_string()
                } else {
                    types
                        .and_then(|t| t.get(k))
                        .map(|t| t.1.clone())
                        .unwrap_or_else(|| "? (unresolved)".into())
                };
                painter.text(
                    pp - vec2(8.0 * z, 0.0),
                    egui::Align2::RIGHT_CENTER,
                    truncate(&label, 34),
                    small.clone(),
                    Color32::from_rgb(150, 210, 255),
                );
                let presp = ui.interact(
                    Rect::from_center_size(pp, vec2(12.0, 12.0)),
                    ui.id().with(("out", &n.id, k)),
                    Sense::drag(),
                );
                if presp.drag_started() {
                    new_drag_from = Some(refname.clone());
                }
                outputs_pos.insert(refname, pp);
                y += row;
            }
        }
        // Wires to local sources.
        let out_of = |r: &str| -> Option<Pos2> {
            if r.contains('/') {
                return None;
            }
            outputs_pos
                .get(r)
                .copied()
                .or_else(|| outputs_pos.get(r.split('.').next().unwrap_or("")).copied())
                .or_else(|| {
                    // Receipt ports of effects: anchor at the effect's first output.
                    let base = r.split('.').next()?;
                    outputs_pos.iter().find(|(k, _)| k.split('.').next() == Some(base)).map(|x| *x.1)
                })
        };
        for n in &rule.nodes {
            let port_names: Vec<String> = match catalog::info(&n.op).map(|i| i.inputs) {
                Some(Inputs::Positional(names)) => names.iter().map(|s| s.to_string()).collect(),
                Some(Inputs::Named(names)) => names.iter().map(|s| s.to_string()).collect(),
                _ => vec![],
            };
            let refs = catalog::node_refs(n);
            let mut rows: Vec<(String, String)> = port_names
                .iter()
                .map(|pn| {
                    (
                        pn.clone(),
                        refs.iter().find(|r| &r.0 == pn).map(|r| r.1.clone()).unwrap_or_default(),
                    )
                })
                .collect();
            for r in &refs {
                if !rows.iter().any(|x| x.0 == r.0) {
                    rows.push(r.clone());
                }
            }
            for (k, (_, target)) in rows.iter().enumerate() {
                if let (Some(a), Some((_, _, b))) = (out_of(target), inputs_pos.iter().find(|x| x.0 == n.id && x.1 == k)) {
                    wire(&painter, a, *b, Color32::from_rgb(120, 170, 220));
                }
            }
        }
        if let Some(src) = &drag_from
            && let (Some(a), Some(ptr)) = (outputs_pos.get(src), ui.ctx().pointer_latest_pos())
        {
            wire(&painter, *a, ptr, Color32::YELLOW);
            if ui.ctx().input(|i| i.pointer.any_released()) {
                if let Some((nid, k, _)) = inputs_pos.iter().find(|x| x.2.distance(ptr) < 10.0) {
                    connect = Some((nid.clone(), *k, src.clone()));
                }
                new_drag_from = None;
            }
        }
    }
    ed.scene_rect = scene_rect;
    ed.inputs_pos = inputs_pos;
    ed.outputs_pos = outputs_pos;
    ed.drag_from = new_drag_from;
    if let Some(id) = started_move {
        ed.edit();
        ed.moving = Some(id);
    }
    if let Some((id, p)) = new_pos
        && let Some(rule) = ed.rule_mut()
        && let Some(n) = rule.nodes.iter_mut().find(|n| n.id == id)
    {
        n.args.insert("pos".into(), json!([p.x.round(), p.y.round()]));
    }
    if let Some(id) = clicked_node {
        ed.sel_node = Some(id);
    }
    if let Some((nid, k, src)) = connect {
        ed.edit();
        if let Some(rule) = ed.rule_mut()
            && let Some(n) = rule.nodes.iter_mut().find(|n| n.id == nid)
        {
            set_input(n, k, &src);
        }
        ed.sel_node = Some(nid);
    }
    if let Some(id) = delete {
        ed.edit();
        if let Some(rule) = ed.rule_mut() {
            rule.nodes.retain(|n| n.id != id);
            sync_effects(rule);
        }
        ed.sel_node = None;
    }
    if let Some(id) = dup {
        ed.edit();
        if let Some(rule) = ed.rule_mut()
            && let Some(n) = rule.nodes.iter().find(|n| n.id == id).cloned()
        {
            let mut m = n.clone();
            let mut k = 2;
            while rule.nodes.iter().any(|x| x.id == format!("{}_{k}", n.id)) {
                k += 1;
            }
            m.id = format!("{}_{k}", n.id);
            if let Some(p) = node_pos(&n) {
                m.args.insert("pos".into(), json!([p.x + 30.0, p.y + 30.0]));
            }
            rule.nodes.push(m);
            sync_effects(rule);
        }
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

fn wire(p: &egui::Painter, a: Pos2, b: Pos2, c: Color32) {
    let dx = ((b.x - a.x).abs() * 0.5).max(30.0);
    let pts = [a, a + vec2(dx, 0.0), b - vec2(dx, 0.0), b];
    p.add(egui::epaint::CubicBezierShape::from_points_stroke(
        pts,
        false,
        Color32::TRANSPARENT,
        Stroke::new(1.5, c),
    ));
}

fn combo_str(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, value: &mut String, options: &[String]) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(truncate(value, 26))
        .width(ui.available_width().clamp(80.0, 200.0))
        .show_ui(ui, |ui| {
            for o in options {
                if ui.selectable_label(value == o, o).clicked() {
                    *value = o.clone();
                    changed = true;
                }
            }
        });
    changed
}

fn properties(ui: &mut Ui, ed: &mut EditorState, s: &Snapshot) {
    // Diagnostics and validation summary first: they explain why a draft cannot apply.
    if let Some(r) = &ed.report {
        egui::CollapsingHeader::new(RichText::new(format!("Validation ({} errors)", r.diagnostics.len())).strong())
            .default_open(true)
            .show(ui, |ui| {
                let mut jump = None;
                for d in &r.diagnostics {
                    let resp = ui.label(RichText::new(d.to_string()).small().color(if d.severity == Severity::Error {
                        Color32::from_rgb(255, 130, 110)
                    } else {
                        Color32::YELLOW
                    }));
                    if resp.clicked() || resp.on_hover_text("click to select").clicked() {
                        jump = Some(d.clone());
                    }
                }
                if let Some(d) = jump
                    && let Some(rule) = &d.rule
                {
                    for (pi, p) in ed.draft.iter().enumerate() {
                        if let Some(ri) = p.rules.iter().position(|x| &x.rule_id == rule) {
                            ed.sel = Some((pi, ri));
                            ed.sel_node = d.node.clone();
                            ed.expanded = true;
                        }
                    }
                }
                ui.label(RichText::new("Summary").strong());
                for l in &r.summary {
                    ui.label(RichText::new(l).small());
                }
                for w in &r.warnings {
                    ui.label(RichText::new(w).small().color(Color32::YELLOW));
                }
            });
    }
    let Some((pi, ri)) = ed.sel else {
        ui.label("No rule selected.");
        return;
    };
    ui.separator();
    // Rule-level properties.
    let rule = ed.draft[pi].rules[ri].clone();
    ui.strong(format!("Rule {}", rule.rule_id));
    let mut label = rule.label.clone();
    if ui.text_edit_singleline(&mut label).changed() {
        ed.edit();
        ed.draft[pi].rules[ri].label = label;
    }
    let mut region = rule.scope.as_ref().and_then(|x| x.region.clone()).unwrap_or_default();
    let mut regions = vec![String::new()];
    regions.extend(s.plan.regions.iter().cloned());
    ui.horizontal(|ui| {
        ui.label("scope region");
        if combo_str(ui, "scope_region", &mut region, &regions) {
            ed.edit();
            let pred = ed.draft[pi].rules[ri].scope.as_ref().and_then(|x| x.predicate.clone());
            ed.draft[pi].rules[ri].scope = if region.is_empty() && pred.is_none() {
                None
            } else {
                Some(Scope {
                    region: (!region.is_empty()).then_some(region.clone()),
                    predicate: pred,
                })
            };
        }
    });
    ui.label(
        RichText::new("A scope scales transfers and rates by region coverage; conservation still applies across its edge.")
            .small()
            .weak(),
    );
    ui.collapsing(format!("Parameters ({})", rule.parameters.len()), |ui| {
        let mut changed: Option<(String, ParamDecl)> = None;
        let mut remove = None;
        for (name, p) in &rule.parameters {
            ui.horizontal(|ui| {
                let mut v = p.value;
                ui.label(name).on_hover_text(&p.label);
                if ui
                    .add(
                        egui::DragValue::new(&mut v)
                            .speed((p.max - p.min).abs() * 0.002)
                            .range(p.min..=p.max),
                    )
                    .changed()
                {
                    changed = Some((name.clone(), ParamDecl { value: v, ..p.clone() }));
                }
                ui.label(RichText::new(format!("{} [{}, {}]", p.unit, p.min, p.max)).small());
                if ed.draft[pi].package_id == PLAYER_PACKAGE && ui.small_button("✕").clicked() {
                    remove = Some(name.clone());
                }
            });
        }
        if let Some((n, p)) = changed {
            ed.edit();
            ed.draft[pi].rules[ri].parameters.insert(n, p);
        }
        if let Some(n) = remove {
            ed.edit();
            ed.draft[pi].rules[ri].parameters.remove(&n);
        }
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut ed.new_param);
            if ui.button("add parameter").clicked() && compiler::valid_id(&ed.new_param) {
                let n = ed.new_param.clone();
                ed.edit();
                ed.draft[pi].rules[ri].parameters.insert(
                    n,
                    ParamDecl {
                        value: 1.0,
                        unit: "1".into(),
                        quantity: None,
                        min: 0.0,
                        max: 10.0,
                        label: "player parameter".into(),
                        stability: None,
                    },
                );
            }
        });
        ui.label(
            RichText::new(
                "Edit a parameter's unit/range by saving the package JSON; values are validated against ranges and stability limits.",
            )
            .small()
            .weak(),
        );
    });
    ui.separator();
    let Some(nid) = ed.sel_node.clone() else {
        ui.label("Select a node to edit it.");
        return;
    };
    let Some(ni) = rule.nodes.iter().position(|n| n.id == nid) else {
        ed.sel_node = None;
        return;
    };
    let node = rule.nodes[ni].clone();
    let info = catalog::info(&node.op);
    ui.strong(format!("Node {} ({})", node.id, node.op));
    if let Some(i) = info {
        ui.label(RichText::new(i.description).small());
    }
    for d in node_diags(ed.report.as_ref(), &rule.rule_id, &node.id) {
        ui.label(RichText::new(d.to_string()).small().color(Color32::from_rgb(255, 130, 110)));
    }
    if let Some(t) = ed
        .report
        .as_ref()
        .and_then(|r| r.types.get(&format!("{}/{}", rule.rule_id, node.id)))
    {
        for (p, d) in t {
            ui.label(RichText::new(format!("{p}: {d}")).small().color(Color32::from_rgb(150, 210, 255)));
        }
    }
    // Rename (updates local references).
    let mut new_id = node.id.clone();
    ui.horizontal(|ui| {
        ui.label("id");
        if ui.text_edit_singleline(&mut new_id).lost_focus()
            && new_id != node.id
            && compiler::valid_id(&new_id)
            && !new_id.contains('.')
            && !new_id.contains('/')
        {
            ed.edit();
            let r = &mut ed.draft[pi].rules[ri];
            if !r.nodes.iter().any(|n| n.id == new_id) {
                for n in &mut r.nodes {
                    rename_refs(&mut n.args, &node.id, &new_id);
                }
                r.nodes[ni].id = new_id.clone();
                sync_effects(r);
                ed.sel_node = Some(new_id.clone());
            }
        }
    });
    // Candidate reference targets.
    let mut targets: Vec<String> = vec![String::new()];
    for n in &rule.nodes {
        if n.id == node.id {
            continue;
        }
        if is_effect(&n.op) {
            for o in catalog::info(&n.op).map(|i| i.outputs).unwrap_or(&[]) {
                targets.push(format!("{}.{o}", n.id));
            }
        } else if n.op == "brain" {
            for k in 0..6 {
                targets.push(format!("{}.out{k}", n.id));
            }
        } else {
            targets.push(n.id.clone());
        }
    }
    let mut args = node.args.clone();
    let mut changed = false;
    // Input references.
    match info.map(|i| i.inputs) {
        Some(Inputs::Positional(names)) => {
            let mut arr = args.get("inputs").and_then(|v| v.as_array().cloned()).unwrap_or_default();
            arr.resize(names.len(), json!(""));
            for (k, pn) in names.iter().enumerate() {
                let mut v = arr[k].as_str().unwrap_or("").to_string();
                ui.horizontal(|ui| {
                    ui.label(*pn);
                    if combo_str(ui, ("in", k), &mut v, &targets) {
                        changed = true;
                    }
                    if ui
                        .add(egui::TextEdit::singleline(&mut v).desired_width(120.0))
                        .on_hover_text("or type rule_id/node.port for another rule")
                        .changed()
                    {
                        changed = true;
                    }
                });
                arr[k] = json!(v);
            }
            if !names.is_empty() {
                args.insert("inputs".into(), Value::Array(arr));
            }
        }
        Some(Inputs::Named(names)) => {
            for pn in names {
                let mut v = args.get(*pn).and_then(|v| v.as_str()).unwrap_or("").to_string();
                ui.horizontal(|ui| {
                    ui.label(*pn);
                    if combo_str(ui, ("named", pn), &mut v, &targets) {
                        changed = true;
                    }
                    if ui.add(egui::TextEdit::singleline(&mut v).desired_width(120.0)).changed() {
                        changed = true;
                    }
                });
                args.insert(pn.to_string(), json!(v));
            }
        }
        Some(Inputs::Variadic) => {
            let arr = args.get("inputs").and_then(|v| v.as_array().cloned()).unwrap_or_default();
            ui.label(RichText::new(format!("{} observation inputs (edit in JSON or by dragging wires)", arr.len())).small());
        }
        None => {}
    }
    // Scalar and structured args.
    let fields = |entity: bool| -> Vec<String> {
        let mut v: Vec<String> = s.plan.cell_fields.iter().map(|f| f.0.clone()).collect();
        if entity && let Some(a) = rule.domain.archetype.as_ref() {
            for p in &s.plan.packages {
                for ar in p.archetypes.iter().filter(|x| &x.id == a) {
                    v.extend(ar.fields.iter().map(|f| format!("self.{}", f.id)));
                }
            }
        }
        v
    };
    let entity = rule.domain.kind == "entities";
    let mut params: Vec<String> = rule.parameters.keys().cloned().collect();
    if let Some(p) = ed.draft.get(pi) {
        params.extend(p.parameters.keys().cloned());
    }
    let endpoints = || -> Vec<String> {
        let mut v = vec![];
        for f in &s.plan.reservoir_fields {
            match rule.domain.kind.as_str() {
                "edges" => {
                    v.push(format!("{f}@from"));
                    v.push(format!("{f}@to"));
                }
                "entities" => v.push(format!("cell.{f}")),
                _ => v.push(f.clone()),
            }
        }
        if entity {
            for f in fields(true).into_iter().filter(|f| f.starts_with("self.")) {
                v.push(f);
            }
        }
        v.extend(s.plan.accounts.iter().map(|a| a.0.clone()));
        v
    };
    if let Some(i) = info {
        for arg in i.args {
            let key = arg.name.to_string();
            match arg.kind {
                ArgKind::Number | ArgKind::Integer => {
                    let mut v = args.get(&key).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    ui.horizontal(|ui| {
                        ui.label(&key);
                        let speed = 0.01 * v.abs().max(0.01);
                        if ui.add(egui::DragValue::new(&mut v).speed(speed)).changed() {
                            changed = true;
                        }
                    });
                    args.insert(
                        key,
                        if arg.kind == ArgKind::Integer {
                            json!(v.round() as i64)
                        } else {
                            json!(v)
                        },
                    );
                }
                ArgKind::Unit | ArgKind::Quantity | ArgKind::Text => {
                    let mut v = args.get(&key).and_then(|v| v.as_str()).unwrap_or("").to_string();
                    ui.horizontal(|ui| {
                        ui.label(&key);
                        let ok = arg.kind != ArgKind::Unit || sim_core::units::Unit::parse(&v).is_ok();
                        let resp = ui.add(egui::TextEdit::singleline(&mut v).desired_width(150.0).text_color(if ok {
                            ui.visuals().text_color()
                        } else {
                            Color32::from_rgb(255, 120, 100)
                        }));
                        if arg.kind == ArgKind::Unit {
                            resp.on_hover_text("SI units: kg, m, s, K (absolute), dK (interval), J, W, 1 — e.g. kg/(m^2*s)");
                        }
                        if v != args.get(&key).and_then(|x| x.as_str()).unwrap_or("") {
                            changed = true;
                        }
                    });
                    if v.is_empty() && !arg.required {
                        args.remove(&key);
                    } else {
                        args.insert(key, json!(v));
                    }
                }
                ArgKind::Field
                | ArgKind::CellFieldOnly
                | ArgKind::Forcing
                | ArgKind::Region
                | ArgKind::Param
                | ArgKind::Resource
                | ArgKind::Endpoint
                | ArgKind::Account
                | ArgKind::Builtin => {
                    let opts: Vec<String> = match arg.kind {
                        ArgKind::Field => fields(entity),
                        ArgKind::CellFieldOnly => s.plan.reservoir_fields.clone(),
                        ArgKind::Forcing => s.plan.scenario.weather.signals.keys().cloned().collect(),
                        ArgKind::Region => s.plan.regions.clone(),
                        ArgKind::Param => params.clone(),
                        ArgKind::Resource => s.plan.resources.clone(),
                        ArgKind::Account => s.plan.accounts.iter().map(|a| a.0.clone()).collect(),
                        ArgKind::Builtin => vec!["age".into(), "heading".into(), "cooldown".into(), "generation".into()],
                        _ => endpoints(),
                    };
                    let mut v = args.get(&key).and_then(|v| v.as_str()).unwrap_or("").to_string();
                    ui.horizontal(|ui| {
                        ui.label(&key);
                        if combo_str(ui, ("arg", &key), &mut v, &opts) {
                            changed = true;
                        }
                    });
                    args.insert(key, json!(v));
                }
                ArgKind::Points => {
                    let pts = args.get(&key).and_then(|v| v.as_array().cloned()).unwrap_or_default();
                    let mut text = pts
                        .iter()
                        .filter_map(|p| Some(format!("{}:{}", p.get(0)?.as_f64()?, p.get(1)?.as_f64()?)))
                        .collect::<Vec<_>>()
                        .join(", ");
                    ui.label("points (x:y, …)");
                    if ui.text_edit_singleline(&mut text).lost_focus() {
                        let parsed: Option<Vec<Value>> = text
                            .split(',')
                            .map(|p| {
                                let (x, y) = p.trim().split_once(':')?;
                                Some(json!([x.trim().parse::<f64>().ok()?, y.trim().parse::<f64>().ok()?]))
                            })
                            .collect();
                        if let Some(p) = parsed {
                            args.insert(key, Value::Array(p));
                            changed = true;
                        }
                    }
                }
                ArgKind::Legs | ArgKind::BirthLegs => {
                    let mut legs = args.get(&key).and_then(|v| v.as_array().cloned()).unwrap_or_default();
                    let mut remove = None;
                    for (li, leg) in legs.iter_mut().enumerate() {
                        ui.group(|ui| {
                            let o = leg.as_object_mut().unwrap();
                            if arg.kind == ArgKind::Legs {
                                for k in ["resource", "from", "to"] {
                                    let mut v = o.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                                    let opts = if k == "resource" { s.plan.resources.clone() } else { endpoints() };
                                    ui.horizontal(|ui| {
                                        ui.label(k);
                                        if combo_str(ui, ("leg", li, k), &mut v, &opts) {
                                            changed = true;
                                        }
                                    });
                                    o.insert(k.into(), json!(v));
                                }
                            } else {
                                let mut v = o.get("field").and_then(|x| x.as_str()).unwrap_or("").to_string();
                                let opts: Vec<String> = fields(true)
                                    .into_iter()
                                    .filter_map(|f| f.strip_prefix("self.").map(|x| x.to_string()))
                                    .collect();
                                ui.horizontal(|ui| {
                                    ui.label("field");
                                    if combo_str(ui, ("bleg", li), &mut v, &opts) {
                                        changed = true;
                                    }
                                });
                                o.insert("field".into(), json!(v));
                            }
                            let key2 = if o.contains_key("amount") || arg.kind == ArgKind::BirthLegs {
                                "amount"
                            } else {
                                "rate"
                            };
                            let mut v = o.get(key2).and_then(|x| x.as_str()).unwrap_or("").to_string();
                            ui.horizontal(|ui| {
                                ui.label(key2);
                                if combo_str(ui, ("legref", li), &mut v, &targets) {
                                    changed = true;
                                }
                            });
                            o.insert(key2.into(), json!(v));
                            if ui.small_button("remove leg").clicked() {
                                remove = Some(li);
                            }
                        });
                    }
                    if let Some(li) = remove {
                        legs.remove(li);
                        changed = true;
                    }
                    if ui.button("add leg").clicked() {
                        legs.push(if arg.kind == ArgKind::Legs {
                            json!({ "resource": "water", "from": "", "to": "", "rate": "" })
                        } else {
                            json!({ "field": "", "amount": "" })
                        });
                        changed = true;
                    }
                    args.insert(key, Value::Array(legs));
                }
            }
        }
    }
    if changed {
        ed.edit();
        ed.draft[pi].rules[ri].nodes[ni].args = args;
    }
    // Composition helper: multiply an input by a regional blend factor.
    let numeric_inputs: Vec<String> = match info.map(|i| i.inputs) {
        Some(Inputs::Positional(n)) => n.iter().map(|s| s.to_string()).collect(),
        Some(Inputs::Named(n)) => n.iter().map(|s| s.to_string()).collect(),
        _ => vec![],
    };
    if !numeric_inputs.is_empty() && rule.domain.kind == "cells" {
        ui.separator();
        ui.label(RichText::new("Regional blend").strong());
        ui.label(
            RichText::new(
                "Multiply an input by lerp(1, factor, region coverage): an explicit, visible composition — not a hidden override.",
            )
            .small(),
        );
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("blend_port")
                .selected_text(numeric_inputs.get(ed.blend_port).cloned().unwrap_or_default())
                .show_ui(ui, |ui| {
                    for (k, n) in numeric_inputs.iter().enumerate() {
                        ui.selectable_value(&mut ed.blend_port, k, n);
                    }
                });
            if ed.blend_region.is_empty() {
                ed.blend_region = s.plan.regions.first().cloned().unwrap_or_default();
            }
            let regions = s.plan.regions.clone();
            combo_str(ui, "blend_region", &mut ed.blend_region, &regions);
        });
        if ui.button("Insert regional factor").clicked() && !ed.blend_region.is_empty() {
            ed.edit();
            insert_blend(&mut ed.draft[pi].rules[ri], ni, ed.blend_port, &ed.blend_region.clone());
            ed.message = "inserted a regional factor; set its parameter below 1 to reduce, above 1 to increase".into();
        }
    }
}
