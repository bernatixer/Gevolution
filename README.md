# Gevolution

A single-player ecological god-game sandbox, built in Rust from [`GAME_DEVELOPMENT_SPEC.md`](GAME_DEVELOPMENT_SPEC.md).

The player defines the laws. Rain, evaporation, flow, plant growth, metabolism and reproduction are typed rule graphs the player can edit. The world plays out their consequences, and organisms survive, reproduce and evolve within them. There are no biome switches: a dry valley is simply the state the laws produce.

The previous JavaScript versions live in `old/` (v1) and `oldv2/` (v2).

## Quick start

```sh
# Rust is pinned by rust-toolchain.toml (installs automatically with rustup)
cargo run --release -p game_client            # the game (guided "seasonal river" scenario)
cargo run --release -p sim_cli                # headless run with budgets and metrics
cargo test  --release -p sim_core             # semantic, resource, organism, replay tests
cargo run --release -p sim_cli -- bench benches/manifests/ecosystem-medium.json
```

The first client build compiles Bevy, which takes a few minutes.

## Playing

- **Top bar:** play controls, three tools (🔍 Look, 💧 Water, 🐾 Animals), and the 3D and Map views.
  - Camera controls: right-drag orbits, middle-drag or shift-drag pans, the wheel zooms, WASD moves.
  - The 🎨 menu switches between the Meadow, Lagoon and Grove themes. Set `GEVOLUTION_THEME` to pick one at startup.
- **Overview:** animals, plant cover and open water at a glance, how evolution is shifting inherited traits, and collapsed details for budgets and performance.
- **Laws:** the rules of nature, grouped by topic (Water, Plants, Animals, Weather). Nudge the parameters, or open the law editor to rewrite a formula.
- **Inspector** (right, collapsible): click the land or an animal with 🔍 Look.
  - The land here: temperature, water, soil moisture, plants.
  - *Why is it changing?* shows each law's contribution over the last moment.
  - An animal shows its health, energy, water and inherited traits.

### The law editor

Rules are node graphs over the canonical JSON documents in `assets/rules/`.

- **Wiring:** drag from an output ● to an input ●. Each port shows the unit, semantic quantity, domain and temporal stage that the compiler inferred.
- **Palette:** covers values, arithmetic, logic, spatial operators, organism sensing and effects.
- **Effects:** transfers, reactions, external sources and sinks, rate contributions, movement, death and birth. They request resources, and the shared resolver decides how much is accepted.
- **Regional blend:** this helper multiplies any input by `lerp(1, factor, region)`. Use it to reduce rain upstream, for example.
- **Validation:** runs on every edit and lists errors at the offending node and port. It also summarizes the affected state, any new sources or sinks, and the cost.
- **Apply:** a valid draft replaces the active laws atomically at the next tick boundary. Invalid drafts never touch the running world.
- **Other tools:** undo/redo, package save/load (`saves/laws/…`), templates (collapsed, expandable), and custom fields.

## Architecture

```
crates/
  sim_core/     headless runtime: never depends on a renderer
    units, schema        typed documents: units (absolute vs interval K), quantities, domains
    compiler             validation, type/unit/domain/stage checks, cycle and receipt-feedback
                         detection, writers, stability limits, budgets
    optimize             bit-identical rewrites: folding, CSE, dead code, scheduling, fusion
    eval                 column executor (reference serial / chunked parallel)
    resolver             shared conservative proportional resource resolver
    world                phases 0-8, transactional commit, ledger, lifecycle, commands
    biology              12-16-6 MLP brain backend, keyed mutation
    inspect              explain-this-value for cells and organisms (read-only)
    persistence          versioned saves with build fingerprint; replay-compatible
    catalog, graph_edit  editor metadata and structural graph edits
  sim_cli/      headless runs, budgets, scheduled parameter changes, benchmarks
  game_client/  Bevy + egui: presentation, tools, inspector, law editor
assets/rules/core/       standard environment and biology packs (all gameplay laws)
assets/scenarios/        seeded scenarios, including the guided seasonal river
benches/manifests/       reproducible benchmark definitions
```

The client runs the simulation on a worker thread and consumes immutable snapshots. When the client falls behind, it discards stale snapshots, never simulation ticks.

## Guarantees and their limits

- **Conservation:** tracked water, nutrients, biomass and energy balance each tick, modulo explicitly declared external accounts (weather, atmosphere, photosynthesis toy exchange, and so on). Per-tick tolerance is `1e-9 + 1e-12·max(total, exchange)`. The 100,000-tick closed soak (T15) stays within `1e-6 + 1e-8·scale`.
- **Determinism:** exact replay holds for the same build and profile, on the same platform, with the same scenario, seed, packages and command log. Runs with 1, 2 and 8 workers produce identical hashes. So do the reference and optimized executors. Saves record the build fingerprint and reject incompatible builds. Cross-platform and cross-version bitwise replay is not claimed.
- **Toy models:** hydrology is head-driven transport without momentum. Temperature is a forced relaxation, not an energy balance. Photosynthesis and respiration are declared exchanges, not chemistry. Complex adaptation is possible, not guaranteed; extinction and drift are legitimate outcomes.

## Measured performance

`sim_cli bench` output on an Apple M5 Pro (18 cores, 64 GiB). Reports are written next to the manifests in `benches/`.

| manifest | world | organisms | median tick | p95 | peak RSS |
|---|---:|---:|---:|---:|---:|
| tiny-reference | 32² | 50 | 0.64 ms | 0.81 ms | 14 MiB |
| hydrology-medium | 256² | 0 | 7.6 ms | 8.5 ms | 242 MiB |
| ecosystem-medium | 256² | 2,000 | 9.3 ms | 10.5 ms | 419 MiB |
| ecosystem-large (scaling study) | 512² | 10,000 | 31.7 ms | 55.9 ms | 1.1 GiB |

`ecosystem-medium` meets the initial product targets (p95 ≤ 25 ms, < 1 GiB, plan ≤ 250 ms). The graph compiles in about 3 ms.

Client frame time was measured offscreen at 1680×1000 while simulating at 10×: 3.6 ms median, 7.8 ms p95 (`game_client --screenshot out.png --speed 10`).
