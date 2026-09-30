//! M3 gates: a living landscape driven by laws, not biome switches.

use serde_json::json;
use sim_core::commands::CommandKind;
use sim_core::schema::InitSpec;
use sim_core::state::compensated_sum;
use sim_core::{RunConfig, World};

fn landscape(size: usize, edit: impl FnOnce(&mut sim_core::schema::Scenario)) -> World {
    let root = sim_core::assets::asset_root();
    let (mut sc, pkgs) = sim_core::assets::load_scenario(&root.join("scenarios/seasonal_river.json")).unwrap();
    sc.grid.width = size;
    sc.grid.height = size;
    sc.regions.clear();
    sc.populations.clear();
    edit(&mut sc);
    World::new(
        sc,
        pkgs,
        RunConfig {
            sample_interval: 0,
            ..RunConfig::default()
        },
    )
    .unwrap()
}

fn total(w: &World, f: &str) -> f64 {
    compensated_sum(&w.state.cells[w.schema().cell_field(f).unwrap()])
}

#[test]
fn m3_drought_and_rain_change_water_and_vegetation() {
    let cfg = || RunConfig {
        sample_interval: 0,
        ..RunConfig::default()
    };
    let branch = |w: &World| sim_core::persistence::load(&sim_core::persistence::save(w), cfg()).unwrap();
    let rain = |w: &mut World, v: f64| {
        w.submit(
            CommandKind::SetParam {
                name: "core.env.rain:rain_factor".into(),
                value: v,
            },
            None,
        );
    };
    let mut control = landscape(48, |_| {});
    control.run(400).unwrap();
    let mut drought = branch(&control);
    rain(&mut drought, 0.0);
    control.run(8000).unwrap();
    drought.run(8000).unwrap();
    let s = |w: &World| (total(w, "soil_water"), total(w, "vegetation_biomass"));
    let ((sc, vc), (sd, vd)) = (s(&control), s(&drought));
    println!("control soil {sc:.3e} veg {vc:.3e}; drought soil {sd:.3e} veg {vd:.3e}");
    assert!(sd < sc * 0.8, "drought dries the soil");
    assert!(vd < vc, "drought reduces vegetation");
    // From the drought, one branch restores rain and one keeps the drought.
    let mut still_dry = branch(&drought);
    rain(&mut drought, 1.0);
    drought.run(8000).unwrap();
    still_dry.run(8000).unwrap();
    let ((sr, vr), (sk, vk)) = (s(&drought), s(&still_dry));
    println!("restored soil {sr:.3e} veg {vr:.3e}; continued drought soil {sk:.3e} veg {vk:.3e}");
    assert!(sr > sd && sr > sk * 1.5, "rain restores soil water");
    assert!(vr > vk, "vegetation recovers where rain returned");
}

#[test]
fn m3_plants_do_not_grow_from_missing_inputs() {
    for missing in ["soil_nutrients", "soil_water"] {
        let mut w = landscape(16, |sc| {
            sc.weather.signals.get_mut("rain_flux").unwrap().mean = 0.0;
            sc.weather.signals.get_mut("rain_flux").unwrap().amplitude = 0.0;
            sc.weather.signals.get_mut("rain_flux").unwrap().noise = 0.0;
            sc.initial.insert("surface_water".into(), InitSpec::Constant { value: 0.0 });
            sc.initial.insert("groundwater".into(), InitSpec::Constant { value: 0.0 });
            sc.initial.insert("vegetation_biomass".into(), InitSpec::Constant { value: 500.0 });
            sc.initial.insert(missing.into(), InitSpec::Constant { value: 0.0 });
        });
        let e = w
            .plan()
            .effects
            .iter()
            .position(|e| e.id == "core.env.vegetation_growth/grow")
            .unwrap();
        let (sn, sw) = (
            w.schema().cell_field("soil_nutrients").unwrap(),
            w.schema().cell_field("soil_water").unwrap(),
        );
        let mut blocked = 0;
        for _ in 0..400 {
            w.step().unwrap();
            let snap = w.prev.as_ref().unwrap();
            let acc = &w.last.as_ref().unwrap().resolved.accepted[e];
            for c in 0..w.grid.cells() {
                // Growth is funded only by inputs present in the snapshot; recycled nutrients may fund it later.
                assert!(
                    acc[1][c] <= snap.cells[sn][c] && acc[2][c] <= snap.cells[sw][c],
                    "growth exceeded its inputs at cell {c}"
                );
                if snap.cells[sn][c] == 0.0 || snap.cells[sw][c] == 0.0 {
                    assert_eq!(acc[0][c], 0.0, "no growth without {missing}");
                    blocked += 1;
                }
            }
        }
        assert!(blocked > 0);
    }
}

#[test]
fn m3_extinct_vegetation_needs_a_propagule_source() {
    let mut w = landscape(24, |sc| {
        sc.initial.insert("vegetation_biomass".into(), InitSpec::Constant { value: 0.0 });
    });
    w.run(2000).unwrap();
    assert_eq!(total(&w, "vegetation_biomass"), 0.0, "nothing regrows from nothing");
    // A single explicit propagule spreads through funded dispersal.
    w.submit(
        CommandKind::AddResource {
            field: "vegetation_biomass".into(),
            shape: serde_json::from_value(json!({ "kind": "rect", "x0": 12, "z0": 12, "x1": 13, "z1": 13 })).unwrap(),
            amount: 200.0,
        },
        None,
    );
    w.run(2000).unwrap();
    let v = &w.state.cells[w.schema().cell_field("vegetation_biomass").unwrap()];
    let colonized = v.iter().filter(|x| **x > 0.0).count();
    assert!(colonized > 1, "vegetation spreads from the seeded cell ({colonized} cells)");
    let biomass = w.schema().resource("biomass").unwrap();
    assert!(
        w.state.intervention_in[biomass] == 200.0,
        "the propagule is recorded as an intervention"
    );
}
