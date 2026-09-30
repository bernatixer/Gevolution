//! Animals: every simulated herbivore is an animated deer (or a stag, if it
//! inherited a large body). Its animation follows what it is actually doing.

use crate::sim::{Action, Speed};
use crate::view::{ViewState, height_at};
use crate::ClientState;
use bevy::prelude::*;
use bevy::world_serialization::{WorldAsset, WorldAssetRoot, WorldInstanceReady};
use std::collections::HashMap;

/// Clip indices inside the Quaternius animal GLBs.
const WALK: usize = 9;
const GALLOP: usize = 4;
const EAT: usize = 3;
const IDLE: usize = 12;
const DRINK: usize = 10;

#[derive(Resource, Default)]
pub struct Fauna {
    ready: bool,
    scenes: [Handle<WorldAsset>; 2],
    graph: Handle<AnimationGraph>,
    nodes: HashMap<usize, AnimationNodeIndex>,
    animals: HashMap<u64, Animal>,
}

struct Animal {
    root: Entity,
    player: Option<Entity>,
    playing: Option<Action>,
    stag: bool,
    /// Drawn pose, eased toward the simulated one so motion stays smooth between ticks.
    pos: Vec3,
    rot: Quat,
    /// A new action must hold for a moment before its clip replaces the current one.
    pending: Option<(Action, f32)>,
}

#[derive(Component)]
pub struct AnimalRoot(pub u64);

fn clip(a: Action) -> usize {
    match a {
        Action::Idle => IDLE,
        Action::Walk => WALK,
        Action::Run => GALLOP,
        Action::Eat => EAT,
        Action::Drink => DRINK,
    }
}

pub fn setup_fauna(mut fauna: ResMut<Fauna>, assets: Res<AssetServer>, mut graphs: ResMut<Assets<AnimationGraph>>) {
    let path = "client/models/animals/deer.glb";
    let clips = [WALK, GALLOP, EAT, IDLE, DRINK];
    let (graph, nodes) = AnimationGraph::from_clips(clips.iter().map(|i| assets.load(GltfAssetLabel::Animation(*i).from_asset(path))));
    fauna.graph = graphs.add(graph);
    fauna.nodes = clips.iter().copied().zip(nodes).collect();
    fauna.scenes = [
        assets.load(GltfAssetLabel::Scene(0).from_asset("client/models/animals/deer.glb")),
        assets.load(GltfAssetLabel::Scene(0).from_asset("client/models/animals/stag.glb")),
    ];
    fauna.ready = true;
}

/// Remember each animal's animation player once its model has spawned.
pub fn on_animal_ready(ready: On<WorldInstanceReady>, mut fauna: ResMut<Fauna>, roots: Query<&AnimalRoot>, children: Query<&Children>, players: Query<(), With<AnimationPlayer>>, mut commands: Commands) {
    let Ok(AnimalRoot(id)) = roots.get(ready.entity) else { return };
    let graph = fauna.graph.clone();
    let Some(player) = children.iter_descendants(ready.entity).find(|c| players.get(*c).is_ok()) else { return };
    commands.entity(player).insert((AnimationGraphHandle(graph), AnimationTransitions::new()));
    if let Some(a) = fauna.animals.get_mut(id) {
        a.player = Some(player);
        a.playing = None;
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update_fauna(
    mut commands: Commands,
    state: Res<ClientState>,
    view: Res<ViewState>,
    time: Res<Time>,
    mut fauna: ResMut<Fauna>,
    mut transforms: Query<&mut Transform>,
    mut players: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
) {
    if !fauna.ready {
        return;
    }
    let Some(s) = state.snapshot.clone() else { return };
    let vs = view.vertical_scale;
    let dt = time.delta_secs();
    let ease = 1.0 - (-dt * 10.0).exp();
    // Clips play at the pace of simulated time (frozen when paused), capped so fast-forward stays readable.
    let pace = if s.speed == Speed::Paused { 0.0 } else { ((s.ticks_per_second * s.dt) as f32).clamp(0.6, 2.5) };
    let mut alive: HashMap<u64, ()> = HashMap::new();
    let Fauna { scenes, nodes, animals, .. } = &mut *fauna;
    for ents in &s.entities {
        for e in ents {
            alive.insert(e.id, ());
            let stag = e.size > 1.12;
            // Presentation scale: animals are drawn larger than life so they read from afar.
            let k = 2.2 + e.size * 1.6;
            let target = Vec3::new(e.x, height_at(&s, vs, e.x, e.z), e.z);
            let target_rot = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2 - e.heading);
            let needs_spawn = animals.get(&e.id).is_none_or(|a| a.stag != stag);
            if needs_spawn {
                if let Some(old) = animals.remove(&e.id) {
                    commands.entity(old.root).despawn();
                }
                let t = Transform::from_translation(target).with_rotation(target_rot).with_scale(Vec3::splat(k));
                let root = commands.spawn((AnimalRoot(e.id), WorldAssetRoot(scenes[usize::from(stag)].clone()), t)).id();
                animals.insert(e.id, Animal { root, player: None, playing: None, stag, pos: target, rot: target_rot, pending: None });
                continue;
            }
            let a = animals.get_mut(&e.id).unwrap();
            // Snap on big jumps (teleports, map wrap); ease otherwise.
            if a.pos.distance(target) > 60.0 {
                a.pos = target;
            } else {
                a.pos = a.pos.lerp(target, ease);
            }
            a.pos.y = height_at(&s, vs, a.pos.x, a.pos.z);
            a.rot = a.rot.slerp(target_rot, ease);
            if let Ok(mut tr) = transforms.get_mut(a.root) {
                *tr = Transform::from_translation(a.pos).with_rotation(a.rot).with_scale(Vec3::splat(k));
            }
            let Some(pe) = a.player else { continue };
            let Ok((mut player, mut transitions)) = players.get_mut(pe) else { continue };
            let want = e.action;
            let switch = match (a.playing, &mut a.pending) {
                (None, _) => true,
                (Some(cur), _) if cur == want => {
                    a.pending = None;
                    false
                }
                (_, Some((p, held))) if *p == want => {
                    *held += dt * pace.max(0.3);
                    *held > 0.4
                }
                _ => {
                    a.pending = Some((want, 0.0));
                    false
                }
            };
            if switch && let Some(n) = nodes.get(&clip(want)) {
                let anim = transitions.play(&mut player, *n, std::time::Duration::from_millis(300)).repeat();
                if a.playing.is_none() {
                    // Desynchronize herds a little.
                    anim.seek_to((e.id % 17) as f32 * 0.07);
                }
                a.playing = Some(want);
                a.pending = None;
            }
            for (_, anim) in player.playing_animations_mut() {
                anim.set_speed(pace);
            }
        }
    }
    let dead: Vec<u64> = animals.keys().filter(|k| !alive.contains_key(k)).copied().collect();
    for id in dead {
        if let Some(a) = animals.remove(&id) {
            commands.entity(a.root).despawn();
        }
    }
}
