//! Seeded world generation: terrain, initial fields, regions, and seed populations.
//!
//! The generator sets elevation, material properties, reservoir amounts, and
//! forcing parameters. It never assigns permanent ecological identities.

use crate::biology;
use crate::compiler::{ArchInfo, FieldPolicy, Schema};
use crate::grid::Grid;
use crate::rng;
use crate::schema::*;
use crate::state::{Entities, Lineage, Origin};

pub const ELEVATION_FIELD: &str = "elevation";
const STREAM_TERRAIN: u64 = 0x7E_0001;
const STREAM_INIT: u64 = 0x7E_0002;
const STREAM_PLACE: u64 = 0x7E_0003;

/// Smooth seeded value noise in [0, 1] with the given feature size (cells).
pub fn value_noise(seed: u64, stream: u64, x: f64, z: f64, feature: f64) -> f64 {
    let (u, v) = (x / feature, z / feature);
    let (i, j) = (u.floor(), v.floor());
    let (fu, fv) = (u - i, v - j);
    let s = |t: f64| t * t * (3.0 - 2.0 * t);
    let h = |a: f64, b: f64| rng::draw(seed, 0, stream, ((a as i64 as u64) << 32) ^ (b as i64 as u64 & 0xFFFF_FFFF), 0);
    let (a, b, c, d) = (h(i, j), h(i + 1.0, j), h(i, j + 1.0), h(i + 1.0, j + 1.0));
    let (su, sv) = (s(fu), s(fv));
    let top = a + (b - a) * su;
    let bot = c + (d - c) * su;
    top + (bot - top) * sv
}

pub fn fbm(seed: u64, stream: u64, x: f64, z: f64, feature: f64, octaves: u32) -> f64 {
    let (mut sum, mut amp, mut norm, mut f) = (0.0, 1.0, 0.0, feature);
    for o in 0..octaves {
        sum += amp * value_noise(seed, stream + o as u64, x, z, f);
        norm += amp;
        amp *= 0.5;
        f *= 0.5;
    }
    sum / norm
}

pub fn terrain(decl: &TerrainDecl, grid: &Grid, seed: u64) -> Result<Vec<f64>, String> {
    let (w, h) = (grid.width, grid.height);
    Ok(match decl {
        TerrainDecl::Flat { elevation } => vec![*elevation; w * h],
        TerrainDecl::Explicit { values } => {
            if values.len() != w * h {
                return Err(format!("explicit terrain has {} values, grid has {}", values.len(), w * h));
            }
            values.clone()
        }
        TerrainDecl::Valley {
            relief,
            valley_depth,
            basin_depth,
            roughness,
        } => {
            let mut e = vec![0.0; w * h];
            let (wf, hf) = (w as f64, h as f64);
            for z in 0..h {
                for x in 0..w {
                    let (xn, zn) = (x as f64 / wf, z as f64 / hf);
                    // Uplands at low z sloping down toward a basin near high z.
                    let slope = relief * (1.0 - zn).powf(1.4);
                    let center = 0.5 + 0.12 * (zn * 7.0).sin() * (1.0 - zn * 0.6);
                    let d = (xn - center) / 0.13;
                    let valley = valley_depth * (-d * d).exp() * (1.0 - 0.5 * zn);
                    let (bx, bz) = ((xn - 0.5) / 0.22, (zn - 0.8) / 0.14);
                    let basin = basin_depth * (-(bx * bx + bz * bz)).exp();
                    let ridge = 0.25 * relief * (xn - 0.5).abs().powf(1.5);
                    let noise = roughness * (fbm(seed, STREAM_TERRAIN, x as f64, z as f64, 24.0, 4) - 0.5);
                    e[z * w + x] = slope + ridge - valley - basin + noise;
                }
            }
            let min = e.iter().cloned().fold(f64::INFINITY, f64::min);
            e.iter_mut().for_each(|v| *v -= min);
            e
        }
    })
}

pub fn normalized(v: &[f64]) -> Vec<f64> {
    let min = v.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = v.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let span = (max - min).max(1e-12);
    v.iter().map(|x| (x - min) / span).collect()
}

pub fn shape_mask(shape: &RegionShape, grid: &Grid) -> Result<Vec<f64>, String> {
    let (w, h) = (grid.width, grid.height);
    let mut m = vec![0.0; w * h];
    match shape {
        RegionShape::Rect { x0, z0, x1, z1 } => {
            for z in (*z0).min(h)..(*z1).min(h) {
                for x in (*x0).min(w)..(*x1).min(w) {
                    m[z * w + x] = 1.0;
                }
            }
        }
        RegionShape::Circle { cx, cz, radius, feather } => {
            for z in 0..h {
                for x in 0..w {
                    let d = ((x as f64 + 0.5 - cx).powi(2) + (z as f64 + 0.5 - cz).powi(2)).sqrt();
                    m[z * w + x] = if d <= *radius {
                        1.0
                    } else if *feather > 0.0 && d < radius + feather {
                        1.0 - (d - radius) / feather
                    } else {
                        0.0
                    };
                }
            }
        }
        RegionShape::Boundary => {
            for c in 0..w * h {
                if grid.is_boundary(c) {
                    m[c] = 1.0;
                }
            }
        }
        RegionShape::Explicit { values } => {
            if values.len() != w * h {
                return Err(format!("explicit region has {} values, grid has {}", values.len(), w * h));
            }
            m = values.clone();
        }
    }
    if m.iter().any(|v| !(0.0..=1.0).contains(v)) {
        return Err("region coverage must be in [0, 1]".into());
    }
    Ok(m)
}

pub fn init_field(
    spec: &InitSpec,
    grid: &Grid,
    seed: u64,
    field: &str,
    elevation: &[f64],
    regions: &[(String, Vec<f64>)],
) -> Result<Vec<f64>, String> {
    let n = grid.cells();
    let en = normalized(elevation);
    let stream = STREAM_INIT ^ rng::hash_str(field);
    Ok(match spec {
        InitSpec::Constant { value } => vec![*value; n],
        InitSpec::Elevation { a, b } => en.iter().map(|e| a + b * e).collect(),
        InitSpec::Noise { lo, hi, feature } => (0..n)
            .map(|c| lo + (hi - lo) * fbm(seed, stream, (c % grid.width) as f64, (c / grid.width) as f64, feature.max(1.0), 3))
            .collect(),
        InitSpec::FillToLevel { level, density, max_depth } => elevation
            .iter()
            .map(|e| (level - e).clamp(0.0, *max_depth) * density * grid.cell_area())
            .collect(),
        InitSpec::Explicit { values } => {
            if values.len() != n {
                return Err(format!("explicit init for {field} has {} values, grid has {n}", values.len()));
            }
            values.clone()
        }
        InitSpec::ScaleOf { .. } => unreachable!("resolved in init_cells"),
        InitSpec::Region { region, inside, outside } => {
            let Some((_, m)) = regions.iter().find(|(r, _)| r == region) else {
                return Err(format!("init for {field} references unknown region {region}"));
            };
            m.iter().map(|c| outside + (inside - outside) * c).collect()
        }
    })
}

/// Initialize all cell fields from defaults, terrain, and scenario init specs.
pub fn init_cells(schema: &Schema, sc: &Scenario, grid: &Grid, regions: &[(String, Vec<f64>)]) -> Result<Vec<Vec<f64>>, String> {
    let n = grid.cells();
    let mut cells: Vec<Vec<f64>> = schema.cell_fields.iter().map(|f| vec![f.default; n]).collect();
    let Some(ei) = schema.cell_field(ELEVATION_FIELD) else {
        return Err(format!("packages must declare a parameter field '{ELEVATION_FIELD}'"));
    };
    if schema.cell_fields[ei].policy != FieldPolicy::Parameter {
        return Err(format!("'{ELEVATION_FIELD}' must be a parameter field (immutable during ticks)"));
    }
    cells[ei] = terrain(&sc.terrain, grid, sc.seed)?;
    let (derived, direct): (Vec<_>, Vec<_>) = sc.initial.iter().partition(|(_, s)| matches!(s, InitSpec::ScaleOf { .. }));
    for (field, spec) in direct.into_iter().chain(derived) {
        let Some(fi) = schema.cell_field(field) else {
            return Err(format!("scenario initializes unknown field {field}"));
        };
        if fi == ei {
            return Err("elevation is set by the terrain generator".into());
        }
        let v = match spec {
            InitSpec::ScaleOf { field: src, factor } => {
                let Some(si) = schema.cell_field(src) else {
                    return Err(format!("init for {field} scales unknown field {src}"));
                };
                if matches!(sc.initial.get(src), Some(InitSpec::ScaleOf { .. })) {
                    return Err(format!("init for {field} cannot scale another derived field"));
                }
                cells[si].iter().map(|x| x * factor).collect()
            }
            _ => init_field(spec, grid, sc.seed, field, &cells[ei], regions)?,
        };
        if v.iter().any(|x| !x.is_finite()) {
            return Err(format!("initial values of {field} are not finite"));
        }
        if matches!(schema.cell_fields[fi].policy, FieldPolicy::Reservoir { .. }) && v.iter().any(|x| *x < 0.0) {
            return Err(format!("initial inventory of {field} is negative"));
        }
        cells[fi] = v;
    }
    // Reservoirs start within capacity.
    for (fi, f) in schema.cell_fields.iter().enumerate() {
        if let FieldPolicy::Reservoir { capacity: Some(cf), .. } = f.policy {
            let cap = cells[cf].clone();
            for (v, c) in cells[fi].iter_mut().zip(cap) {
                *v = v.min(c);
            }
        }
    }
    Ok(cells)
}

pub fn authored_genome(a: &ArchInfo, links: &[(usize, usize, f64)], bias: &[f64], traits: &[f64]) -> Result<Vec<f64>, String> {
    let b = &a.brain;
    if links.len() > b.hidden {
        return Err(format!(
            "authored brain has {} links but only {} hidden units",
            links.len(),
            b.hidden
        ));
    }
    let mut g = traits.to_vec();
    let mut w1 = vec![0.0; b.inputs * b.hidden];
    let b1 = vec![0.0; b.hidden];
    let mut w2 = vec![0.0; b.hidden * b.outputs];
    let mut b2 = vec![0.0; b.outputs];
    for (j, &(i, o, w)) in links.iter().enumerate() {
        if i >= b.inputs || o >= b.outputs {
            return Err(format!("authored link ({i}, {o}) out of range"));
        }
        w1[j * b.inputs + i] = 1.0;
        w2[o * b.hidden + j] = w.clamp(-b.weight_bound, b.weight_bound);
    }
    for (k, v) in bias.iter().enumerate().take(b.outputs) {
        b2[k] = v.clamp(-b.weight_bound, b.weight_bound);
    }
    g.extend(w1);
    g.extend(b1);
    g.extend(w2);
    g.extend(b2);
    Ok(g)
}

/// Place organisms uniformly (optionally inside a mask), with seeded genomes.
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    a: &ArchInfo,
    ents: &mut Entities,
    next_id: &mut u64,
    grid: &Grid,
    seed: u64,
    tick: u64,
    decl: &PopulationDecl,
    mask: Option<&[f64]>,
    origin_override: Option<Origin>,
) -> Result<usize, String> {
    let (ex, ez) = (grid.extent_x(), grid.extent_z());
    let authored = decl.genome.starts_with("authored");
    if !authored && decl.genome != "random" {
        return Err(format!("unknown genome source {}", decl.genome));
    }
    let mut placed = 0;
    for k in 0..decl.count {
        let id = *next_id;
        let mut pos = None;
        for attempt in 0..64u64 {
            let x = rng::draw(seed, tick, STREAM_PLACE, id, attempt * 3) * ex;
            let z = rng::draw(seed, tick, STREAM_PLACE, id, attempt * 3 + 1) * ez;
            let ok = mask.is_none_or(|m| m[grid.cell_at(x, z)] > 0.5);
            if ok {
                pos = Some((x, z));
                break;
            }
        }
        let Some((x, z)) = pos else {
            if k == 0 {
                return Err("could not place organisms inside the requested region".into());
            }
            break;
        };
        let mut genome = biology::random_genome(a, seed ^ tick, id);
        for (name, v) in &decl.traits {
            let Some(ti) = a.traits.iter().position(|t| &t.id == name) else {
                return Err(format!("unknown trait override {name}"));
            };
            let t = &a.traits[ti];
            if !(t.min..=t.max).contains(v) {
                return Err(format!("trait override {name} = {v} outside [{}, {}]", t.min, t.max));
            }
            genome[ti] = *v;
        }
        if authored {
            genome = authored_genome(a, &decl.authored_links, &decl.authored_bias, &genome[..a.traits.len()])?;
        }
        if !biology::genome_valid(a, &genome) {
            return Err("seed genome violates declared bounds".into());
        }
        let heading = rng::draw(seed, tick, STREAM_PLACE, id, 2) * std::f64::consts::TAU;
        let origin = origin_override.unwrap_or(if authored { Origin::Authored } else { Origin::Random });
        let lineage = Lineage {
            parent: 0,
            root: id,
            generation: 0,
            birth_tick: tick,
            birth_x: x,
            birth_z: z,
            origin,
            offspring: 0,
            mutation_version: biology::MUTATION_VERSION,
        };
        ents.push(id, x, z, heading, &a.seed_state, &genome, lineage, 0.0);
        *next_id += 1;
        placed += 1;
    }
    Ok(placed)
}
