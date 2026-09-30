//! Presentation core: terrain (texture-blended from simulation state), water,
//! cameras, picking, and selection gizmos. Nothing here feeds back into the
//! simulation except explicit, validated commands.

use crate::materials::{self, TerrainExt, TerrainMaterial, WaterExt, WaterMaterial};
use crate::sim::{Snapshot, ToSim};
use crate::{ClientState, SimLink};
use bevy::asset::RenderAssetUsages;
use bevy::camera::ScalingMode;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::ExtendedMaterial;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use sim_core::commands::CommandKind;
use sim_core::schema::RegionShape;
use std::sync::Arc;

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
    pub vertical_scale: f32,
    pub tool: Tool,
    pub brush_radius: f32,
    pub water_amount: f64,
    pub spawn_count: usize,
    pub inspect_field: String,
    pub selected_cell: Option<usize>,
    pub selected_entity: Option<u64>,
    pub hover_cell: Option<usize>,
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
            vertical_scale: 6.0,
            tool: Tool::Inspect,
            brush_radius: 8.0,
            water_amount: 2000.0,
            spawn_count: 20,
            inspect_field: "surface_water".into(),
            selected_cell: None,
            selected_entity: None,
            hover_cell: None,
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

#[derive(Resource, Default)]
pub struct Scene3d {
    built: Option<(usize, usize, f32)>,
    elevation: Option<Arc<Vec<f32>>>,
    terrain: Option<Handle<Mesh>>,
    water: Option<Handle<Mesh>>,
    terrain_mat: Option<Handle<TerrainMaterial>>,
    water_mat: Option<Handle<WaterMaterial>>,
    pub slope: Vec<f32>,
    colored_for: Option<u64>,
}

/// Offscreen render target used for scripted captures (works with a locked or hidden display).
#[derive(Resource, Clone)]
pub struct Offscreen(pub Handle<Image>);

pub fn setup(
    mut commands: Commands,
    mut egui_settings: ResMut<bevy_egui::EguiGlobalSettings>,
    script: Res<crate::Script>,
    mut images: ResMut<Assets<Image>>,
    theme: Res<crate::theme::ActiveTheme>,
) {
    // A dedicated camera renders egui on top of the full-window world view.
    egui_settings.auto_create_primary_context = false;
    let target = script.screenshot.as_ref().map(|_| {
        let size = bevy::render::render_resource::Extent3d { width: 1680, height: 1000, depth_or_array_layers: 1 };
        let mut image = Image { data: Some(vec![0; (size.width * size.height * 4) as usize]), ..default() };
        image.texture_descriptor.usage |= bevy::render::render_resource::TextureUsages::RENDER_ATTACHMENT;
        image.texture_descriptor.size = size;
        images.add(image)
    });
    let mut ui_cam = commands.spawn((
        bevy_egui::PrimaryEguiContext,
        Camera2d,
        bevy::camera::visibility::RenderLayers::none(),
        Camera { order: 1, clear_color: ClearColorConfig::None, ..default() },
    ));
    if let Some(t) = &target {
        ui_cam.insert(bevy::camera::RenderTarget::Image(t.clone().into()));
    }
    let sky = Color::srgb(theme.0.sky[0], theme.0.sky[1], theme.0.sky[2]);
    let mut world_cam = commands.spawn((
        MainCamera,
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection { far: 60000.0, near: 0.5, ..default() }),
        Transform::from_xyz(0.0, 3000.0, 3000.0).looking_at(Vec3::ZERO, Vec3::Y),
        bevy::pbr::DistanceFog { color: sky, falloff: bevy::pbr::FogFalloff::Linear { start: 3500.0, end: 14000.0 }, ..default() },
    ));
    if let Some(t) = &target {
        world_cam.insert(bevy::camera::RenderTarget::Image(t.clone().into()));
        commands.insert_resource(Offscreen(t.clone()));
    }
    commands.spawn((
        DirectionalLight { illuminance: 11000.0, color: Color::srgb(1.0, 0.95, 0.86), shadow_maps_enabled: true, ..default() },
        bevy::light::CascadeShadowConfigBuilder { num_cascades: 3, first_cascade_far_bound: 250.0, maximum_distance: 5000.0, ..default() }.build(),
        Transform::from_xyz(-1.0, 0.85, 0.5).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.insert_resource(GlobalAmbientLight { color: Color::srgb(0.78, 0.85, 1.0), brightness: 220.0, ..default() });
    commands.insert_resource(ClearColor(sky));
}

pub fn height_at(s: &Snapshot, vs: f32, x: f32, z: f32) -> f32 {
    let cs = s.cell_size as f32;
    let cx = ((x / cs).floor().max(0.0) as usize).min(s.width - 1);
    let cz = ((z / cs).floor().max(0.0) as usize).min(s.height - 1);
    s.elevation[cz * s.width + cx] * vs
}

pub fn field<'a>(s: &'a Snapshot, name: &str) -> Option<&'a [f32]> {
    s.plan.cell_fields.iter().position(|f| f.0 == name).map(|i| &s.fields[i][..])
}

pub fn hash01(k: u64, salt: u64) -> f32 {
    (sim_core::rng::mix(k ^ salt.wrapping_mul(0x9E37_79B9)) >> 40) as f32 / (1u64 << 24) as f32
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn grid_mesh(w: usize, h: usize, cs: f32, heights: impl Fn(usize) -> f32) -> Mesh {
    let mut pos = Vec::with_capacity(w * h);
    let mut uv = Vec::with_capacity(w * h);
    for z in 0..h {
        for x in 0..w {
            pos.push([(x as f32 + 0.5) * cs, heights(z * w + x), (z as f32 + 0.5) * cs]);
            uv.push([x as f32 / w as f32, z as f32 / h as f32]);
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
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD);
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.0f32, 0.0]; n]);
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0f32, 0.0, 0.0, 0.0]; n]);
    m.insert_indices(Indices::U32(idx));
    m.compute_smooth_normals();
    m
}

#[allow(clippy::too_many_arguments)]
pub fn update_terrain(
    mut commands: Commands,
    state: Res<ClientState>,
    mut view: ResMut<ViewState>,
    mut scene: ResMut<Scene3d>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut terrain_mats: ResMut<Assets<TerrainMaterial>>,
    mut water_mats: ResMut<Assets<WaterMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(s) = state.snapshot.clone() else { return };
    let (w, h, cs) = (s.width, s.height, s.cell_size as f32);
    let vs = view.vertical_scale;
    let rebuild = scene.built != Some((w, h, vs)) || scene.elevation.as_ref().is_none_or(|e| !Arc::ptr_eq(e, &s.elevation));
    if rebuild {
        let elev = s.elevation.clone();
        if scene.terrain_mat.is_none() {
            let layers = images.add(materials::build_layer_array(&state.assets));
            scene.terrain_mat = Some(terrain_mats.add(ExtendedMaterial {
                base: StandardMaterial { base_color: Color::WHITE, perceptual_roughness: 0.92, ..default() },
                extension: TerrainExt { layers },
            }));
            scene.water_mat = Some(water_mats.add(ExtendedMaterial {
                base: StandardMaterial { base_color: Color::WHITE, alpha_mode: AlphaMode::Blend, perceptual_roughness: 0.05, reflectance: 0.6, ..default() },
                extension: WaterExt { strength: 0.12 },
            }));
            let terrain = meshes.add(grid_mesh(w, h, cs, |c| elev[c] * vs));
            let water = meshes.add(grid_mesh(w, h, cs, |c| elev[c] * vs - 1.0));
            commands.spawn((TerrainMesh, Mesh3d(terrain.clone()), MeshMaterial3d(scene.terrain_mat.clone().unwrap())));
            commands.spawn((WaterMesh, Mesh3d(water.clone()), MeshMaterial3d(scene.water_mat.clone().unwrap()), bevy::light::NotShadowCaster));
            scene.terrain = Some(terrain);
            scene.water = Some(water);
        } else {
            if let Some(mut m) = scene.terrain.as_ref().and_then(|t| meshes.get_mut(t)) {
                *m = grid_mesh(w, h, cs, |c| elev[c] * vs);
            }
            if let Some(mut m) = scene.water.as_ref().and_then(|t| meshes.get_mut(t)) {
                *m = grid_mesh(w, h, cs, |c| elev[c] * vs - 1.0);
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
        let (ex, ez) = (w as f32 * cs, h as f32 * cs);
        if view.orbit_target == Vec3::ZERO {
            view.orbit_target = Vec3::new(ex / 2.0, s.elevation[(h / 2) * w + w / 2] * vs, ez * 0.55);
            view.top_center = Vec2::new(ex / 2.0, ez / 2.0);
            view.top_height = ez * 1.05;
            view.orbit_distance = ex.max(ez) * 0.32;
            view.orbit_pitch = 0.5;
        }
    }
    if scene.colored_for == Some(s.tick) {
        return;
    }
    scene.colored_for = Some(s.tick);
    // Texture weights from the living state of each spot.
    let get = |f: Option<&[f32]>, c: usize| f.map(|v| v[c]).unwrap_or(0.0);
    let (soil, cap, veg, sw) = (field(&s, "soil_water"), field(&s, "soil_water_capacity"), field(&s, "vegetation_biomass"), field(&s, "surface_water"));
    let emin = s.elevation.iter().cloned().fold(f32::INFINITY, f32::min);
    let emax = s.elevation.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let area = cs * cs;
    let mut w0 = Vec::with_capacity(w * h);
    let mut w1 = Vec::with_capacity(w * h);
    for c in 0..w * h {
        let m = if get(cap, c) > 0.0 { (get(soil, c) / get(cap, c)).clamp(0.0, 1.0) } else { 0.0 };
        let b = get(veg, c);
        let cover = b / (b + 500.0);
        let depth = get(sw, c) / (1000.0 * area);
        let e = (s.elevation[c] - emin) / (emax - emin).max(1e-6);
        let slope = scene.slope.get(c).copied().unwrap_or(0.0);
        let rock = smooth(0.16, 0.42, slope).max(smooth(0.86, 1.0, e) * 0.7);
        let wet = smooth(0.0008, 0.01, depth);
        let lush = smooth(0.18, 0.55, m) * smooth(0.04, 0.3, cover);
        let straw = (1.0 - smooth(0.18, 0.5, m)) * smooth(0.04, 0.3, cover);
        let bare = 1.0 - smooth(0.04, 0.3, cover);
        let forest = smooth(0.55, 0.85, cover) * 0.8;
        let k = 1.0 - rock;
        w0.push([lush * k * (1.0 - wet), straw * k * (1.0 - wet), bare * k * (1.0 - wet) + 0.02, rock]);
        w1.push([wet * k, forest * k * (1.0 - wet)]);
    }
    if let Some(mut mesh) = scene.terrain.as_ref().and_then(|t| meshes.get_mut(t)) {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, w0);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, w1);
    }
    if let (Some(sw), Some(mut mesh)) = (sw, scene.water.as_ref().and_then(|t| meshes.get_mut(t))) {
        let mut pos = Vec::with_capacity(w * h);
        let mut col = Vec::with_capacity(w * h);
        for z in 0..h {
            for x in 0..w {
                let c = z * w + x;
                let depth = sw[c] / (1000.0 * area);
                let e = s.elevation[c] * vs;
                let wet = depth > 0.004;
                pos.push([(x as f32 + 0.5) * cs, if wet { e + depth * vs + 0.25 } else { e - 2.0 }, (z as f32 + 0.5) * cs]);
                let t = ((depth / 0.004).ln() / (800.0f32).ln()).clamp(0.0, 1.0);
                let shallow = [0.30, 0.62, 0.66];
                let deep = [0.05, 0.22, 0.40];
                let rgb = [shallow[0] + (deep[0] - shallow[0]) * t, shallow[1] + (deep[1] - shallow[1]) * t, shallow[2] + (deep[2] - shallow[2]) * t];
                col.push([rgb[0], rgb[1], rgb[2], if wet { 0.55 + 0.42 * t } else { 0.0 }]);
            }
        }
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
    }
}

pub fn camera_control(
    mut view: ResMut<ViewState>,
    state: Res<ClientState>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut cam: Single<(&mut Transform, &mut Projection, &mut bevy::pbr::DistanceFog), With<MainCamera>>,
    window: Single<&Window, With<PrimaryWindow>>,
) {
    let over_ui = state.pointer_over_ui;
    let (ref mut t, ref mut proj, ref mut fog) = *cam;
    // Atmospheric haze scales with viewing distance so the focus area stays crisp.
    // The map is a flat plan, so it stays clear.
    let (start, end) = if view.mode == ViewMode::Orbit { (view.orbit_distance * 1.2, view.orbit_distance * 5.0) } else { (1e8, 2e8) };
    fog.falloff = bevy::pbr::FogFalloff::Linear { start, end };
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
                view.orbit_pitch = (view.orbit_pitch + drag.y * 0.004).clamp(0.06, 1.5);
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
            view.orbit_distance = (view.orbit_distance * (1.0 - scroll_y)).clamp(25.0, 20000.0);
            let dir = Vec3::new(view.orbit_yaw.sin() * view.orbit_pitch.cos(), view.orbit_pitch.sin(), view.orbit_yaw.cos() * view.orbit_pitch.cos());
            **t = Transform::from_translation(view.orbit_target + dir * view.orbit_distance).looking_at(view.orbit_target, Vec3::Y);
            if !matches!(**proj, Projection::Perspective(_)) {
                **proj = Projection::Perspective(PerspectiveProjection { far: 60000.0, near: 0.5, ..default() });
            }
        }
        ViewMode::TopDown => {
            if buttons.pressed(MouseButton::Right) || buttons.pressed(MouseButton::Middle) {
                let k = view.top_height / window.height().max(1.0);
                view.top_center -= Vec2::new(drag.x, drag.y) * k;
            }
            view.top_height = (view.top_height * (1.0 - scroll_y)).clamp(50.0, 30000.0);
            **t = Transform::from_xyz(view.top_center.x, 5000.0, view.top_center.y).looking_at(Vec3::new(view.top_center.x, 0.0, view.top_center.y), Vec3::NEG_Z);
            **proj = Projection::from(OrthographicProjection {
                scaling_mode: ScalingMode::FixedVertical { viewport_height: view.top_height },
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
            return Some((p.z / cs) as usize * s.width + (p.x / cs) as usize);
        }
        t += step;
        if t > 80_000.0 {
            break;
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
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
    let brush = RegionShape::Circle { cx, cz, radius: view.brush_radius as f64, feather: (view.brush_radius * 0.3) as f64 };
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
                view.selected_entity = best.map(|b| b.2);
                let _ = link.0.tx.send(ToSim::SelectEntity(best.map(|(_, a, id)| (a, id))));
            }
        }
        Tool::AddWater => {
            if buttons.pressed(MouseButton::Left) && throttle(&mut view) {
                let _ = link.0.tx.send(ToSim::Submit(CommandKind::AddResource { field: "surface_water".into(), shape: brush, amount: view.water_amount }));
            }
        }
        Tool::SpawnOrganisms => {
            if buttons.just_pressed(MouseButton::Left)
                && let Some(a) = s.plan.archetypes.first()
            {
                let _ = link.0.tx.send(ToSim::Submit(CommandKind::SpawnOrganisms { archetype: a.0.clone(), shape: brush, count: view.spawn_count }));
            }
        }
    }
}

pub fn gizmos(view: Res<ViewState>, state: Res<ClientState>, mut g: Gizmos) {
    let Some(s) = state.snapshot.clone() else { return };
    let cs = s.cell_size as f32;
    let vs = view.vertical_scale;
    let flat = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
    let ring = |g: &mut Gizmos, c: usize, color: Color, r: f32| {
        let (x, z) = ((c % s.width) as f32 + 0.5, (c / s.width) as f32 + 0.5);
        let y = s.elevation[c] * vs + 1.5;
        g.circle(Isometry3d::new(Vec3::new(x * cs, y, z * cs), flat), r, color);
    };
    if let Some(c) = view.hover_cell {
        let r = if matches!(view.tool, Tool::Inspect) { cs * 0.8 } else { view.brush_radius * cs };
        ring(&mut g, c, Color::srgba(1.0, 1.0, 1.0, 0.8), r);
    }
    if let Some(c) = view.selected_cell {
        ring(&mut g, c, Color::srgb(1.0, 0.85, 0.2), cs * 0.9);
    }
    if let Some(id) = view.selected_entity {
        for ents in &s.entities {
            if let Some(e) = ents.iter().find(|e| e.id == id) {
                let y = height_at(&s, vs, e.x, e.z) + 0.5;
                g.circle(Isometry3d::new(Vec3::new(e.x, y, e.z), flat), 4.0, Color::srgb(1.0, 0.6, 0.2));
            }
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
