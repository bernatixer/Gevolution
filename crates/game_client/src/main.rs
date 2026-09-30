#![allow(clippy::type_complexity, clippy::too_many_arguments)]
//! Evolving Worlds desktop client: Bevy presentation over the headless sim_core runtime.

mod charts;
mod editor;
mod sim;
mod theme;
mod ui;
mod view;

use bevy::prelude::*;
use bevy::window::{PresentMode, WindowResolution};
use bevy_egui::{EguiPlugin, EguiPrimaryContextPass};
use sim::{SimHandle, Snapshot};
use std::sync::Arc;

#[derive(Resource)]
pub struct SimLink(pub SimHandle);

#[derive(Resource, Default)]
pub struct ClientState {
    pub snapshot: Option<Arc<Snapshot>>,
    pub pointer_over_ui: bool,
    pub keyboard_captured: bool,
    pub messages: Vec<String>,
    pub assets: std::path::PathBuf,
}

fn pull_snapshot(link: Res<SimLink>, mut state: ResMut<ClientState>) {
    let snap = link.0.latest.lock().unwrap().take();
    if let Some(s) = snap {
        if let Some(m) = &s.message {
            state.messages.push(m.clone());
            if state.messages.len() > 50 {
                state.messages.remove(0);
            }
        }
        state.snapshot = Some(Arc::new(s));
    }
}

/// Scripted capture for automated visual checks: `--screenshot out.png [--after secs] [--speed max] [--view top] [--overlay name] [--tab name]`.
#[derive(Resource, Default, Clone)]
pub struct Script {
    pub screenshot: Option<String>,
    pub after: f32,
    pub speed: Option<String>,
    pub view: Option<String>,
    pub overlay: Option<String>,
    pub tab: Option<String>,
    pub editor_rule: Option<String>,
    pub select_cell: Option<usize>,
    done: bool,
    started: bool,
    frames: Vec<f32>,
}

fn run_script(
    mut commands: Commands,
    mut script: ResMut<Script>,
    time: Res<Time>,
    link: Res<SimLink>,
    state: Res<ClientState>,
    mut view: ResMut<view::ViewState>,
    mut uis: ResMut<ui::UiState>,
    mut editor: ResMut<editor::EditorState>,
    mut exit: MessageWriter<AppExit>,
    offscreen: Option<Res<view::Offscreen>>,
) {
    if script.screenshot.is_none() {
        return;
    }
    let Some(snap) = state.snapshot.clone() else { return };
    if !script.started {
        script.started = true;
        let speed = match script.speed.as_deref() {
            Some("max") => sim::Speed::Max,
            Some("10") => sim::Speed::X10,
            Some("1") => sim::Speed::X1,
            _ => sim::Speed::Paused,
        };
        let _ = link.0.tx.send(sim::ToSim::Speed(speed));
        if script.view.as_deref() == Some("top") {
            view.mode = view::ViewMode::TopDown;
        }
        if let Some(o) = &script.overlay
            && let Some(ov) = view::Overlay::ALL
                .iter()
                .find(|x| x.label().to_lowercase().starts_with(&o.to_lowercase()))
        {
            view.overlay = *ov;
            let _ = link.0.tx.send(sim::ToSim::WantFlow(*ov == view::Overlay::Flow));
        }
        uis.tab = if script.tab.as_deref() == Some("laws") {
            ui::Tab::Laws
        } else {
            ui::Tab::Overview
        };
        if let Some(c) = script.select_cell {
            view.selected_cell = Some(c);
            let _ = link.0.tx.send(sim::ToSim::SelectCell(Some(c), view.inspect_field.clone()));
            if let Some(e) = snap.entities.first().and_then(|e| e.first()) {
                view.selected_entity = Some(e.id);
                let _ = link.0.tx.send(sim::ToSim::SelectEntity(Some((0, e.id))));
            }
        }
        if let Some(r) = &script.editor_rule {
            editor.open(&snap);
            editor.select_rule(r);
        }
    }
    let t = time.elapsed_secs();
    if t > 2.0 && !script.done {
        script.frames.push(time.delta_secs() * 1000.0);
    }
    if !script.done && t > script.after {
        let mut f = script.frames.clone();
        f.sort_by(|a, b| a.partial_cmp(b).unwrap());
        if !f.is_empty() {
            println!(
                "frame time ms: median {:.2}, p95 {:.2} over {} frames (population {}, {:.1} sim ticks/s)",
                f[f.len() / 2],
                f[(f.len() - 1) * 95 / 100],
                f.len(),
                snap.population,
                snap.ticks_per_second
            );
        }
        script.done = true;
        let path = script.screenshot.clone().unwrap();
        let shot = match &offscreen {
            Some(o) => bevy::render::view::screenshot::Screenshot::image(o.0.clone()),
            None => bevy::render::view::screenshot::Screenshot::primary_window(),
        };
        commands.spawn(shot).observe(bevy::render::view::screenshot::save_to_disk(path));
    }
    if script.done && t > script.after + 1.5 {
        exit.write(AppExit::Success);
    }
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut script = Script { after: 4.0, ..default() };
    let mut args = vec![];
    let mut i = 0;
    while i < raw.len() {
        let next = |i: &mut usize| -> String {
            *i += 1;
            raw.get(*i).cloned().unwrap_or_default()
        };
        match raw[i].as_str() {
            "--screenshot" => script.screenshot = Some(next(&mut i)),
            "--after" => script.after = next(&mut i).parse().unwrap_or(4.0),
            "--speed" => script.speed = Some(next(&mut i)),
            "--view" => script.view = Some(next(&mut i)),
            "--overlay" => script.overlay = Some(next(&mut i)),
            "--tab" => script.tab = Some(next(&mut i)),
            "--editor" => script.editor_rule = Some(next(&mut i)),
            "--select" => script.select_cell = next(&mut i).parse().ok(),
            a => args.push(a.to_string()),
        }
        i += 1;
    }
    let assets = sim_core::assets::asset_root();
    let scenario_path = args
        .first()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| assets.join("scenarios/seasonal_river.json"));
    let (scenario, packages) = match sim_core::assets::load_scenario(&scenario_path) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let handle = match sim::spawn(scenario, packages) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("world creation failed:\n{e}");
            std::process::exit(2);
        }
    };
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Gevolution".into(),
                resolution: WindowResolution::new(1680, 1000),
                present_mode: PresentMode::AutoVsync,
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .insert_resource(SimLink(handle))
        .insert_resource(script)
        .insert_resource(ClientState { assets, ..default() })
        .init_resource::<view::ViewState>()
        .init_resource::<view::Scene3d>()
        .init_resource::<editor::EditorState>()
        .init_resource::<ui::UiState>()
        .init_resource::<theme::ActiveTheme>()
        .add_systems(Startup, view::setup)
        .add_systems(PreUpdate, pull_snapshot)
        .add_systems(
            Update,
            (
                view::update_scene,
                view::camera_control,
                view::pointer_tools,
                view::gizmos,
                view::sky,
            )
                .chain(),
        )
        .add_systems(EguiPrimaryContextPass, ui::ui_system)
        .add_systems(Update, run_script)
        .run();
}
