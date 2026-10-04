# Agent guide: Gevolution

Read `README.md` first for what the game is, how to run it, and the crate map.
`GAME_DEVELOPMENT_SPEC.md` is the original spec: milestones M0 to M7 are all built.
This file holds what the README does not: how to work here, why things are the way they are, and what is open.

## Working rules

- Commit on `master`, one commit per coherent change. Push only when the owner asks.
- Commits are unsigned. The signing agent refuses here, so use `git -c commit.gpgsign=false commit`.
- End commit messages with a `Co-Authored-By:` line for the agent.
- Before a commit, run `cargo clippy --release -p game_client` (and `-p sim_core` if you touched it), and keep it at zero warnings.
- `rustfmt.toml` sets `max_width = 140`. Rust is pinned to 1.98.1 in `rust-toolchain.toml`.
- In a fresh shell, `cargo` may be missing from `PATH`. Run `source ~/.cargo/env` first.
- `old/` (v1) and `oldv2/` (v2) are the previous JavaScript versions. Do not edit them.

## Checks

- `cargo test --release -p sim_core` runs 49 tests (T01 to T32 plus the milestone gates). All pass.
- `cargo run --release -p sim_cli -- bench benches/manifests/ecosystem-medium.json` must stay within the targets in the README.
- `sim_core` must never depend on a renderer. Everything visual lives in `game_client`.

## Seeing the client without a screen

The owner's screen is often locked, so a normal window renders black.
Use the scripted offscreen capture, then read the PNG:

```sh
./target/release/game_client --screenshot out.png [--after SECS] [--speed 1|10|max] \
  [--view top] [--tab laws|settings] [--tool water|animals] [--select CELL_INDEX] [--editor RULE_ID]
```

- `--after` counts real seconds. `--speed` starts the simulation. Without it the world stays paused.
- `--select` takes a cell index (`z * width + x`). If an animal stands there, it is selected too.
- `--editor` takes a `rule_id` such as `core.env.rain` (see `assets/rules/core/*.json`).
- The run prints frame-time stats (median and p95). Use them to check performance after visual changes.
- Visual and UX changes are not done until a capture looks right. Look at the image, do not reason from the code.

## Client architecture (game_client)

- `sim.rs`: the simulation runs on a worker thread and publishes `Snapshot` structs. It also derives presentation data:
  - `EntityView.action` (Idle/Walk/Run/Eat/Drink) comes from speed and the accepted `/eat` and `/drink` amounts.
  - `cell_history` samples the selected spot every 4 ticks (capped at 600) for the inspector chart.
- `view.rs`: cameras, lights, fog, terrain and water meshes, picking, the pointer tools, and gizmos.
  - Terrain texture weights are computed per cell in `update_terrain`:
    - vertex `COLOR` holds (lush, straw, soil, rock);
    - `UV_1` holds (wet/mud, forest floor).
  - Fog scales with orbit distance. The Map view turns fog off.
- `materials.rs` with `assets/client/shaders/terrain.wgsl` and `water.wgsl`: `ExtendedMaterial` on `StandardMaterial`.
  - The terrain samples a 6-layer texture array, built on the CPU with a full mip chain.
  - In `terrain.wgsl`, noise patches decide where lush grass, straw and bare soil grow. The simulated state sets their balance.
- `flora.rs`: plant models on a jittered lattice of slots.
  - `choose()` maps biomass, moisture, temperature, elevation, water depth, slope and detritus to a model and scale.
  - A slot changes model only after the new choice persists for 4 updates, so plants do not flicker.
- `fauna.rs`: each organism is an animated deer, or a stag when `size > 1.12`.
  - The drawn pose eases toward the simulated pose between ticks.
  - A new action must hold for about 0.4 s before its clip cross-fades in (300 ms, via `AnimationTransitions`).
  - Clips play at simulated pace, clamped to 0.6–2.5×, and freeze at speed 0 when paused.
- `ui.rs`: the world fills the window and the HUD floats over it as egui `Area`s:
  - top left: glance stats, display only;
  - top centre: clock and speed;
  - top right: Overview and Laws toggles, 3D/Map, and the ⚙ "Under the hood" modal;
  - bottom centre: the tool dock, with a separate hint above it.

  Overview, Laws and Inspector are floating `egui::Window`s. The Inspector exists only while something is selected; its × clears the selection.
  `pointer_over_ui` is `ctx.is_pointer_over_egui()`. There are no side panels any more.
- `theme.rs`: the single Meadow palette. `editor.rs`: the "Laws of nature" window, with a Read view by default and an advanced Graph view.

## Assets

- `assets/client/` is about 12 MB and all CC0. Credits are in `assets/client/ATTRIBUTION.md`.
  - Kenney Nature Kit models in `models/nature/`.
  - Quaternius deer and stag in `models/animals/`. Clip indices: 0 Attack_Headbutt, 3 Eating, 4 Gallop, 9 Walk, 10 Idle_Headlow, 12 Idle.
  - ambientCG ground textures in `textures/terrain/`.
- The Kenney GLBs ship unlit, fully metallic and mint-colored. `tools/prepare_nature_models.py` rewrites only their material JSON:
  - lit, non-metallic, roughness 0.85;
  - colors from a palette by material name.

  The palette is in sRGB and the script converts it to linear. glTF factors are linear, so sRGB values written directly look washed out.
  The script is idempotent. Re-run it after you add or replace a Kenney model.
- Bevy needs specific Cargo features for these assets. `reflect_auto_register` is required for scenes, and `gltf_animation` for clips. Without them the client panics or hangs at load.

## Bevy 0.19 / egui 0.36 notes

- Scenes use `WorldAssetRoot(Handle<WorldAsset>)` and the `WorldInstanceReady` event (module `bevy::world_serialization`), not `SceneRoot`.
- `meshes.get_mut` returns a wrapper that needs a `mut` binding.
- egui rich text: some glyphs render as boxes in the bundled font. Check new emoji in a capture.
- Strong text needs `visuals.widgets.active.fg_stroke` set to the text color, or it becomes invisible on the light theme.

## Owner preferences

- **Simplicity over features.** Earlier rounds removed the guide, experiment tab, overlay and region selectors, and display toggles. Cut before you add.
- **Low cognitive load.** Use thin separators, not boxed cards. Use few tabs. Developer data (budgets, events, performance) belongs in the ⚙ modal, never in the main UI.
- **Name every empty state.** A paused or empty view must say why and what to do, for example "Press ▶ Play to start recording", not "gathering data…".
- **The world should look alive and follow the simulation.** Visuals must come from simulated parameters, not decoration.
- **Inspiration:** Terra Nil, Equilinox and Townscaper. The world comes first, with light floating chrome.

## Open ideas, not started

- Toasts for notable events (a herd is born, an extinction, a drought starts).
- The default camera does not show the world's one lake, at the south edge of `seasonal_river`.
- The terrain weights depend on moisture varying across the map. In the default scenario moisture is near-uniform, so most of the variety comes from shader noise.
