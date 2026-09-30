//! Vegetation and ground clutter that express the simulated state of each
//! spot: moisture, plant biomass, temperature, elevation, water, and slope
//! choose which model grows there. Presentation only.

use crate::view::{Scene3d, ViewState, field, hash01};
use crate::ClientState;
use bevy::prelude::*;
use bevy::world_serialization::{WorldAsset, WorldAssetRoot};
use std::collections::HashMap;

const BROADLEAF: [&str; 6] = ["tree_default", "tree_oak", "tree_detailed", "tree_fat", "tree_tall", "tree_simple"];
const PINES: [&str; 5] = ["tree_pineRoundA", "tree_pineRoundC", "tree_pineTallA", "tree_pineTallB", "tree_pineDefaultA"];
const BUSHES: [&str; 4] = ["plant_bush", "plant_bushDetailed", "plant_bushLarge", "plant_bushSmall"];
const GRASS: [&str; 4] = ["grass", "grass_large", "grass_leafs", "grass_leafsLarge"];
const FLOWERS: [&str; 4] = ["flower_purpleA", "flower_redA", "flower_yellowA", "flower_yellowB"];
const DRY: [&str; 2] = ["cactus_short", "cactus_tall"];
const PALMS: [&str; 2] = ["tree_palmTall", "tree_palmShort"];
const MUSHROOMS: [&str; 2] = ["mushroom_redGroup", "mushroom_tanGroup"];
const LILIES: [&str; 2] = ["lily_large", "lily_small"];
const ROCKS: [&str; 6] = ["rock_largeA", "rock_largeB", "rock_tallA", "rock_tallB", "stone_smallA", "stone_smallC"];
const DEAD: [&str; 2] = ["stump_old", "log"];

#[derive(Resource, Default)]
pub struct Flora {
    scenes: HashMap<String, Handle<WorldAsset>>,
    slots: Vec<Slot>,
    built_for: Option<(usize, usize, f32)>,
    last_tick: Option<u64>,
}

struct Slot {
    cell: usize,
    x: f32,
    z: f32,
    hash: u64,
    shown: Option<(&'static str, f32)>,
    entity: Option<Entity>,
    pending: Option<&'static str>,
    pending_count: u8,
}

fn pick(set: &'static [&'static str], h: u64) -> &'static str {
    set[(sim_core::rng::mix(h) % set.len() as u64) as usize]
}

/// Which model (and model scale) best expresses the state of a spot.
fn choose(h: u64, b: f32, moisture: f32, temp_c: f32, e: f32, depth: f32, slope: f32, detritus: f32) -> Option<(&'static str, f32)> {
    let r = hash01(h, 17);
    if depth > 0.25 {
        return (r < 0.18).then(|| (pick(&LILIES, h), 14.0));
    }
    if depth > 0.015 {
        return None;
    }
    if slope > 0.34 && r < 0.35 {
        return Some((pick(&ROCKS, h), 7.0 + 8.0 * hash01(h, 3)));
    }
    let hot_dry = temp_c > 21.0 && moisture < 0.22;
    if b < 150.0 {
        if hot_dry && r < 0.12 {
            return Some((pick(&DRY, h), 10.0));
        }
        if b > 40.0 && r < 0.3 {
            return Some((pick(&DEAD, h), 9.0));
        }
        return (moisture > 0.3 && r < 0.35).then(|| (pick(&GRASS, h), 14.0));
    }
    if b < 650.0 {
        if hot_dry && r < 0.25 {
            return Some((pick(&DRY, h), 10.0));
        }
        if moisture > 0.4 && temp_c > 9.0 && r < 0.28 {
            return Some((pick(&FLOWERS, h), 18.0));
        }
        if r < 0.6 {
            return Some((pick(&BUSHES, h), 13.0 + 6.0 * (b / 650.0)));
        }
        return Some((pick(&GRASS, h), 14.0));
    }
    // Forest: denser canopy as biomass grows, with clearings of shrubs between.
    let density = 0.3 + 0.65 * ((b - 650.0) / 1800.0).clamp(0.0, 1.0);
    if hash01(h, 23) > density {
        return Some(if r < 0.5 { (pick(&BUSHES, h), 17.0) } else { (pick(&GRASS, h), 14.0) });
    }
    let size = 11.0 + 7.0 * ((b - 650.0) / 2400.0).clamp(0.0, 1.0);
    if moisture > 0.55 && detritus > 150.0 && r < 0.06 {
        return Some((pick(&MUSHROOMS, h), 14.0));
    }
    if hot_dry && r < 0.4 {
        return Some((pick(&PALMS, h), size * 0.9));
    }
    if e > 0.55 || temp_c < 7.0 {
        return Some((pick(&PINES, h), size));
    }
    let tree = pick(&BROADLEAF, h);
    // Drought stress turns broadleaf trees to autumn colors.
    if moisture < 0.2 {
        let fall: &'static str = match tree {
            "tree_default" => "tree_default_fall",
            "tree_oak" => "tree_oak_fall",
            "tree_detailed" => "tree_detailed_fall",
            "tree_fat" => "tree_fat_fall",
            "tree_tall" => "tree_tall_fall",
            _ => "tree_simple_fall",
        };
        return Some((fall, size));
    }
    Some((tree, size))
}

#[allow(clippy::too_many_arguments)]
pub fn update_flora(
    mut commands: Commands,
    state: Res<ClientState>,
    view: Res<ViewState>,
    scene: Res<Scene3d>,
    mut flora: ResMut<Flora>,
    assets: Res<AssetServer>,
    mut transforms: Query<&mut Transform>,
) {
    let Some(s) = state.snapshot.clone() else { return };
    let (w, h, cs) = (s.width, s.height, s.cell_size as f32);
    let vs = view.vertical_scale;
    if scene.slope.len() != w * h {
        return;
    }
    if flora.built_for != Some((w, h, vs)) {
        for slot in flora.slots.drain(..) {
            if let Some(e) = slot.entity {
                commands.entity(e).despawn();
            }
        }
        // Blue-noise-ish lattice with jitter: denser than one per cell would be too heavy.
        let stride = (w / 128).max(1) as f32 * 1.4;
        let mut z = stride * 0.5;
        let mut k = 0u64;
        while z < h as f32 {
            let mut x = stride * 0.5;
            while x < w as f32 {
                let hsh = sim_core::rng::mix(k ^ 0xF10A);
                let jx = (hash01(hsh, 1) - 0.5) * stride;
                let jz = (hash01(hsh, 2) - 0.5) * stride;
                let (px, pz) = ((x + jx).clamp(0.0, w as f32 - 0.01), (z + jz).clamp(0.0, h as f32 - 0.01));
                flora.slots.push(Slot { cell: pz as usize * w + px as usize, x: px * cs, z: pz * cs, hash: hsh, shown: None, entity: None, pending: None, pending_count: 0 });
                x += stride;
                k += 1;
            }
            z += stride;
        }
        flora.built_for = Some((w, h, vs));
        flora.last_tick = None;
    }
    if flora.last_tick == Some(s.tick) {
        return;
    }
    let first = flora.last_tick.is_none();
    flora.last_tick = Some(s.tick);
    let get = |f: Option<&[f32]>, c: usize| f.map(|v| v[c]).unwrap_or(0.0);
    let (soil, cap, veg, sw, temp, det) = (
        field(&s, "soil_water"),
        field(&s, "soil_water_capacity"),
        field(&s, "vegetation_biomass"),
        field(&s, "surface_water"),
        field(&s, "temperature"),
        field(&s, "detritus_biomass"),
    );
    let emin = s.elevation.iter().cloned().fold(f32::INFINITY, f32::min);
    let emax = s.elevation.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let area = cs * cs;
    let Flora { scenes, slots, .. } = &mut *flora;
    for slot in slots.iter_mut() {
        let c = slot.cell;
        let moisture = if get(cap, c) > 0.0 { (get(soil, c) / get(cap, c)).clamp(0.0, 1.0) } else { 0.0 };
        let depth = get(sw, c) / (1000.0 * area);
        let e = (s.elevation[c] - emin) / (emax - emin).max(1e-6);
        let want = choose(slot.hash, get(veg, c), moisture, get(temp, c) - 273.15, e, depth, scene.slope[c], get(det, c));
        let want_name = want.map(|w| w.0);
        let shown_name = slot.shown.map(|w| w.0);
        // Hysteresis: a new look must persist a few updates before it replaces the old one.
        if want_name != shown_name {
            if slot.pending == want_name.or(Some("")) {
                slot.pending_count += 1;
            } else {
                slot.pending = Some(want_name.unwrap_or(""));
                slot.pending_count = 1;
            }
            if first || slot.pending_count >= 4 {
                if let Some(ent) = slot.entity.take() {
                    commands.entity(ent).despawn();
                }
                slot.shown = want;
                slot.pending = None;
                if let Some((name, _)) = want {
                    let handle = scenes
                        .entry(name.to_string())
                        .or_insert_with(|| assets.load(GltfAssetLabel::Scene(0).from_asset(format!("client/models/nature/{name}.glb"))))
                        .clone();
                    slot.entity = Some(commands.spawn((WorldAssetRoot(handle), Transform::default())).id());
                }
            }
        } else {
            slot.pending = None;
        }
        // Keep transform in step with growth and water level.
        if let (Some(ent), Some((name, base))) = (slot.entity, want.or(slot.shown)) {
            let jitter = 0.8 + 0.4 * hash01(slot.hash, 5);
            let k = base * jitter;
            let y = if name.starts_with("lily") { s.elevation[c] * vs + depth * vs + 0.2 } else { s.elevation[c] * vs - 0.15 };
            let t = Transform::from_xyz(slot.x, y, slot.z)
                .with_rotation(Quat::from_rotation_y(hash01(slot.hash, 9) * std::f32::consts::TAU))
                .with_scale(Vec3::splat(k));
            if let Ok(mut tr) = transforms.get_mut(ent) {
                *tr = t;
            } else {
                commands.entity(ent).insert(t);
            }
        }
    }
}
