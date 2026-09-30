//! Saves, checkpoints, and replay compatibility.
//!
//! Format: `EVWSAVE1` magic, u64 little-endian header length, a JSON header
//! (schema-versioned, with build/profile fingerprint and canonical source
//! packages), then a blob of little-endian f64 values whose layout the header
//! declares. Compiled plans are never saved; they are rebuilt from sources.
//! Loading treats the file as untrusted: sizes are checked before allocation.

use crate::commands::Command;
use crate::compiler::Schema;
use crate::eval::Buffers;
use crate::schema::{Package, Scenario};
use crate::state::{Entities, Lineage, State};
use crate::world::{self, Active, ResourceLedger, RunConfig, Stats, World};
use crate::worldgen;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;

pub const MAGIC: &[u8; 8] = b"EVWSAVE1";
pub const FORMAT_VERSION: u32 = 1;
pub const MAX_HEADER_BYTES: u64 = 256 << 20;
pub const MAX_SAVE_BYTES: u64 = 4 << 30;

#[derive(Serialize, Deserialize)]
struct ArchSave {
    id: String,
    fields: Vec<String>,
    genome_len: usize,
    ids: Vec<u64>,
    lineage: Vec<Lineage>,
}

#[derive(Serialize, Deserialize)]
struct Header {
    format: String,
    format_version: u32,
    build: String,
    scenario: Scenario,
    packages: Vec<Package>,
    package_hash: u64,
    tick: u64,
    next_entity_id: u64,
    next_seq: u64,
    regions: Vec<String>,
    cell_fields: Vec<String>,
    params: Vec<(String, f64)>,
    accounts: Vec<(String, f64, f64)>,
    /// (resource, intervention_in, intervention_out, roundoff)
    resources: Vec<(String, f64, f64, f64)>,
    ledger: Vec<ResourceLedger>,
    archetypes: Vec<ArchSave>,
    pending: Vec<Command>,
    log: Vec<Command>,
    stats: Stats,
    blob_values: u64,
}

fn push(blob: &mut Vec<u8>, v: &[f64]) {
    for x in v {
        blob.extend_from_slice(&x.to_le_bytes());
    }
}

pub fn save(w: &World) -> Vec<u8> {
    let s = w.schema();
    let st = &w.state;
    let mut blob = vec![];
    for c in st.cells.iter().chain(&st.regions) {
        push(&mut blob, c);
    }
    let mut archetypes = vec![];
    for (ai, a) in s.archetypes.iter().enumerate() {
        let e = &st.entities[ai];
        for col in [&e.x, &e.z, &e.heading, &e.age, &e.cooldown] {
            push(&mut blob, col);
        }
        for col in &e.fields {
            push(&mut blob, col);
        }
        push(&mut blob, &e.genome);
        archetypes.push(ArchSave {
            id: a.id.clone(),
            fields: a.fields.iter().map(|f| f.id.clone()).collect(),
            genome_len: e.genome_len,
            ids: e.ids.clone(),
            lineage: e.lineage.clone(),
        });
    }
    let header = Header {
        format: "evolving-worlds-save".into(),
        format_version: FORMAT_VERSION,
        build: world::build_fingerprint(),
        scenario: w.scenario.clone(),
        packages: w.active.packages.clone(),
        package_hash: w.plan().source_hash,
        tick: st.tick,
        next_entity_id: st.next_entity_id,
        next_seq: w.next_seq,
        regions: s.regions.clone(),
        cell_fields: s.cell_fields.iter().map(|f| f.id.clone()).collect(),
        params: w.plan().params.iter().zip(&st.params).map(|(p, v)| (p.name.clone(), *v)).collect(),
        accounts: s
            .accounts
            .iter()
            .enumerate()
            .map(|(i, a)| (a.id.clone(), st.account_in[i], st.account_out[i]))
            .collect(),
        resources: s
            .resources
            .iter()
            .enumerate()
            .map(|(i, r)| (r.id.clone(), st.intervention_in[i], st.intervention_out[i], st.roundoff[i]))
            .collect(),
        ledger: w.ledger.clone(),
        archetypes,
        pending: w.pending.clone(),
        log: w.log.clone(),
        stats: w.stats.clone(),
        blob_values: (blob.len() / 8) as u64,
    };
    let h = serde_json::to_vec(&header).expect("header serializes");
    let mut out = Vec::with_capacity(16 + h.len() + blob.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(h.len() as u64).to_le_bytes());
    out.extend_from_slice(&h);
    out.extend_from_slice(&blob);
    out
}

struct Reader<'a> {
    blob: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<Vec<f64>, String> {
        let bytes = n.checked_mul(8).ok_or("save layout overflow")?;
        if self.pos + bytes > self.blob.len() {
            return Err("save data is truncated".into());
        }
        let v = self.blob[self.pos..self.pos + bytes]
            .as_chunks::<8>()
            .0
            .iter()
            .map(|c| f64::from_le_bytes(*c))
            .collect();
        self.pos += bytes;
        Ok(v)
    }
}

pub fn load(bytes: &[u8], config: RunConfig) -> Result<World, String> {
    if bytes.len() as u64 > MAX_SAVE_BYTES {
        return Err("save file too large".into());
    }
    if bytes.len() < 16 || &bytes[..8] != MAGIC {
        return Err("not an Evolving Worlds save".into());
    }
    let hlen = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
    if hlen > MAX_HEADER_BYTES || hlen > (bytes.len() - 16) as u64 {
        return Err("save header length is invalid".into());
    }
    let header: Header = serde_json::from_slice(&bytes[16..16 + hlen as usize]).map_err(|e| format!("invalid save header: {e}"))?;
    if header.format != "evolving-worlds-save" || header.format_version != FORMAT_VERSION {
        return Err(format!("unsupported save format {} v{}", header.format, header.format_version));
    }
    let build = world::build_fingerprint();
    if header.build != build {
        return Err(format!(
            "incompatible build/profile: save was written by {}, this engine is {build}; exact replay is only supported on the same build",
            header.build
        ));
    }
    let blob = &bytes[16 + hlen as usize..];
    if header.blob_values.checked_mul(8) != Some(blob.len() as u64) {
        return Err("save data length does not match its header".into());
    }
    world::validate_scenario(&header.scenario)?;
    let sc = header.scenario;
    let env = world::compile_env(&sc, header.regions.clone());
    let active = Active::build(header.packages, &env, &sc, &config)?;
    if active.plan.source_hash != header.package_hash {
        return Err("saved package hash does not match its packages".into());
    }
    let schema: &Schema = &active.plan.schema;
    let names: Vec<String> = schema.cell_fields.iter().map(|f| f.id.clone()).collect();
    if names != header.cell_fields {
        return Err("saved cell fields do not match the compiled schema".into());
    }
    let n = sc.grid.width * sc.grid.height;
    let mut r = Reader { blob, pos: 0 };
    let mut cells = vec![];
    for _ in 0..schema.cell_fields.len() {
        cells.push(r.take(n)?);
    }
    let mut regions = vec![];
    for _ in 0..header.regions.len() {
        regions.push(r.take(n)?);
    }
    if header.archetypes.len() != schema.archetypes.len() {
        return Err("saved archetypes do not match the compiled schema".into());
    }
    let mut entities = vec![];
    for (a, sv) in schema.archetypes.iter().zip(header.archetypes) {
        let fields: Vec<String> = a.fields.iter().map(|f| f.id.clone()).collect();
        if sv.id != a.id || sv.fields != fields || sv.genome_len != a.genome_len() || sv.lineage.len() != sv.ids.len() {
            return Err(format!("saved archetype {} does not match the compiled schema", sv.id));
        }
        if sv.ids.windows(2).any(|w| w[0] >= w[1]) {
            return Err("saved entity ids are not strictly increasing".into());
        }
        let m = sv.ids.len();
        if m > sc.population_cap {
            return Err("saved population exceeds the cap".into());
        }
        let mut e = Entities::new(a.fields.len(), a.genome_len());
        e.ids = sv.ids;
        e.lineage = sv.lineage;
        e.x = r.take(m)?;
        e.z = r.take(m)?;
        e.heading = r.take(m)?;
        e.age = r.take(m)?;
        e.cooldown = r.take(m)?;
        for f in 0..a.fields.len() {
            e.fields[f] = r.take(m)?;
        }
        e.genome = r.take(m.checked_mul(a.genome_len()).ok_or("genome overflow")?)?;
        entities.push(e);
    }
    if r.pos != blob.len() {
        return Err("save data has trailing values".into());
    }
    let find = |v: &[(String, f64, f64)], id: &str| v.iter().find(|x| x.0 == id).map(|x| (x.1, x.2)).unwrap_or((0.0, 0.0));
    let params: Vec<f64> = active
        .plan
        .params
        .iter()
        .map(|p| header.params.iter().find(|x| x.0 == p.name).map(|x| x.1).unwrap_or(p.value))
        .collect();
    let res = |id: &str| {
        header
            .resources
            .iter()
            .find(|x| x.0 == id)
            .map(|x| (x.1, x.2, x.3))
            .unwrap_or((0.0, 0.0, 0.0))
    };
    let state = State {
        tick: header.tick,
        cells,
        regions,
        entities,
        params,
        account_in: schema.accounts.iter().map(|a| find(&header.accounts, &a.id).0).collect(),
        account_out: schema.accounts.iter().map(|a| find(&header.accounts, &a.id).1).collect(),
        intervention_in: schema.resources.iter().map(|x| res(&x.id).0).collect(),
        intervention_out: schema.resources.iter().map(|x| res(&x.id).1).collect(),
        roundoff: schema.resources.iter().map(|x| res(&x.id).2).collect(),
        next_entity_id: header.next_entity_id,
    };
    let all_finite = state.cells.iter().chain(&state.regions).flatten().all(|v| v.is_finite())
        && state.entities.iter().all(|e| {
            e.fields
                .iter()
                .flatten()
                .chain(&e.genome)
                .chain(&e.x)
                .chain(&e.z)
                .all(|v| v.is_finite())
        });
    if !all_finite {
        return Err("save contains non-finite values".into());
    }
    let elevation = schema.cell_field(worldgen::ELEVATION_FIELD).ok_or("missing elevation field")?;
    let elevation_norm = worldgen::normalized(&state.cells[elevation]);
    let grid = crate::grid::Grid {
        id: sc.grid.id.clone(),
        width: sc.grid.width,
        height: sc.grid.height,
        cell_size: sc.grid.cell_size,
        chunk_size: sc.grid.chunk_size,
    };
    let pool = config
        .threads
        .map(|n| Arc::new(rayon::ThreadPoolBuilder::new().num_threads(n).build().expect("thread pool")));
    let mut w = World {
        scenario: sc,
        active,
        grid,
        state,
        prev: None,
        elevation_norm,
        config,
        pool,
        forcing: vec![],
        bufs: Buffers::default(),
        spare: None,
        pending: header.pending,
        log: header.log,
        next_seq: header.next_seq,
        last: None,
        ledger: header.ledger,
        failure: None,
        events: VecDeque::new(),
        history: VecDeque::new(),
        stats: header.stats,
        timings: Default::default(),
    };
    w.sample();
    Ok(w)
}

pub fn save_to(w: &World, path: &std::path::Path) -> Result<(), String> {
    std::fs::write(path, save(w)).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn load_from(path: &std::path::Path, config: RunConfig) -> Result<World, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > MAX_SAVE_BYTES {
        return Err("save file too large".into());
    }
    load(&std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?, config)
}
