//! Presentation: terrain and water meshes, overlays, vegetation and organism
//! instances, cameras, picking, and gizmos. Nothing here feeds back into the
//! simulation except explicit, validated commands.

use crate::sim::{Snapshot, ToSim};
use crate::{ClientState, SimLink};
use bevy::asset::RenderAssetUsages;
use bevy::camera::ScalingMode;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use sim_core::commands::CommandKind;
use sim_core::schema::RegionShape;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlay {
    Natural,
    Elevation,
    Temperature,
    SurfaceWater,
    SoilMoisture,
    Groundwater,
    Vegetation,
    Nutrients,
    Region,
    Flow,
}

impl Overlay {
    pub const ALL: [Overlay; 10] = [
        Overlay::Natural,
        Overlay::Elevation,
        Overlay::Temperature,
        Overlay::SurfaceWater,
        Overlay::SoilMoisture,
        Overlay::Groundwater,
        Overlay::Vegetation,
        Overlay::Nutrients,
        Overlay::Region,
        Overlay::Flow,
    ];
    pub fn label(&self) -> &'static str {
        match self {
            Overlay::Natural => "Natural",
            Overlay::Elevation => "Elevation",
            Overlay::Temperature => "Temperature",
            Overlay::SurfaceWater => "Surface water depth",
            Overlay::SoilMoisture => "Soil moisture",
            Overlay::Groundwater => "Groundwater",
            Overlay::Vegetation => "Vegetation",
            Overlay::Nutrients => "Soil nutrients",
            Overlay::Region => "Region scope",
            Overlay::Flow => "Flow direction",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ramp {
    Terrain,
    Diverging,
    Blues,
    Greens,
    Moisture,
    Purples,
    Orange,
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn stops(s: &[[f32; 3]], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0) * (s.len() - 1) as f32;
    let i = (t.floor() as usize).min(s.len() - 2);
    lerp3(s[i], s[i + 1], t - i as f32)
}

pub fn ramp(r: Ramp, t: f32) -> [f32; 3] {
    match r {
        Ramp::Terrain => stops(&[[0.16, 0.36, 0.22], [0.52, 0.55, 0.30], [0.62, 0.50, 0.36], [0.85, 0.85, 0.82]], t),
        Ramp::Diverging => stops(
            &[
                [0.13, 0.28, 0.62],
                [0.55, 0.72, 0.88],
                [0.96, 0.95, 0.92],
                [0.94, 0.62, 0.40],
                [0.70, 0.12, 0.13],
            ],
            t,
        ),
        Ramp::Blues => stops(&[[0.93, 0.95, 0.97], [0.55, 0.74, 0.88], [0.18, 0.44, 0.72], [0.05, 0.19, 0.42]], t),
        Ramp::Greens => stops(&[[0.95, 0.94, 0.86], [0.66, 0.82, 0.52], [0.25, 0.60, 0.28], [0.05, 0.30, 0.14]], t),
        Ramp::Moisture => stops(&[[0.80, 0.66, 0.44], [0.62, 0.66, 0.46], [0.30, 0.58, 0.58], [0.10, 0.32, 0.55]], t),
        Ramp::Purples => stops(&[[0.95, 0.94, 0.97], [0.72, 0.66, 0.84], [0.45, 0.33, 0.66], [0.25, 0.10, 0.45]], t),
        Ramp::Orange => stops(&[[0.62, 0.62, 0.62], [0.98, 0.60, 0.20]], t),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    Orbit,
    TopDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Inspect,
    AddWater,
    SpawnOrganisms,
}

#[derive(Resource)]
pub struct ViewState {
    pub mode: ViewMode,
    pub overlay: Overlay,
    pub vertical_scale: f32,
    pub show_water: bool,
    pub show_vegetation: bool,
    pub show_organisms: bool,
    pub tool: Tool,
    pub brush_radius: f32,
    pub region: String,
    pub water_amount: f64,
    pub spawn_count: usize,
    pub inspect_field: String,
    pub selected_cell: Option<usize>,
    pub selected_entity: Option<u64>,
    pub hover_cell: Option<usize>,
    /// Viewport rect in physical pixels (x, y, w, h), set by the UI layout.
    pub viewport: Option<(u32, u32, u32, u32)>,
    pub last_paint: Option<(usize, f64)>,
    pub press_pos: Option<Vec2>,
    pub orbit_target: Vec3,
    pub orbit_yaw: f32,
    pub orbit_pitch: f32,
    pub orbit_distance: f32,
    pub top_center: Vec2,
    pub top_height: f32,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            mode: ViewMode::Orbit,
            overlay: Overlay::Natural,
            vertical_scale: 6.0,
            show_water: true,
            show_vegetation: true,
            show_organisms: true,
            tool: Tool::Inspect,
            brush_radius: 8.0,
            region: String::new(),
            water_amount: 2000.0,
            spawn_count: 20,
            inspect_field: "surface_water".into(),
            selected_cell: None,
            selected_entity: None,
            hover_cell: None,
            viewport: None,
            last_paint: None,
            press_pos: None,
            orbit_target: Vec3::ZERO,
            orbit_yaw: 0.6,
            orbit_pitch: 0.75,
            orbit_distance: 3200.0,
            top_center: Vec2::ZERO,
            top_height: 2600.0,
        }
    }
}

#[derive(Component)]
pub struct MainCamera;
#[derive(Component)]
pub struct TerrainMesh;
#[derive(Component)]
pub struct WaterMesh;
#[derive(Component)]
pub struct Plant(usize);
#[derive(Component)]
pub struct Organism;

#[derive(Resource, Default)]
pub struct Scene3d {
    built: Option<(usize, usize, f32)>,
    elevation: Option<Arc<Vec<f32>>>,
    terrain: Option<Handle<Mesh>>,
    water: Option<Handle<Mesh>>,
    lit: Option<Handle<StandardMaterial>>,
    unlit: Option<Handle<StandardMaterial>>,
    plant_meshes: Vec<Handle<Mesh>>,
    plant_mats: Vec<Handle<StandardMaterial>>,
    slope: Vec<f32>,
    organism_mesh: Option<Handle<Mesh>>,
    organism_mats: Vec<Handle<StandardMaterial>>,
    organisms: HashMap<u64, Entity>,
    colored_for: Option<(u64, Overlay, String)>,
    /// Frames to keep recoloring after (re)spawning instances, whose entities appear a frame later.
    recolor_frames: u8,
    plants_stride: usize,
}

/// Offscreen render target used for scripted captures (works with a locked or hidden display).
#[derive(Resource, Clone)]
pub struct Offscreen(pub Handle<Image>);

pub fn setup(
    mut commands: Commands,
    mut egui_settings: ResMut<bevy_egui::EguiGlobalSettings>,
    script: Res<crate::Script>,
    mut images: ResMut<Assets<Image>>,
) {
    // A dedicated full-window camera renders egui; the world camera renders into the central viewport.
    egui_settings.auto_create_primary_context = false;
    let target = script.screenshot.as_ref().map(|_| {
        let size = bevy::render::render_resource::Extent3d {
            width: 1680,
            height: 1000,
            depth_or_array_layers: 1,
        };
        let mut image = Image {
            data: Some(vec![0; (size.width * size.height * 4) as usize]),
            ..default()
        };
        image.texture_descriptor.usage |= bevy::render::render_resource::TextureUsages::RENDER_ATTACHMENT;
        image.texture_descriptor.size = size;
        images.add(image)
    });
    let mut ui_cam = commands.spawn((
        bevy_egui::PrimaryEguiContext,
        Camera2d,
        bevy::camera::visibility::RenderLayers::none(),
        Camera {
            order: 1,
            clear_color: ClearColorConfig::None,
            ..default()
        },
    ));
    if let Some(t) = &target {
        ui_cam.insert(bevy::camera::RenderTarget::Image(t.clone().into()));
    }
    let mut world_cam = commands.spawn((
        MainCamera,
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            far: 60000.0,
            near: 1.0,
            ..default()
        }),
        Transform::from_xyz(0.0, 3000.0, 3000.0).looking_at(Vec3::ZERO, Vec3::Y),
        bevy::pbr::DistanceFog {
            color: SKY,
            falloff: bevy::pbr::FogFalloff::Linear {
                start: 3500.0,
                end: 14000.0,
            },
            ..default()
        },
    ));
    if let Some(t) = &target {
        world_cam.insert(bevy::camera::RenderTarget::Image(t.clone().into()));
        commands.insert_resource(Offscreen(t.clone()));
    }
    commands.spawn((
        DirectionalLight {
            illuminance: 10000.0,
            color: Color::srgb(1.0, 0.96, 0.88),
            shadow_maps_enabled: true,
            ..default()
        },
        bevy::light::CascadeShadowConfigBuilder {
            num_cascades: 3,
            first_cascade_far_bound: 900.0,
            maximum_distance: 7000.0,
            ..default()
        }
        .build(),
        Transform::from_xyz(-1.0, 0.9, 0.55).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.75, 0.82, 1.0),
        brightness: 260.0,
        ..default()
    });
    commands.insert_resource(ClearColor(SKY));
}

const SKY: Color = Color::srgb(0.64, 0.76, 0.88);

fn hash01(k: u64, salt: u64) -> f32 {
    (sim_core::rng::mix(k ^ salt.wrapping_mul(0x9E37_79B9)) >> 40) as f32 / (1u64 << 24) as f32
}

fn tint(mut m: Mesh, rgb: [f32; 3]) -> Mesh {
    let n = m.count_vertices();
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[rgb[0], rgb[1], rgb[2], 1.0]; n]);
    m
}

fn merged(parts: Vec<Mesh>) -> Mesh {
    let mut it = parts.into_iter();
    let mut m = it.next().unwrap();
    for p in it {
        let _ = m.merge(&p);
    }
    m
}

/// Conifer: trunk and two stacked cones; ~12 m tall at scale 1.
fn conifer_mesh() -> Mesh {
    merged(vec![
        tint(
            Mesh::from(Cylinder::new(0.45, 3.0)).transformed_by(Transform::from_xyz(0.0, 1.5, 0.0)),
            [0.36, 0.25, 0.16],
        ),
        tint(
            Mesh::from(Cone { radius: 3.0, height: 6.5 }).transformed_by(Transform::from_xyz(0.0, 5.2, 0.0)),
            [0.16, 0.34, 0.20],
        ),
        tint(
            Mesh::from(Cone { radius: 2.1, height: 5.0 }).transformed_by(Transform::from_xyz(0.0, 8.4, 0.0)),
            [0.18, 0.38, 0.22],
        ),
    ])
}

/// Broadleaf: trunk and a rounded, slightly squashed canopy.
fn broadleaf_mesh() -> Mesh {
    merged(vec![
        tint(
            Mesh::from(Cylinder::new(0.5, 4.0)).transformed_by(Transform::from_xyz(0.0, 2.0, 0.0)),
            [0.38, 0.27, 0.17],
        ),
        tint(
            Sphere::new(3.2)
                .mesh()
                .ico(2)
                .unwrap()
                .transformed_by(Transform::from_xyz(0.0, 6.2, 0.0).with_scale(Vec3::new(1.0, 0.8, 1.0))),
            [0.30, 0.46, 0.20],
        ),
        tint(
            Sphere::new(2.0)
                .mesh()
                .ico(1)
                .unwrap()
                .transformed_by(Transform::from_xyz(1.6, 5.4, 0.8)),
            [0.26, 0.42, 0.18],
        ),
    ])
}

/// A small grazing quadruped facing +x.
fn animal_mesh() -> Mesh {
    let leg = |x: f32, z: f32| {
        tint(
            Mesh::from(Cylinder::new(0.32, 2.0)).transformed_by(Transform::from_xyz(x, -1.3, z)),
            [0.25, 0.22, 0.2],
        )
    };
    merged(vec![
        tint(
            Mesh::from(Capsule3d::new(1.35, 2.6))
                .transformed_by(Transform::from_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2))),
            [1.0, 1.0, 1.0],
        ),
        tint(
            Sphere::new(1.05)
                .mesh()
                .ico(1)
                .unwrap()
                .transformed_by(Transform::from_xyz(2.7, 0.9, 0.0)),
            [0.9, 0.9, 0.9],
        ),
        leg(1.2, 0.7),
        leg(1.2, -0.7),
        leg(-1.2, 0.7),
        leg(-1.2, -0.7),
    ])
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn height_at(s: &Snapshot, vs: f32, x: f32, z: f32) -> f32 {
    let cs = s.cell_size as f32;
    let cx = ((x / cs).floor().max(0.0) as usize).min(s.width - 1);
    let cz = ((z / cs).floor().max(0.0) as usize).min(s.height - 1);
    s.elevation[cz * s.width + cx] * vs
}

fn field<'a>(s: &'a Snapshot, name: &str) -> Option<&'a [f32]> {
    s.plan.cell_fields.iter().position(|f| f.0 == name).map(|i| &s.fields[i][..])
}

fn cell_color(s: &Snapshot, view: &ViewState, c: usize, cache: &OverlayCache) -> [f32; 4] {
    let get = |v: Option<&[f32]>| v.map(|v| v[c]).unwrap_or(0.0);
    let rgb = match view.overlay {
        Overlay::Natural | Overlay::Flow => {
            let moisture = if get(cache.cap) > 0.0 {
                (get(cache.soil) / get(cache.cap)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let veg = get(cache.veg);
            let cover = veg / (veg + 500.0);
            let e = (s.elevation[c] - cache.emin) / (cache.emax - cache.emin).max(1e-6);
            // Grass from dry straw to lush green; darker forest floor under dense cover.
            let grass = lerp3([0.52, 0.52, 0.26], [0.20, 0.42, 0.12], smooth(0.08, 0.5, moisture));
            let mut rgb = lerp3(grass, [0.12, 0.24, 0.09], cover * 0.8);
            // Bare, dry ground where vegetation is sparse.
            rgb = lerp3(
                lerp3([0.60, 0.53, 0.38], [0.38, 0.31, 0.22], moisture),
                rgb,
                smooth(0.02, 0.2, cover),
            );
            // Rock on steep slopes and high ground.
            let slope = cache.slope.get(c).copied().unwrap_or(0.0);
            rgb = lerp3(rgb, [0.52, 0.50, 0.47], smooth(0.18, 0.45, slope).max(smooth(0.85, 1.0, e) * 0.5));
            // Wet mud along water.
            let depth = get(cache.sw) / (1000.0 * (s.cell_size * s.cell_size) as f32);
            rgb = lerp3(rgb, [0.33, 0.30, 0.22], smooth(0.0005, 0.004, depth) * 0.8);
            // Per-cell variation so fields do not look flat.
            let n = 0.92 + 0.16 * hash01(c as u64, 7);
            rgb = [rgb[0] * n, rgb[1] * n, rgb[2] * n];
            if view.overlay == Overlay::Flow {
                rgb = lerp3(rgb, [0.5, 0.5, 0.5], 0.5);
                if let Some(f) = &cache.flow_mag {
                    let t = ((f[c] + 1.0).ln() / 9.0).clamp(0.0, 1.0);
                    rgb = lerp3(rgb, ramp(Ramp::Blues, 0.4 + 0.6 * t), t);
                }
            }
            rgb
        }
        Overlay::Elevation => ramp(Ramp::Terrain, (s.elevation[c] - cache.emin) / (cache.emax - cache.emin).max(1e-6)),
        Overlay::Temperature => ramp(Ramp::Diverging, (get(cache.temp) - 270.0) / 40.0),
        Overlay::SurfaceWater => {
            let depth = get(cache.sw) / (1000.0 * (s.cell_size * s.cell_size) as f32);
            if depth < 0.001 {
                [0.85, 0.83, 0.78]
            } else {
                ramp(Ramp::Blues, ((depth / 0.001).ln() / (5000.0f32).ln()).clamp(0.0, 1.0))
            }
        }
        Overlay::SoilMoisture => ramp(
            Ramp::Moisture,
            if get(cache.cap) > 0.0 {
                get(cache.soil) / get(cache.cap)
            } else {
                0.0
            },
        ),
        Overlay::Groundwater => ramp(Ramp::Blues, get(cache.gw) / 10000.0),
        Overlay::Vegetation => ramp(Ramp::Greens, get(cache.veg) / 3000.0),
        Overlay::Nutrients => ramp(Ramp::Purples, get(cache.nut) / 150.0),
        Overlay::Region => ramp(Ramp::Orange, cache.region.map(|r| r[c]).unwrap_or(0.0)),
    };
    [rgb[0], rgb[1], rgb[2], 1.0]
}

struct OverlayCache<'a> {
    soil: Option<&'a [f32]>,
    cap: Option<&'a [f32]>,
    veg: Option<&'a [f32]>,
    sw: Option<&'a [f32]>,
    gw: Option<&'a [f32]>,
    temp: Option<&'a [f32]>,
    nut: Option<&'a [f32]>,
    region: Option<&'a [f32]>,
    flow_mag: Option<Vec<f32>>,
    slope: &'a [f32],
    emin: f32,
    emax: f32,
}

fn grid_mesh(w: usize, h: usize, cs: f32, heights: impl Fn(usize) -> f32) -> Mesh {
    let mut pos = Vec::with_capacity(w * h);
    for z in 0..h {
        for x in 0..w {
            pos.push([(x as f32 + 0.5) * cs, heights(z * w + x), (z as f32 + 0.5) * cs]);
        }
    }
    let mut idx: Vec<u32> = Vec::with_capacity((w - 1) * (h - 1) * 6);
    for z in 0..h - 1 {
        for x in 0..w - 1 {
            let a = (z * w + x) as u32;
            let b = a + 1;
            let c = a + w as u32;
            let d = c + 1;
            idx.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    let n = pos.len();
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0f32, 1.0, 1.0, 1.0]; n]);
    m.insert_indices(Indices::U32(idx));
    m.compute_smooth_normals();
    m
}

#[allow(clippy::too_many_arguments)]
pub fn update_scene(
    mut commands: Commands,
    state: Res<ClientState>,
    mut view: ResMut<ViewState>,
    mut scene: ResMut<Scene3d>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut plants: Query<(&Plant, &mut Transform, &mut Visibility), (Without<Organism>, Without<TerrainMesh>, Without<WaterMesh>)>,
    mut organisms: Query<(&mut Transform, &mut Visibility), (With<Organism>, Without<Plant>)>,
    mut terrain_q: Query<
        (&mut MeshMaterial3d<StandardMaterial>, &mut Visibility),
        (With<TerrainMesh>, Without<Plant>, Without<Organism>, Without<WaterMesh>),
    >,
    mut water_q: Query<&mut Visibility, (With<WaterMesh>, Without<TerrainMesh>, Without<Plant>, Without<Organism>)>,
) {
    let Some(s) = state.snapshot.clone() else { return };
    let (w, h, cs) = (s.width, s.height, s.cell_size as f32);
    let vs = view.vertical_scale;
    let rebuild = scene.built != Some((w, h, vs)) || scene.elevation.as_ref().is_none_or(|e| !Arc::ptr_eq(e, &s.elevation));
    if rebuild {
        for e in scene.organisms.drain().map(|x| x.1) {
            commands.entity(e).despawn();
        }
        let elev = s.elevation.clone();
        let terrain = meshes.add(grid_mesh(w, h, cs, |c| elev[c] * vs));
        let water = meshes.add(grid_mesh(w, h, cs, |c| elev[c] * vs - 1.0));
        if scene.lit.is_none() {
            scene.lit = Some(materials.add(StandardMaterial {
                base_color: Color::WHITE,
                perceptual_roughness: 0.95,
                ..default()
            }));
            scene.unlit = Some(materials.add(StandardMaterial {
                base_color: Color::WHITE,
                unlit: true,
                ..default()
            }));
            scene.plant_meshes = vec![meshes.add(conifer_mesh()), meshes.add(broadleaf_mesh())];
            // Slight tint variations; vertex colors carry trunk vs. canopy.
            scene.plant_mats = [[1.0, 1.0, 1.0], [0.85, 0.95, 0.85], [1.12, 1.05, 0.85]]
                .iter()
                .map(|t| {
                    materials.add(StandardMaterial {
                        base_color: Color::srgb(t[0], t[1], t[2]),
                        perceptual_roughness: 0.92,
                        ..default()
                    })
                })
                .collect();
            scene.organism_mesh = Some(meshes.add(animal_mesh()));
            // Muted, earthy lineage colors that stay readable against grass.
            scene.organism_mats = (0..10)
                .map(|k| {
                    let hue = [20.0, 35.0, 0.0, 200.0, 280.0, 45.0, 330.0, 180.0, 15.0, 240.0][k];
                    materials.add(StandardMaterial {
                        base_color: Color::hsl(hue, 0.45, 0.62),
                        perceptual_roughness: 0.8,
                        ..default()
                    })
                })
                .collect();
            let water_mat = materials.add(StandardMaterial {
                base_color: Color::srgba(1.0, 1.0, 1.0, 1.0),
                alpha_mode: AlphaMode::Blend,
                perceptual_roughness: 0.06,
                reflectance: 0.55,
                ..default()
            });
            commands.spawn((TerrainMesh, Mesh3d(terrain.clone()), MeshMaterial3d(scene.lit.clone().unwrap())));
            commands.spawn((WaterMesh, Mesh3d(water.clone()), MeshMaterial3d(water_mat)));
        } else if let (Some(t), Some(wm)) = (&scene.terrain, &scene.water) {
            if let Some(mut m) = meshes.get_mut(t) {
                *m = grid_mesh(w, h, cs, |c| elev[c] * vs);
            }
            if let Some(mut m) = meshes.get_mut(wm) {
                *m = grid_mesh(w, h, cs, |c| elev[c] * vs - 1.0);
            }
        }
        if scene.terrain.is_none() {
            scene.terrain = Some(terrain);
            scene.water = Some(water);
        }
        // Vegetation instance pool on a coarse lattice (presentation only).
        if scene.plants_stride == 0 {
            let stride = (w / 110).max(1);
            scene.plants_stride = stride;
            let mut z = stride / 2;
            while z < h {
                let mut x = stride / 2;
                while x < w {
                    // Jitter within the lattice so vegetation does not read as a grid.
                    let hsh = sim_core::rng::mix((z * w + x) as u64);
                    let jx = ((hsh & 0xffff) as f32 / 65535.0 - 0.5) * stride as f32;
                    let jz = (((hsh >> 16) & 0xffff) as f32 / 65535.0 - 0.5) * stride as f32;
                    let (px, pz) = (
                        (x as f32 + jx).clamp(0.0, w as f32 - 1.0),
                        (z as f32 + jz).clamp(0.0, h as f32 - 1.0),
                    );
                    let c = pz as usize * w + px as usize;
                    let emin = s.elevation.iter().cloned().fold(f32::INFINITY, f32::min);
                    let emax = s.elevation.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
                    let en = (s.elevation[c] - emin) / (emax - emin).max(1e-6);
                    // Conifers favor higher ground, broadleaf trees the lowlands.
                    let variant = usize::from(hash01(hsh, 3) > 0.25 + 0.6 * en);
                    let mat = (hash01(hsh, 5) * 3.0) as usize % 3;
                    let yaw = hash01(hsh, 9) * std::f32::consts::TAU;
                    commands.spawn((
                        Plant(c),
                        Mesh3d(scene.plant_meshes[variant].clone()),
                        MeshMaterial3d(scene.plant_mats[mat].clone()),
                        Transform::from_xyz((px + 0.5) * cs, 0.0, (pz + 0.5) * cs).with_rotation(Quat::from_rotation_y(yaw)),
                        Visibility::Hidden,
                    ));
                    x += stride;
                }
                z += stride;
            }
        }
        scene.slope = (0..w * h)
            .map(|c| {
                let (x, z) = (c % w, c / w);
                let ex = s.elevation[z * w + (x + 1).min(w - 1)] - s.elevation[z * w + x.saturating_sub(1)];
                let ez = s.elevation[(z + 1).min(h - 1) * w + x] - s.elevation[z.saturating_sub(1) * w + x];
                (ex * ex + ez * ez).sqrt() / (2.0 * cs)
            })
            .collect();
        scene.built = Some((w, h, vs));
        scene.elevation = Some(s.elevation.clone());
        scene.colored_for = None;
        scene.recolor_frames = 3;
        let (ex, ez) = (w as f32 * cs, h as f32 * cs);
        if view.orbit_target == Vec3::ZERO {
            view.orbit_target = Vec3::new(ex / 2.0, 0.0, ez / 2.0);
            view.top_center = Vec2::new(ex / 2.0, ez / 2.0);
            view.top_height = ez * 1.05;
            view.orbit_distance = ex.max(ez) * 0.45;
            view.orbit_pitch = 0.55;
        }
    }

    // Terrain colors per overlay.
    let key = (s.tick, view.overlay, view.region.clone());
    if scene.recolor_frames > 0 {
        scene.recolor_frames -= 1;
        scene.colored_for = None;
    }
    if scene.colored_for.as_ref() != Some(&key) {
        scene.colored_for = Some(key);
        let region = s.plan.regions.iter().position(|r| *r == view.region).map(|i| &s.regions[i][..]);
        let flow_mag = s.flow.as_ref().map(|f| {
            let mut m = vec![0.0f32; w * h];
            let he = (w - 1) * h;
            for (e, q) in f.iter().enumerate() {
                let (a, b) = if e < he {
                    let z = e / (w - 1);
                    let x = e % (w - 1);
                    (z * w + x, z * w + x + 1)
                } else {
                    (e - he, e - he + w)
                };
                m[a] += q.abs() * 0.5;
                m[b] += q.abs() * 0.5;
            }
            m
        });
        let emin = s.elevation.iter().cloned().fold(f32::INFINITY, f32::min);
        let emax = s.elevation.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let cache = OverlayCache {
            soil: field(&s, "soil_water"),
            cap: field(&s, "soil_water_capacity"),
            veg: field(&s, "vegetation_biomass"),
            sw: field(&s, "surface_water"),
            gw: field(&s, "groundwater"),
            temp: field(&s, "temperature"),
            nut: field(&s, "soil_nutrients"),
            region,
            flow_mag,
            slope: &scene.slope,
            emin,
            emax,
        };
        let colors: Vec<[f32; 4]> = (0..w * h).map(|c| cell_color(&s, &view, c, &cache)).collect();
        if let Some(mut m) = scene.terrain.as_ref().and_then(|t| meshes.get_mut(t)) {
            m.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
        }
        // Water surface: elevation + depth where wet, hidden below terrain elsewhere.
        if let (Some(sw), Some(mut m)) = (cache.sw, scene.water.as_ref().and_then(|t| meshes.get_mut(t))) {
            let area = (s.cell_size * s.cell_size) as f32;
            let mut pos = Vec::with_capacity(w * h);
            let mut col = Vec::with_capacity(w * h);
            for z in 0..h {
                for x in 0..w {
                    let c = z * w + x;
                    let depth = sw[c] / (1000.0 * area);
                    let e = s.elevation[c] * vs;
                    let wet = depth > 0.003;
                    pos.push([
                        (x as f32 + 0.5) * cs,
                        if wet { e + depth * vs + 0.3 } else { e - 2.0 },
                        (z as f32 + 0.5) * cs,
                    ]);
                    let t = ((depth / 0.003).ln() / (1000.0f32).ln()).clamp(0.0, 1.0);
                    let rgb = lerp3([0.22, 0.48, 0.72], [0.03, 0.14, 0.38], t);
                    col.push([rgb[0], rgb[1], rgb[2], if wet { 0.72 + 0.26 * t } else { 0.0 }]);
                }
            }
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
            m.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
        }
        let natural = matches!(view.overlay, Overlay::Natural | Overlay::Flow);
        for (mut mat, _) in &mut terrain_q {
            let want = if natural {
                scene.lit.clone().unwrap()
            } else {
                scene.unlit.clone().unwrap()
            };
            if mat.0 != want {
                mat.0 = want;
            }
        }
        for mut v in &mut water_q {
            *v = if view.show_water && natural {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };
        }
        // Vegetation density as instance scale.
        let veg = cache.veg;
        let show_plants = view.show_vegetation && natural;
        for (p, mut t, mut vis) in &mut plants {
            let b = veg.map(|v| v[p.0]).unwrap_or(0.0);
            let depth = cache.sw.map(|v| v[p.0]).unwrap_or(0.0) / (1000.0 * cs * cs);
            if !show_plants || b < 250.0 || depth > 0.02 {
                *vis = Visibility::Hidden;
                continue;
            }
            *vis = Visibility::Visible;
            let jitter = 0.75 + 0.5 * hash01(p.0 as u64, 11);
            let k = (b / 1800.0).clamp(0.35, 1.5) * scene.plants_stride as f32 * 1.25 * jitter;
            t.scale = Vec3::new(k, k * (0.9 + 0.3 * hash01(p.0 as u64, 13)), k);
            t.translation.y = s.elevation[p.0] * vs - 0.3;
        }
    }

    // Organisms: one presentation entity per simulated organism.
    let mut alive: HashMap<u64, ()> = HashMap::new();
    for ents in &s.entities {
        for e in ents {
            alive.insert(e.id, ());
            let k = 0.9 + e.size * 0.5;
            let y = height_at(&s, vs, e.x, e.z) + 2.3 * k;
            let tr = Transform::from_xyz(e.x, y, e.z)
                .with_rotation(Quat::from_rotation_y(-e.heading))
                .with_scale(Vec3::splat(k));
            match scene.organisms.get(&e.id) {
                Some(&ent) => {
                    if let Ok((mut t, mut v)) = organisms.get_mut(ent) {
                        *t = tr;
                        *v = if view.show_organisms {
                            Visibility::Visible
                        } else {
                            Visibility::Hidden
                        };
                    }
                }
                None => {
                    let mat = scene.organism_mats[(sim_core::rng::mix(e.root) % scene.organism_mats.len() as u64) as usize].clone();
                    let ent = commands
                        .spawn((Organism, Mesh3d(scene.organism_mesh.clone().unwrap()), MeshMaterial3d(mat), tr))
                        .id();
                    scene.organisms.insert(e.id, ent);
                }
            }
        }
    }
    let dead: Vec<u64> = scene.organisms.keys().filter(|k| !alive.contains_key(k)).copied().collect();
    for id in dead {
        if let Some(e) = scene.organisms.remove(&id) {
            commands.entity(e).despawn();
        }
    }
}

/// Keep the sky and haze in tune with the UI theme.
pub fn sky(theme: Res<crate::theme::ActiveTheme>, mut clear: ResMut<ClearColor>, mut fog: Query<&mut bevy::pbr::DistanceFog>) {
    if !theme.is_changed() {
        return;
    }
    let c = Color::srgb(theme.0.sky[0], theme.0.sky[1], theme.0.sky[2]);
    clear.0 = c;
    for mut f in &mut fog {
        f.color = c;
    }
}

pub fn camera_control(
    mut view: ResMut<ViewState>,
    state: Res<ClientState>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut cam: Single<(&mut Transform, &mut Projection, &mut Camera, &mut bevy::pbr::DistanceFog), With<MainCamera>>,
    window: Single<&Window, With<PrimaryWindow>>,
    offscreen: Option<Res<Offscreen>>,
) {
    let offscreen = offscreen.is_some();
    let over_ui = state.pointer_over_ui;
    let (ref mut t, ref mut proj, ref mut camera, ref mut fog) = *cam;
    // Atmospheric haze scales with viewing distance so the focus area stays crisp.
    let d = if view.mode == ViewMode::Orbit {
        view.orbit_distance
    } else {
        view.top_height
    };
    fog.falloff = bevy::pbr::FogFalloff::Linear {
        start: d * 1.1,
        end: d * 4.5,
    };
    if let Some((x, y, w, h)) = view.viewport.filter(|_| !offscreen) {
        let size = UVec2::new(w.max(1), h.max(1));
        let pos = UVec2::new(x, y);
        let win = UVec2::new(window.physical_width(), window.physical_height());
        if pos.x + size.x <= win.x && pos.y + size.y <= win.y {
            camera.viewport = Some(bevy::camera::Viewport {
                physical_position: pos,
                physical_size: size,
                ..default()
            });
        }
    }
    let scroll_y = if over_ui {
        0.0
    } else {
        match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y * 0.1,
            MouseScrollUnit::Pixel => scroll.delta.y * 0.002,
        }
    };
    let drag = if over_ui { Vec2::ZERO } else { motion.delta };
    let pan_speed = |d: f32| d * 0.0012;
    match view.mode {
        ViewMode::Orbit => {
            if buttons.pressed(MouseButton::Right) {
                view.orbit_yaw -= drag.x * 0.005;
                view.orbit_pitch = (view.orbit_pitch + drag.y * 0.004).clamp(0.08, 1.5);
            }
            if buttons.pressed(MouseButton::Middle) || (buttons.pressed(MouseButton::Left) && keys.pressed(KeyCode::ShiftLeft)) {
                let d = view.orbit_distance;
                let (s, c) = view.orbit_yaw.sin_cos();
                let right = Vec3::new(c, 0.0, -s);
                let fwd = Vec3::new(s, 0.0, c);
                view.orbit_target += (-right * drag.x + fwd * drag.y) * pan_speed(d);
            }
            let mut kpan = Vec3::ZERO;
            let (s, c) = view.orbit_yaw.sin_cos();
            if keys.pressed(KeyCode::KeyW) {
                kpan -= Vec3::new(s, 0.0, c);
            }
            if keys.pressed(KeyCode::KeyS) {
                kpan += Vec3::new(s, 0.0, c);
            }
            if keys.pressed(KeyCode::KeyA) {
                kpan -= Vec3::new(c, 0.0, -s);
            }
            if keys.pressed(KeyCode::KeyD) {
                kpan += Vec3::new(c, 0.0, -s);
            }
            if !state.keyboard_captured {
                let d = view.orbit_distance;
                view.orbit_target += kpan * d * 0.01;
            }
            view.orbit_distance = (view.orbit_distance * (1.0 - scroll_y)).clamp(60.0, 20000.0);
            let dir = Vec3::new(
                view.orbit_yaw.sin() * view.orbit_pitch.cos(),
                view.orbit_pitch.sin(),
                view.orbit_yaw.cos() * view.orbit_pitch.cos(),
            );
            **t = Transform::from_translation(view.orbit_target + dir * view.orbit_distance).looking_at(view.orbit_target, Vec3::Y);
            if !matches!(**proj, Projection::Perspective(_)) {
                **proj = Projection::Perspective(PerspectiveProjection {
                    far: 60000.0,
                    near: 1.0,
                    ..default()
                });
            }
        }
        ViewMode::TopDown => {
            if buttons.pressed(MouseButton::Right) || buttons.pressed(MouseButton::Middle) {
                let k = view.top_height / window.height().max(1.0);
                view.top_center -= Vec2::new(drag.x, drag.y) * k;
            }
            view.top_height = (view.top_height * (1.0 - scroll_y)).clamp(50.0, 30000.0);
            **t = Transform::from_xyz(view.top_center.x, 5000.0, view.top_center.y)
                .looking_at(Vec3::new(view.top_center.x, 0.0, view.top_center.y), Vec3::NEG_Z);
            **proj = Projection::from(OrthographicProjection {
                scaling_mode: ScalingMode::FixedVertical {
                    viewport_height: view.top_height,
                },
                far: 20000.0,
                ..OrthographicProjection::default_3d()
            });
        }
    }
}

/// Ray-march the height field to find the cell under the cursor.
fn pick(s: &Snapshot, vs: f32, ray: Ray3d) -> Option<usize> {
    let cs = s.cell_size as f32;
    let (ex, ez) = (s.width as f32 * cs, s.height as f32 * cs);
    let step = cs * 0.5;
    let mut t = 0.0;
    let dir = *ray.direction;
    for _ in 0..200_000 {
        let p = ray.origin + dir * t;
        if p.y < -100.0 {
            break;
        }
        if p.x >= 0.0 && p.z >= 0.0 && p.x < ex && p.z < ez && p.y <= height_at(s, vs, p.x, p.z) {
            let cx = (p.x / cs) as usize;
            let cz = (p.z / cs) as usize;
            return Some(cz * s.width + cx);
        }
        t += step;
        if t > 80_000.0 {
            break;
        }
    }
    None
}

pub fn pointer_tools(
    mut view: ResMut<ViewState>,
    state: Res<ClientState>,
    link: Res<SimLink>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    cam: Single<(&Camera, &GlobalTransform), With<MainCamera>>,
    window: Single<&Window, With<PrimaryWindow>>,
) {
    let Some(s) = state.snapshot.clone() else { return };
    view.hover_cell = None;
    if state.pointer_over_ui {
        return;
    }
    let Some(cursor) = window.cursor_position() else { return };
    let (camera, gt) = *cam;
    let Ok(ray) = camera.viewport_to_world(gt, cursor) else { return };
    let Some(cell) = pick(&s, view.vertical_scale, ray) else { return };
    view.hover_cell = Some(cell);
    if keys.pressed(KeyCode::ShiftLeft) {
        return;
    }
    let (cx, cz) = ((cell % s.width) as f64 + 0.5, (cell / s.width) as f64 + 0.5);
    let brush = RegionShape::Circle {
        cx,
        cz,
        radius: view.brush_radius as f64,
        feather: (view.brush_radius * 0.3) as f64,
    };
    let now = time.elapsed_secs_f64();
    let throttle = |view: &mut ViewState| -> bool {
        let ok = view.last_paint.is_none_or(|(c, t)| c != cell || now - t > 0.25);
        if ok {
            view.last_paint = Some((cell, now));
        }
        ok
    };
    match view.tool {
        Tool::Inspect => {
            // Select on a clean click: pressed and released without dragging the camera.
            if buttons.just_pressed(MouseButton::Left) {
                view.press_pos = Some(cursor);
            }
            let clean = buttons.just_released(MouseButton::Left) && view.press_pos.take().is_some_and(|p0| p0.distance(cursor) < 6.0);
            if clean {
                // Prefer a nearby organism, otherwise the cell.
                let (px, pz) = (cx as f32 * s.cell_size as f32, cz as f32 * s.cell_size as f32);
                let mut best: Option<(f32, usize, u64)> = None;
                for (a, ents) in s.entities.iter().enumerate() {
                    for e in ents {
                        let d = (e.x - px).hypot(e.z - pz);
                        if d < s.cell_size as f32 * 1.5 && best.is_none_or(|b| d < b.0) {
                            best = Some((d, a, e.id));
                        }
                    }
                }
                view.selected_cell = Some(cell);
                let _ = link.0.tx.send(ToSim::SelectCell(Some(cell), view.inspect_field.clone()));
                if let Some((_, a, id)) = best {
                    view.selected_entity = Some(id);
                    let _ = link.0.tx.send(ToSim::SelectEntity(Some((a, id))));
                }
            }
        }
        Tool::AddWater => {
            if buttons.pressed(MouseButton::Left) && throttle(&mut view) {
                let _ = link.0.tx.send(ToSim::Submit(CommandKind::AddResource {
                    field: "surface_water".into(),
                    shape: brush,
                    amount: view.water_amount,
                }));
            }
        }
        Tool::SpawnOrganisms => {
            if buttons.just_pressed(MouseButton::Left)
                && let Some(a) = s.plan.archetypes.first()
            {
                let _ = link.0.tx.send(ToSim::Submit(CommandKind::SpawnOrganisms {
                    archetype: a.0.clone(),
                    shape: brush,
                    count: view.spawn_count,
                }));
            }
        }
    }
}

pub fn gizmos(view: Res<ViewState>, state: Res<ClientState>, mut g: Gizmos) {
    let Some(s) = state.snapshot.clone() else { return };
    let cs = s.cell_size as f32;
    let vs = view.vertical_scale;
    let cell_box = |g: &mut Gizmos, c: usize, color: Color, r: f32| {
        let (x, z) = ((c % s.width) as f32 + 0.5, (c / s.width) as f32 + 0.5);
        let y = s.elevation[c] * vs + 2.0;
        g.circle(
            Isometry3d::new(Vec3::new(x * cs, y, z * cs), Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
            r,
            color,
        );
    };
    if let Some(c) = view.hover_cell {
        let r = if matches!(view.tool, Tool::Inspect) {
            cs * 0.8
        } else {
            view.brush_radius * cs
        };
        cell_box(&mut g, c, Color::srgb(1.0, 1.0, 1.0), r);
    }
    if let Some(c) = view.selected_cell {
        cell_box(&mut g, c, Color::srgb(1.0, 0.85, 0.1), cs * 0.9);
    }
    if let Some(id) = view.selected_entity {
        for ents in &s.entities {
            if let Some(e) = ents.iter().find(|e| e.id == id) {
                let y = height_at(&s, vs, e.x, e.z) + 6.0;
                g.circle(
                    Isometry3d::new(Vec3::new(e.x, y, e.z), Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                    12.0,
                    Color::srgb(1.0, 0.2, 0.9),
                );
                let (sn, c) = e.heading.sin_cos();
                g.arrow(
                    Vec3::new(e.x, y, e.z),
                    Vec3::new(e.x + c * 30.0, y, e.z + sn * 30.0),
                    Color::srgb(1.0, 0.2, 0.9),
                );
            }
        }
    }
    if view.overlay == Overlay::Flow
        && let Some(f) = &s.flow
    {
        let (w, h) = (s.width, s.height);
        let stride = (w / 64).max(1);
        let he = (w - 1) * h;
        for z in (stride / 2..h).step_by(stride) {
            for x in (stride / 2..w).step_by(stride) {
                let c = z * w + x;
                let mut v = Vec2::ZERO;
                if x + 1 < w {
                    v.x += f[z * (w - 1) + x];
                }
                if x > 0 {
                    v.x += f[z * (w - 1) + x - 1];
                }
                if z + 1 < h {
                    v.y += f[he + z * w + x];
                }
                if z > 0 {
                    v.y += f[he + (z - 1) * w + x];
                }
                let m = v.length();
                if m < 1.0 {
                    continue;
                }
                let len = ((m.ln() + 1.0) * 6.0).min(stride as f32 * cs * 0.9);
                let d = v / m * len;
                let y = s.elevation[c] * vs + 4.0;
                let p = Vec3::new((x as f32 + 0.5) * cs, y, (z as f32 + 0.5) * cs);
                g.arrow(p, p + Vec3::new(d.x, 0.0, d.y), Color::srgb(0.1, 0.3, 0.9));
            }
        }
    }
}
