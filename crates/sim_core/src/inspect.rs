//! Explain-this-value inspection of the last committed tick.
//!
//! Everything here reads retained buffers of the committed tick (snapshot,
//! requested/accepted amounts, factors, slot values). Nothing writes to the
//! world, so inspecting can never change what it explains.

use crate::compiler::FieldPolicy;
use crate::ir::*;
use crate::resolver::Touch;
use crate::world::World;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Contribution {
    pub effect: String,
    pub rule: String,
    pub label: String,
    /// Positive deposits into the inspected value, negative withdrawals.
    pub requested: f64,
    pub accepted: f64,
    /// Number of process elements aggregated (edges, organisms).
    pub elements: usize,
    /// Smallest acceptance factor among aggregated elements.
    pub min_factor: f64,
    /// What limited the process when not fully accepted.
    pub limited_by: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CellExplanation {
    pub tick: u64,
    pub field: String,
    pub unit: String,
    pub cell: usize,
    pub policy: String,
    pub previous: f64,
    pub current: f64,
    pub contributions: Vec<Contribution>,
    /// Lifecycle deposits (deaths) or other engine-owned changes not listed above.
    pub other: f64,
}

fn rule_label(w: &World, rule: &str) -> String {
    for p in &w.active.packages {
        if let Some(r) = p.rules.iter().find(|r| r.rule_id == rule) {
            if !r.label.is_empty() {
                return r.label.clone();
            }
        }
    }
    rule.to_string()
}

impl World {
    /// Value of a plan slot for an element, from the last tick's buffers.
    pub fn slot_value(&self, slot: Slot, i: usize) -> Option<f64> {
        let b = self.buffers().slots.get(slot)?;
        if b.len() == 1 { Some(b[0]) } else { b.get(i).copied() }
    }

    fn limiting(&self, e: usize, elem: usize) -> Option<String> {
        let last = self.last.as_ref()?;
        let r = &last.resolved;
        let alpha = *r.receipts.alpha.get(e)?.get(elem)?;
        if alpha >= 1.0 {
            return None;
        }
        let plan = &last.active.plan;
        let s = &plan.schema;
        let eff = &plan.effects[e];
        let desc = |ep: Endpoint, dest: bool| -> Option<(f64, String)> {
            let (f, cell) = match (ep, eff.dom) {
                (Endpoint::Cell(f), Dom::Entities(a)) => (f, last.spatial[a as usize].cell_of[elem] as usize),
                (Endpoint::Cell(f), _) => (f, elem),
                (Endpoint::EdgeA(f), _) => (f, self.grid.edge_cells(elem).0),
                (Endpoint::EdgeB(f), _) => (f, self.grid.edge_cells(elem).1),
                (Endpoint::Entity(a, f), _) => {
                    let af = &s.archetypes[a as usize].fields[f];
                    return Some((alpha, format!("{} of the organism{}", af.id, if dest { " (capacity)" } else { "" })));
                }
                _ => return None,
            };
            let tab = if dest { &r.dest_factor[f] } else { &r.source_factor[f] };
            let v = tab.as_ref()?[cell];
            Some((
                v,
                format!(
                    "{} at cell {cell}{}",
                    s.cell_fields[f].id,
                    if dest { " (capacity)" } else { " (available inventory)" }
                ),
            ))
        };
        let mut cands = vec![];
        match &eff.kind {
            EffectKind::Process { legs } => {
                for l in legs {
                    cands.extend(desc(l.from, false));
                    cands.extend(desc(l.to, true));
                }
            }
            EffectKind::EdgeTransfer { field, .. } => {
                let (a, b) = self.grid.edge_cells(elem);
                let src = if r.edge_dir[e][elem] { a } else { b };
                if let Some(t) = &r.source_factor[*field] {
                    cands.push((t[src], format!("{} at cell {src} (available inventory)", s.cell_fields[*field].id)));
                }
            }
            _ => {}
        }
        cands.into_iter().find(|(v, _)| *v == alpha).map(|(_, d)| d)
    }

    /// Accepted-versus-requested breakdown of one cell value over the last tick.
    pub fn explain_cell(&self, field: &str, cell: usize) -> Option<CellExplanation> {
        let last = self.last.as_ref()?;
        let prev = self.prev.as_ref()?;
        let plan = &last.active.plan;
        let s = &plan.schema;
        let f = s.cell_field(field)?;
        if cell >= self.grid.cells() {
            return None;
        }
        let info = &s.cell_fields[f];
        let previous = prev.cells.get(f)?[cell];
        let current = self.state.cells[f][cell];
        let r = &last.resolved;
        let mut contributions: Vec<Contribution> = vec![];
        let mut push = |w: &World, e: usize, sign: f64, elems: &[usize], leg: usize, suffix: &str| {
            let eff = &plan.effects[e];
            let (mut req, mut acc, mut minf, mut lim) = (0.0, 0.0, 1.0f64, None);
            let mut n = 0;
            for &el in elems {
                let q = r.receipts.requested[e][leg][el];
                if q == 0.0 {
                    continue;
                }
                n += 1;
                req += q;
                acc += r.accepted[e][leg][el];
                let a = r.receipts.alpha[e][el];
                if a < minf {
                    minf = a;
                    lim = w.limiting(e, el);
                }
            }
            if n == 0 {
                return;
            }
            contributions.push(Contribution {
                effect: eff.id.clone(),
                rule: eff.rule.clone(),
                label: format!("{}{suffix}", rule_label(w, &eff.rule)),
                requested: sign * req,
                accepted: sign * acc,
                elements: n,
                min_factor: minf,
                limited_by: lim,
            });
        };
        match info.policy {
            FieldPolicy::Reservoir { .. } => {
                for &(e, l, t) in &last.active.index.cell[f] {
                    match t {
                        Touch::CellOut => push(self, e, -1.0, &[cell], l, ""),
                        Touch::CellIn => push(self, e, 1.0, &[cell], l, ""),
                        Touch::EdgeAOut | Touch::EdgeBOut | Touch::EdgeAIn | Touch::EdgeBIn => {
                            let want_a = matches!(t, Touch::EdgeAOut | Touch::EdgeAIn);
                            let out = matches!(t, Touch::EdgeAOut | Touch::EdgeBOut);
                            let edges: Vec<usize> = self
                                .grid
                                .incident(cell)
                                .into_iter()
                                .flatten()
                                .filter(|(_, a)| *a == want_a)
                                .map(|x| x.0)
                                .collect();
                            push(
                                self,
                                e,
                                if out { -1.0 } else { 1.0 },
                                &edges,
                                l,
                                if out { " (out)" } else { " (in)" },
                            );
                        }
                        Touch::EdgeSigned => {
                            let dir = &r.edge_dir[e];
                            let (mut outs, mut ins) = (vec![], vec![]);
                            for (edge, is_a) in self.grid.incident(cell).into_iter().flatten() {
                                if dir[edge] == is_a { outs.push(edge) } else { ins.push(edge) }
                            }
                            push(self, e, 1.0, &ins, 0, " (inflow)");
                            push(self, e, -1.0, &outs, 0, " (outflow)");
                        }
                        Touch::EntCellOut(a) | Touch::EntCellIn(a) => {
                            let members: Vec<usize> = last.spatial[a as usize].in_cell(cell).iter().map(|m| *m as usize).collect();
                            let out = matches!(t, Touch::EntCellOut(_));
                            push(self, e, if out { -1.0 } else { 1.0 }, &members, l, "");
                        }
                    }
                }
            }
            FieldPolicy::Integrated => {
                for e in &plan.effects {
                    if let EffectKind::RateCell { field: tf, rate } = e.kind
                        && tf == f
                    {
                        let v = self.slot_value(rate, cell).unwrap_or(0.0)
                            * e.coverage.and_then(|c| self.slot_value(c, cell)).unwrap_or(1.0)
                            * self.scenario.dt;
                        contributions.push(Contribution {
                            effect: e.id.clone(),
                            rule: e.rule.clone(),
                            label: rule_label(self, &e.rule),
                            requested: v,
                            accepted: v,
                            elements: 1,
                            min_factor: 1.0,
                            limited_by: None,
                        });
                    }
                }
            }
            _ => {}
        }
        let explained: f64 = contributions.iter().map(|c| c.accepted).sum();
        let other = current - previous - explained;
        Some(CellExplanation {
            tick: self.state.tick,
            field: field.to_string(),
            unit: info.unit.to_string(),
            cell,
            policy: format!("{:?}", info.policy).split_whitespace().next().unwrap_or("").to_string(),
            previous,
            current,
            contributions,
            other: if other.abs() <= 1e-9 * (previous.abs() + current.abs() + 1.0) {
                0.0
            } else {
                other
            },
        })
    }

    /// Per-organism view: sensed inputs, attempted vs actual actions, receipts.
    pub fn explain_entity(&self, arch: usize, id: u64) -> Option<EntityExplanation> {
        let last = self.last.as_ref()?;
        let plan = &last.active.plan;
        let s = &plan.schema;
        let a = s.archetypes.get(arch)?;
        let prev = self.prev.as_ref()?;
        let row = prev.entities[arch].index_of(id)?;
        let now_row = self.state.entities[arch].index_of(id);
        let e = &self.state.entities[arch];
        let pe = &prev.entities[arch];
        let brain = plan.instrs.iter().find(|i| i.op == Op::Brain(arch as u16));
        let inputs = brain
            .map(|b| {
                b.inputs
                    .iter()
                    .map(|&sl| (plan.slots[sl].name.clone(), self.slot_value(sl, row).unwrap_or(f64::NAN)))
                    .collect()
            })
            .unwrap_or_default();
        let outputs = brain
            .map(|b| b.outputs.iter().map(|&sl| self.slot_value(sl, row).unwrap_or(f64::NAN)).collect())
            .unwrap_or_default();
        let mut processes = vec![];
        for (ei, eff) in plan.effects.iter().enumerate() {
            if eff.dom != Dom::Entities(arch as u16) {
                continue;
            }
            if let EffectKind::Process { legs } = &eff.kind {
                let r = &last.resolved;
                let legs: Vec<(String, f64, f64)> = legs
                    .iter()
                    .enumerate()
                    .map(|(li, l)| {
                        (
                            s.resources[l.resource].id.clone(),
                            r.receipts.requested[ei][li][row],
                            r.accepted[ei][li][row],
                        )
                    })
                    .collect();
                processes.push(EntityProcess {
                    effect: eff.id.clone(),
                    label: rule_label(self, &eff.rule),
                    factor: r.receipts.alpha[ei][row],
                    limited_by: self.limiting(ei, row),
                    legs,
                });
            }
        }
        let (attempted_speed, actual_speed) = match last.active.wiring.moves[arch] {
            Some((_, speed)) => {
                let actual = self.slot_value(speed, row).unwrap_or(0.0).clamp(0.0, a.max_speed);
                let attempted = plan
                    .source_map
                    .iter()
                    .find(|(k, _)| k.ends_with("/speed_request"))
                    .and_then(|(_, v)| v.first())
                    .and_then(|&sl| self.slot_value(sl, row))
                    .unwrap_or(actual);
                (attempted, actual)
            }
            None => (0.0, 0.0),
        };
        let src = now_row.map(|r| (e, r)).unwrap_or((pe, row));
        let (ent, r) = src;
        Some(EntityExplanation {
            id,
            alive: now_row.is_some(),
            x: ent.x[r],
            z: ent.z[r],
            heading: ent.heading[r],
            age: ent.age[r],
            fields: a
                .fields
                .iter()
                .enumerate()
                .map(|(f, fi)| (fi.id.clone(), ent.fields[f][r], pe.fields[f][row]))
                .collect(),
            traits: a
                .traits
                .iter()
                .enumerate()
                .map(|(t, ti)| (ti.id.clone(), ent.genome_of(r)[t]))
                .collect(),
            lineage: ent.lineage[r].clone(),
            inputs,
            outputs,
            attempted_speed,
            actual_speed,
            processes,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct EntityProcess {
    pub effect: String,
    pub label: String,
    pub factor: f64,
    pub limited_by: Option<String>,
    /// (resource, requested, accepted) per leg.
    pub legs: Vec<(String, f64, f64)>,
}

#[derive(Clone, Debug, Serialize)]
pub struct EntityExplanation {
    pub id: u64,
    pub alive: bool,
    pub x: f64,
    pub z: f64,
    pub heading: f64,
    pub age: f64,
    /// (field, current, previous tick).
    pub fields: Vec<(String, f64, f64)>,
    /// Inherited trait values (genome), distinct from transient condition above.
    pub traits: Vec<(String, f64)>,
    pub lineage: crate::state::Lineage,
    pub inputs: Vec<(String, f64)>,
    pub outputs: Vec<f64>,
    pub attempted_speed: f64,
    pub actual_speed: f64,
    pub processes: Vec<EntityProcess>,
}
