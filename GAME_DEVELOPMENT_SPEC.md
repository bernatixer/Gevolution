# Evolving Worlds
## Game and Engine Development Specification

**Version:** 1.0  
**Date:** 2026-09-29  
**Status:** Development baseline, not an implemented or benchmarked product  
**Implementation language:** Rust  
**Initial product:** Single-player desktop ecological god-game sandbox  
**Simulation:** Two-dimensional fields with terrain elevation  
**Presentation:** Three-dimensional terrain, plus a top-down diagnostic view

> The player defines the laws. The world expresses their consequences. Organisms survive, reproduce, and evolve within those consequences.

---

## Contents

1. [Product vision](#1-product-vision)
2. [Decisions and scope](#2-decisions-and-scope)
3. [Player experience](#3-player-experience)
4. [System architecture](#4-system-architecture)
5. [World and spatial model](#5-world-and-spatial-model)
6. [Typed rule language](#6-typed-rule-language)
7. [State, time, and execution semantics](#7-state-time-and-execution-semantics)
8. [Resource resolution and conservation](#8-resource-resolution-and-conservation)
9. [Initial environmental rules](#9-initial-environmental-rules)
10. [Regions and player-authored laws](#10-regions-and-player-authored-laws)
11. [Organisms and evolution](#11-organisms-and-evolution)
12. [Rule compiler and graph optimization](#12-rule-compiler-and-graph-optimization)
13. [Runtime and performance](#13-runtime-and-performance)
14. [Numerical safety and determinism](#14-numerical-safety-and-determinism)
15. [Rule editor and debugging](#15-rule-editor-and-debugging)
16. [Rendering and presentation](#16-rendering-and-presentation)
17. [Persistence, replay, and rule changes](#17-persistence-replay-and-rule-changes)
18. [Rule-package security](#18-rule-package-security)
19. [Rust workspace and dependency policy](#19-rust-workspace-and-dependency-policy)
20. [Initial configuration](#20-initial-configuration)
21. [Implementation milestones](#21-implementation-milestones)
22. [Verification and acceptance tests](#22-verification-and-acceptance-tests)
23. [Performance gates](#23-performance-gates)
24. [Risks and deferred features](#24-risks-and-deferred-features)
25. [Definition of done](#25-definition-of-done)
26. [Engineering references](#26-engineering-references)

---

## 1. Product vision

Build a god game in which the player creates and edits a world's environmental and biological rules. Those rules act on spatial fields and organisms. Their interactions produce changing landscapes and selective pressures without hardcoded transitions between named biomes.

The player does not paint a tile as a permanent desert. The player changes rainfall, heat input, soil properties, water transport, or other relationships. A dry landscape is a consequence of the simulated state. Rain can restore water, vegetation can expand or retreat, and organisms can respond through behavior and inherited traits.

The central technical asset is a **typed, bounded simulation language**, represented as a graph and executed by a headless Rust runtime. The game client provides visualization, rule authoring, interventions, and explanations.

### 1.1 Design pillars

**Consequences rather than labels.** Environmental state drives behavior. Names such as desert, forest, or wetland are optional descriptions generated for the interface, never authoritative simulation inputs in the standard rule pack.

**Editable laws rather than arbitrary scripts.** Players compose typed expressions, spatial operators, transfers, reactions, and bounded entity actions. The engine validates them before they affect the world.

**Explainable emergence.** Unexpected outcomes are desirable; invisible execution-order bugs are not. A player must be able to inspect why a reservoir drained or an organism died.

**Evolution inside ecology.** Organisms reproduce in the running world. Offspring inherit mutated traits and neural-network parameters. The standard sandbox does not select a global top percentage by an artificial fitness score.

**Correctness before acceleration.** A simple reference executor establishes meaning. Optimizations must preserve the declared numerical contract, rather than silently changing the laws.

### 1.2 Product success

A successful first release lets a player create a small world, alter a regional rule, observe a river become seasonal, inspect resulting vegetation and population changes, and save or branch the experiment. The player can connect the outcome to accepted resource flows and inherited traits rather than to a hidden biome switch.

Complex adaptation, migration, stable coexistence, or an arms race are possible outcomes, not guaranteed outputs. Test environments must establish that the mechanics support selection; the game must not manufacture a discovery and present it as evolved.

## 2. Decisions and scope

### 2.1 Established design direction

| Concern | Decision |
|---|---|
| World representation | Continuous-valued fields, initially stored on a regular 2D grid. |
| Biomes | Derived descriptions; no authoritative `Biome::Desert` behavior. |
| Rule authoring | General typed graph language available to players. |
| Rule execution | Explicit time boundaries, dependencies, effects, and resource arbitration. |
| Optimization | Compile graphs into execution plans; simplify only when semantics permit. |
| Simulation and rendering | Independent; the simulation runs without a game window. |
| Dimensionality | A 2D ecological surface with a height field; 3D presentation. |
| Evolution | Local survival and reproduction with inherited variation. |

### 2.2 Proposed defaults adopted by this baseline

These choices make the specification implementable. They are defaults, not additional user requirements.

The initial game is an offline desktop sandbox without multiplayer, a technology tree, a currency, or a mandatory victory condition. It uses a square grid, CPU-authoritative simulation, fixed integration steps, `f64` authoritative continuous values, a small fixed-topology neural network, and asexual reproduction.

Build a top-down diagnostic prototype before the 3D presentation. The first complete playable slice includes the player-facing graph editor; editing coefficient sliders alone does not satisfy the rule-authoring requirement.

### 2.3 What “the rules can define everything” means

The language must be extensible enough to express environmental processes, physiological relationships, action costs, sensing, inheritance parameters, and bounded lifecycle conditions through a shared type and effect system.

It does **not** mean unrestricted player code, arbitrary memory access, an automatically solvable system of equations, or a promise that every future mechanic will fit the initial operator set. The trusted engine owns storage, scheduling, numerical integration, validation, rendering, and capability implementations. Adding a fundamentally new primitive requires a reviewed engine extension.

Two graphs remain conceptually distinct:

- The **law graph** describes environmental and biological relationships.
- An organism's **brain graph** maps observations to attempted actions.

They may share evaluation infrastructure, but a brain cannot edit global laws or bypass action costs.

### 2.4 Initial release boundary

Include terrain, surface and soil water, a groundwater reservoir, externally driven weather, temperature, vegetation, a simple nutrient cycle, evolving herbivores, regional laws, graph editing, inspection, deterministic same-build replay, and saved experiments.

Defer full atmospheric circulation, volumetric fluid simulation, terrain erosion, caves, predators, sexual reproduction, topology evolution, multiplayer, native-code player plugins, and GPU-authoritative simulation. Preserve interfaces for these without implementing them speculatively.

## 3. Player experience

### 3.1 Core loop

```text
Generate a landscape and seed life
    -> Observe fields, organisms, and resource budgets
    -> Form a hypothesis
    -> Change a law or make an explicit intervention
    -> Run, pause, single-step, or accelerate
    -> Inspect consequences and inheritance
    -> Save, compare, branch, or revise the experiment
```

Rule editing and intervention are different actions. Editing evaporation changes an ongoing relationship. Adding water is an explicit resource source. Both are recorded, but neither masquerades as the other.

### 3.2 First guided scenario: the seasonal river

The starter world contains elevated terrain feeding a lower basin. The player sees surface water, soil moisture, temperature, vegetation, and organisms.

The scenario guides the player to reduce rainfall over an upstream region. Over enough simulated time, the river can shrink as outflows exceed replenishment. The player inspects the water budget, then restores rainfall and observes recovery where the remaining conditions allow it.

Next, the player modifies the relationship between temperature and water demand. Organisms with different inherited traits experience different costs. A lineage panel exposes reproduction and trait distributions over time.

The guide supplies starting conditions and explanations, not scripted population migration or forced biome transitions.

### 3.3 Player tools

Provide world generation and seeded reset; pause and single-step; time-speed controls; regional selection; typed rule creation and duplication; parameter editing; explicit rain/water/organism interventions; field overlays; organism inspection; lineage statistics; save/load; and checkpoint-based experiment branching.

Changing the camera or hiding an overlay must never change the simulation result.

## 4. System architecture

```text
Player graph editor / built-in rule packages
                    |
                    v
        Versioned rule document + schemas
                    |
                    v
     Validate -> type-check -> analyze -> lower
                    |
                    v
            Compiled execution plan
                    |
                    v
               Rust runtime
       +------------+--------------+
       |            |              |
     Fields      Organisms    Resource ledger
       |            |              |
       +------------+--------------+
                    |
          Immutable presentation snapshots
                    |
                    v
        Bevy client: terrain, UI, inspection
```

### 4.1 Ownership boundaries

**The simulation runtime** owns authoritative world state, simulation time, stable IDs, random streams, active law versions, resource budgets, and lifecycle events.

**The compiler** owns type checking, dependency analysis, bounded spatial expansion, effect validation, numerical-mode restrictions, execution planning, and source maps.

**The client** owns cameras, meshes, animation, audio, editor interaction, and presentation-only labels. It submits validated commands for a specified simulation tick, not direct mutations of world buffers.

**The standard rule pack** owns the initial gameplay formulas and parameters. The engine must not contain a second hidden version of the ecology that bypasses this pack.

### 4.2 Engine invariant

Given the same initial state, validated rules, numerical profile, engine build, seed, and tick-stamped commands, execution must produce the same authoritative state under the supported same-build replay contract.

This is an engineering requirement to verify, not a consequence automatically provided by an ECS or graph library.

## 5. World and spatial model

### 5.1 Domains

The language distinguishes at least:

```text
Uniform<T>                   One value for a world or invocation
CellField<T, GridId>          One value per cell on a specified grid
EdgeField<T, GridId>          One value per canonical neighbor edge
EntityColumn<T, ArchetypeId>  One value per entity of an archetype
RegionMask<GridId>            Cell membership or coverage in [0, 1]
```

A field type includes its domain identity, not just the number of spatial dimensions. Two 2D fields at different resolutions cannot be multiplied without explicit sampling or resampling.

Reserve an extensible domain descriptor for layered or 3D fields. Only the regular 2D backend is required initially. Do not implement a fully generic mesh library before the square-grid simulation works.

### 5.2 Coordinates and neighbors

Use simulation coordinates `(x, z)` on the ground plane and `elevation` for height. The initial transport neighborhood contains four orthogonal neighbors. Eight-neighbor sensing can be offered separately with explicit distance weighting.

Each grid stores its dimensions, cell spacing in meters, area per cell, origin, boundary policy, and stable identity. An edge has one canonical ID independent of the worker that evaluates it. Transport proposals are emitted once per edge, not once from each endpoint.

Chunk edges must use the same neighborhood behavior as interior edges. Read-only halo cells supply neighboring values; ownership of writes remains explicit.

### 5.3 Storage

Use structure-of-arrays storage in chunks. Store temperature values together, surface-water amounts together, and so on. Do not create one Bevy entity per simulation cell.

The authoritative view of a cell is a projection over those arrays; a convenient `Tile` struct in editor code is not the storage contract.

Start with fixed-resolution, fully resident worlds. Camera-based unloading must not freeze ecology. Out-of-core simulation and changing simulation resolution require separate conservation and scheduling designs.

### 5.4 Initial state and quantities

| Field | Quantity / representation | Role |
|---|---|---|
| `elevation` | Length, meters | Terrain height; immutable during ordinary initial-release ticks. |
| `temperature` | Absolute temperature, kelvin | Dynamic environmental state. |
| `surface_water` | Water mass, kilograms per cell inventory | Rivers, ponds, and immediately accessible water. |
| `soil_water` | Water mass, kilograms per cell inventory | Water held in the soil reservoir. |
| `groundwater` | Water mass, kilograms per cell inventory | Slow below-ground reservoir. |
| `vegetation_biomass` | Biomass mass, kilograms per cell inventory | Plant food and visual density. |
| `soil_nutrients` | Nutrient mass, kilograms per cell inventory | Available nutrient inventory. |
| `plant_nutrients` | Nutrient mass, kilograms per cell inventory | Nutrients retained in living plants. |
| `detritus_biomass` | Biomass mass, kilograms per cell inventory | Dead organic material. |
| `detritus_nutrients` | Nutrient mass, kilograms per cell inventory | Nutrients available through decomposition. |
| `soil_water_capacity` | Water mass, kilograms per cell | Material parameter, editable at a tick boundary. |
| `hydraulic_conductance` | Declared conductance units | Edge/cell material parameter used by transport. |

Other parameters include heat response, infiltration coefficients, plant growth limits, and groundwater release coefficients. Each must declare a unit and valid range.

Inventories are amounts in a cell, not densities. Convert a mass density to inventory using cell area, and use the inverse conversion for overlays and rate laws. Derive surface-water depth as:

```text
water_depth = surface_water_mass / (water_density * cell_area)
```

`water_density` is a declared toy-model constant in `kg/m^3`; the result is meters. Never add liters directly to a terrain height or use a per-area rate as a whole-cell amount.

### 5.5 State, forcing, and derived values

**State** persists across ticks: water inventories, temperature, biomass, entity energy, age, and neural memory when supported.

**Forcing** is supplied by a declared external process: solar input, rainfall, ambient temperature, or humidity in the initial weather model.

**Derived values** are pure functions of state, forcing, and parameters: water depth, hydraulic head, moisture ratio, vegetation cover, and environmental classifications.

The initial weather model is intentionally open: rain enters through a named external water source and evaporation exits to a named external atmosphere account. It does not claim to simulate a closed water cycle. A later atmosphere can replace those boundary accounts with tracked atmospheric reservoirs.

### 5.6 Boundary conditions

Default to a closed horizontal water boundary. Optional open outflow boundaries must record every exported amount in the ledger. Periodic boundaries are supported only by explicitly selected scenarios.

The world generator sets elevation, material properties, initial reservoir amounts, and forcing parameters. It does not assign permanent ecological identities.

## 6. Typed rule language

### 6.1 Representation

Player rules are data, not arbitrary Rust closures. The canonical authoring format is a versioned JSON graph document. A node editor reads and writes that document; built-in packages use the same schema.

The compiler produces a typed intermediate representation, then an execution plan. Type metadata may be erased from hot numerical loops only after validation. Dynamic player-authored fields require runtime schema/type checking; Rust generic types alone cannot validate a graph loaded from disk.

A rule package declares stable IDs, parameters, field/component schemas, graph nodes and connections, scope, permitted effects, package version, and required engine capabilities.

### 6.2 Type information

Every graph port carries:

```text
Type = {
    value_kind,        // number, bool, vector, ID, bounded record
    physical_unit,     // dimensions and canonical scale
    semantic_quantity,// water, nutrient, biomass, energy, etc.
    domain,            // uniform, cells on grid A, edges, entities
    temporal_stage,   // snapshot, proposal, receipt, next-state intent
    effect_class      // pure value, transfer request, lifecycle request
}
```

Maintain physical dimensions separately from semantic tags. Water and nutrients can both be measured in kilograms, but a water transfer cannot silently become a nutrient transfer. Explicit conversion or reaction definitions must declare the relationship and its budget effects.

Unit checking is necessary, not sufficient: a dimensionally valid rule can still be numerically unstable or ecologically unreasonable.

### 6.3 Unit and domain examples

```text
WaterMass + WaterMass                         valid
WaterMass / Duration                          WaterMassRate
WaterMassRate * Duration                      WaterMass
WaterMass + Temperature                       invalid
CellField<WaterMass, A> + EntityColumn<WaterMass, Herbivore>
                                              invalid without a domain operation
CellField<WaterMass, A> + CellField<WaterMass, B>
                                              invalid without explicit resampling
```

Absolute temperature and temperature intervals are distinct. Subtracting two absolute temperatures produces an interval; adding an interval to an absolute temperature produces an absolute temperature. Raw Celsius values must not be used in multiplicative physical expressions. Normalize through a reference temperature or convert to an appropriate absolute quantity. The distinction is also reflected in `uom`'s quantity model. [R4]

Functions such as logarithm and exponential require dimensionless arguments. Curves declare their input and output units. Numeric literals require either a declared unit or an explicitly dimensionless type.

### 6.4 Initial pure operators

Provide constants, parameters, state reads, unit conversion, addition, subtraction, multiplication, safe division, min/max, clamp, comparisons, Boolean logic, explicit selection, linear interpolation, bounded piecewise-linear curves, dot products, and selected bounded vector operations.

Provide explicit spatial operators for reading an endpoint, orthogonal neighbors, bounded neighborhood sums/averages, gradients, Laplacians, region reductions, field sampling at entity positions, and entity-to-cell resource requests.

Neighborhood radius and maximum entity candidates must be statically bounded. There is no unbounded recursive graph traversal or unrestricted all-entities query inside a rule.

`select` is eager in the initial graph evaluator. Both inputs must be safe to evaluate. `safe_divide` requires an explicit fallback with the output unit; it does not hide a zero denominator by inventing an epsilon.

### 6.5 Effects and state ownership

| Effect | Meaning |
|---|---|
| `Transfer` | Request movement of one resource between two reservoirs. |
| `Reaction` | Request a coupled, explicitly declared transformation with multiple inputs/outputs. |
| `ExternalSource` / `ExternalSink` | Exchange with a named boundary or divine-intervention account. |
| `RateContribution` | Contribute to a non-conserved continuous state through a registered integrator. |
| `NextValue` | Supply the single registered next-value expression for a state field. |
| `ActionIntent` | Request bounded movement, sensing, eating, drinking, or another registered capability. |
| `LifecycleIntent` | Request birth or death processing under engine-owned commit rules. |

Each state field declares exactly one update policy: conserved reservoir, integrated rate, exclusive next value, immutable parameter, or engine-owned lifecycle value. Multiple rules may request transfers involving the same reservoir. Multiple exclusive writers to a `NextValue` field are a compile error unless the graph explicitly combines their values before the single writer.

A rule cannot write an arbitrary delta directly to a conserved reservoir.

### 6.6 State feedback

A pure expression graph within a temporal stage must be acyclic. Feedback is expressed through stored state or an explicit delay with an initial value.

```text
vegetation_now -> transpiration_request -> accepted_water_loss
humidity_now  -> rainfall_request       -> accepted_rain
accepted processes -> next state
next tick reads that committed state
```

The compiler must not insert hidden delays to resolve an algebraic cycle. It reports the cycle and requires the author to introduce state, reformulate the graph, or use a future explicitly supported solver.

## 7. State, time, and execution semantics

### 7.1 Fixed simulation time

The initial integration tick advances by a fixed `dt = 0.25 simulated seconds`. Time is stored as an integer tick count plus the configured time quantum, not as a repeatedly accumulated floating-point clock.

This value is a starting configuration, not a universal stability guarantee. Every shipped rule pack must pass numerical tests at its allowed timestep and parameter ranges. Changing the integration timestep changes the numerical model and is recorded as an experiment configuration change.

Game speed changes how many simulation ticks execute per wall-clock second, never the size of an individual tick. At `1x`, four default ticks represent one wall-clock second. Headless runs execute a specified number of ticks without waiting for rendering.

The initial release evaluates active simulation rules every tick. Multirate scheduling is deferred until the baseline is verified. A later slow system must integrate its actual elapsed simulated duration and specify whether it holds rates, aggregates inputs, or substeps; “run less often” is not a semantics-free optimization.

### 7.2 The authoritative tick

The engine uses the following versioned phases:

| Phase | Reads | Produces |
|---|---|---|
| 0. Boundary commands | Last committed world | Validated rule/schema changes and explicit interventions. |
| 1. Snapshot and forcing | Boundary-adjusted world, tick, seed | Immutable `S_n`, external forcing values. |
| 2. Pure evaluation | `S_n`, forcing, parameters | Derived values, observations, brain outputs. |
| 3. Proposals | Snapshot-derived values | Resource requests and bounded action intents. |
| 4. Resolution | Snapshot inventories, all competing requests | Accepted process amounts and receipts. |
| 5. Outcomes | `S_n`, accepted receipts | Actual movement, stress, health, and other non-resource changes. |
| 6. Candidate integration | Snapshot, accepted changes | Candidate `S_(n+1)`. |
| 7. Lifecycle | Candidate state, lifecycle intents | Atomic births, deaths, and ownership transfers. |
| 8. Validation and commit | Complete candidate | Committed world, event log, optional presentation snapshot. |

Phase order is part of the language contract. **Rule registration order is not.** Independent operations within a phase may run in any legal schedule, subject to the deterministic numerical reduction contract.

This is deliberately more precise than saying “all rules run simultaneously.” Dependencies, resource conflicts, numerical integration, and discrete events still need defined semantics.

### 7.3 Snapshot visibility

All resource proposals read the same `S_n`. Rain received during a tick is not available to an evaporation or transport request until the next tick. Likewise, water arriving at cell B cannot be re-spent by B during that same tick.

This is an explicit numerical choice. It prevents accidental multiple-cell propagation based on iteration order. Faster transport requires a smaller approved timestep or a separately specified solver, not an in-place loop that happens to visit cells in a favorable order.

An explicit intervention in phase 0 is different: it is applied before `S_n` is captured and therefore is visible that tick. Its source or sink is recorded in the intervention ledger.

### 7.4 Accepted results, not requested results

A rule can request more water or energy than it receives. Dependent outcomes must use its **accepted receipt**.

For example, a movement request costing 4 J cannot move the organism the full requested distance when only 1 J was accepted. A plant cannot receive full biomass growth when only half of the required nutrients were available.

Receipt-dependent calculations form a later DAG. They may update permitted non-conserved outcomes, but cannot submit new resource demands in the same tick. Feedback from a receipt into an earlier proposal is a compile error.

### 7.5 Transactional failure

Build changes in candidate buffers. Validate finiteness, resource constraints, schema constraints, and lifecycle ownership before swapping buffers.

A failed tick leaves the previously committed state intact, pauses the experiment, and records the smallest reproducible diagnostic context: tick, graph version, node IDs, affected cells/entities, and inputs. No partial world state is published.

A graph must not silently disappear because it is expensive or produces an invalid value.

## 8. Resource resolution and conservation

### 8.1 Why a delta buffer is insufficient

Suppose a reservoir contains 10 kg of water. Evaporation requests 8 kg and downhill transport requests 8 kg. Both requests read a valid snapshot; blindly summing their deltas still produces a negative reservoir.

Clamping the source to zero afterward is not a fix: destinations may already have received 16 kg from a source containing only 10 kg.

The engine must resolve all competing withdrawals **before** applying changes. Environment rules, organism drinking, plant uptake, and other consumers share the same resolver for the same resource.

### 8.2 Process model

A process request contains a stable process-instance ID, nonnegative requested input amounts, nonnegative requested output amounts, resource identities, source/destination reservoirs, rule provenance, and any declared external accounts.

Amounts are already integrated over the tick. If a graph emits a rate, the engine multiplies by `dt` exactly once while building the request.

A transfer has matching input and output amounts of the same resource. A coupled reaction has several inputs and outputs that all receive one common acceptance factor. Conservation declarations describe which tracked quantities must balance and which named external exchanges are allowed.

Resource-preserving reaction signatures derive matched inputs and outputs from shared typed amounts and declared coefficients. The compiler validates their conservation relationships; it must not accept independently computed outputs merely because the units match. General transformations require explicit declared exchanges. Runtime validation checks the accepted budgets as well.

A no-op self-transfer is eliminated before arbitration. Do not represent net-zero internal bookkeeping as competing withdrawals and deposits.

### 8.3 Initial deterministic allocation policy

Use conservative proportional scaling. For reservoir `r`, let `S_r` be its snapshot amount and `D_r` the sum of all requested withdrawals:

```text
source_factor_r = 1                         when D_r = 0
source_factor_r = min(1, S_r / D_r)         otherwise
```

For a capacity-limited destination with capacity `C_r`, snapshot free space is `F_r = max(0, C_r - S_r)`. If total requested deposits are `I_r`:

```text
destination_factor_r = 1                    when I_r = 0
destination_factor_r = min(1, F_r / I_r)    otherwise
```

For process `p`:

```text
alpha_p = minimum of:
    1,
    every relevant source_factor,
    every relevant destination_factor

accepted_input_(r,p)  = alpha_p * requested_input_(r,p)
accepted_output_(r,p) = alpha_p * requested_output_(r,p)
```

Then integrate each reservoir from its snapshot plus accepted deposits minus accepted withdrawals. Infinite-capacity destinations omit the destination factor. Declared external sources do not pretend to have a finite internal inventory, but remain subject to package limits and accounting.

This policy does not credit simultaneous inflows toward withdrawals or simultaneous withdrawals toward free capacity. It is intentionally conservative and can underuse resources for coupled multi-input processes. It guarantees feasible accepted amounts under the stated assumptions; it does not claim an optimal or maximally fair allocation.

Do not add an order-dependent “give the leftovers to the next rule” pass. A future iterative or prioritized allocator is a new, versioned policy with its own tests.

### 8.4 Worked examples

**Competing withdrawals:** 10 kg available, with two requests of 8 kg. The shared factor is `10 / 16 = 0.625`; each receives 5 kg. The source becomes 0 kg, and destinations collectively gain 10 kg.

**Coupled growth:** a process requests 2 kg of water and 1 kg of nutrients for its declared biomass output. If water permits a factor of 0.5 and nutrients permit 0.8, the whole process receives factor 0.5. Every input, output, and associated energy exchange scales together.

**Capacity limit:** a soil reservoir has 7 kg and a capacity of 10 kg. Two infiltration requests each offer 4 kg. Destination space is 3 kg; the combined requested inflow is 8 kg. Both deposits are scaled by at most `3 / 8`, and rejected water stays in its original source.

### 8.5 Conservation ledger

For each declared conserved quantity, track:

```text
total_next = total_now + explicit_external_inputs - explicit_external_outputs
```

Internal transfers cancel. Totals include applicable cell, entity, detritus, and lifecycle-owned reservoirs. Deleting an entity must transfer its remaining tracked water and nutrients somewhere; removing its rows from storage is not a resource sink.

The initial standard pack conserves tracked **water** and **nutrient** inventories modulo declared external accounts and bounded numerical error. It does not claim closed, scientifically complete conservation of all mass and energy. Temperature is a simplified forced state, and photosynthesis/respiration use declared toy-model exchanges rather than a full chemical simulation.

Every package exposes its conservation profile. A “divine creation” rule is allowed in sandbox mode through an explicit external source. It is visibly non-closed, not a hidden exception to conservation checks.

### 8.6 Floating-point accounting

Use deterministic accumulation order and a higher-accuracy diagnostic summation strategy for global budgets. Define per-resource absolute and relative tolerances in the numerical profile; benchmark their growth over long runs.

Tiny roundoff corrections, when permitted, must be bounded and recorded. Material negative inventories, capacity violations, or unexplained drift fail validation. Never rely on blanket `max(0, value)` as the primary correctness mechanism.

## 9. Initial environmental rules

The following are intentionally simplified game models. They establish causal relationships and testable budgets, not a claim of scientific climate or hydrological fidelity. Formula syntax is explanatory; it is not an existing Rust API.

### 9.1 Rainfall

Rainfall is a nonnegative mass flux per ground area, supplied by a weather profile and optionally a regional mask:

```text
requested_rain_mass = rain_flux * cell_area * dt * region_coverage
external.weather_water -> cell.surface_water
```

Units are `kg/(m^2*s) * m^2 * s = kg`. A prescribed seasonal/weather function is sufficient initially. The weather source must appear in the water budget.

### 9.2 Evaporation

Use a bounded temperature response and an explicit dimensionless humidity input:

```text
temperature_factor = clamp(1 + beta * (T - T_ref), 0, 3)
dryness_factor     = clamp(1 - relative_humidity, 0, 1)
evaporation_rate   = k_evap * surface_water * temperature_factor * dryness_factor
```

`beta` has units of inverse temperature interval, `k_evap` has units `1/s`, and the result is `kg/s`. Emit a transfer from surface water to the external atmosphere account. A separate soil-evaporation rule can withdraw soil water through the same resolver.

Base any evaporation-related secondary effect on the accepted water amount. Do not assume the full requested amount occurred.

### 9.3 Surface-water transport

Derive hydraulic head from terrain elevation and water depth:

```text
H_i = elevation_i + water_mass_i / (water_density * cell_area_i)
requested_rate_i_to_j = conductance_ij * max(H_i - H_j, 0)
```

`conductance_ij` is measured in `kg/(m*s)`, yielding `kg/s`. Determine direction once on each canonical edge and emit one transfer. Equal head produces no request, even when terrain elevations differ beneath the water surface.

This is a simplified head-driven transport model without fluid momentum. It can represent pooling and connected surface flow, but is not a full shallow-water solver. Donor scaling prevents overspending; an additional timestep/conductance restriction is required to avoid large oscillations. Test on basins and connected networks, not only a single pair of cells.

### 9.4 Infiltration and groundwater

Infiltration requests move surface water to a capacity-limited soil reservoir:

```text
requested_infiltration_rate = k_infiltration * surface_water
```

The resolver applies both source availability and destination capacity. Rejected inflow remains on the surface.

Drainage moves soil water above a declared retention threshold into groundwater. A simplified spring rule returns groundwater above a threshold to surface water. Below-ground lateral transport is optional after the initial local groundwater loop is verified; it must have its own declared hydraulic model rather than reuse surface elevation blindly.

### 9.5 Temperature

Start with a forced relaxation model and optional uniform-capacity diffusion:

```text
temperature_rate = (ambient_temperature - temperature) / relaxation_time
                 + thermal_diffusivity * laplacian(temperature)
                 + heating_rate * (1 - shade_strength * vegetation_cover)
```

The result is a temperature-interval rate. Vegetation reduces the heating term; it does not multiply an absolute Celsius temperature by a cooling coefficient.

For the standard explicit 2D diffusion stencil, require the nonnegative-weight condition:

```text
dt * diffusivity * (2 / dx^2 + 2 / dz^2) <= 1
```

This condition covers that diffusion term, not every other coupled rule. The complete pack still requires timestep refinement tests. Temperature is not treated as a conserved quantity. A future thermal-energy model must represent heat capacity and energy flux explicitly.

### 9.6 Vegetation growth and mortality

A starting potential growth model is:

```text
potential_growth_rate = growth_coefficient * biomass
                      * max(0, 1 - biomass / carrying_capacity)
                      * moisture_response
                      * nutrient_response
                      * temperature_response
```

Each response is a bounded dimensionless curve. Convert potential growth into one coupled process: nutrient transfer from soil to plant stores, water expenditure to an explicit transpiration destination, and biomass creation through a named photosynthetic-growth exchange. Accepted inputs determine accepted growth.

Vegetation mortality moves associated biomass and plant nutrients to detritus. Decomposition returns detritus nutrients to soil while recording the declared biomass/respiration exchange.

Provide seeded initial vegetation and a bounded, resource-funded neighbor dispersal process. A rule proportional to existing biomass cannot spontaneously recolonize a world in which every plant has disappeared. Recovery after complete extinction requires surviving propagules or an explicit intervention.

### 9.7 Required built-in interactions

The standard pack must support these chains without biome-specific conditionals:

```text
Rain -> surface water -> transport -> soil water -> vegetation growth
Heat -> increased evaporative demand -> lower water availability
Vegetation -> reduced heating + water demand + food availability
Organisms -> consumption + waste + death deposits -> changed local resources
Dry conditions -> organism stress -> differential survival and reproduction
```

Whether a particular run exhibits each outcome depends on starting conditions, coefficients, and time. Tutorial fixtures must be tuned and tested rather than relying on every random world to demonstrate every chain.

## 10. Regions and player-authored laws

### 10.1 Spatial scope

A rule can operate globally, on a saved region mask, on cells satisfying a typed predicate, on canonical edges selected by an explicit policy, or on entities matching an archetype/component query.

A region can be painted, rectangular, circular, or computed from fields. Region membership is not a biome. A predicate such as `soil_moisture_ratio < threshold` is evaluated against the current snapshot and may change next tick.

Region statistics require explicit reductions and broadcasts. A rainfall rule depending on regional mean temperature reads `mean_temperature(S_n)`; it does not see a partially updated region.

### 10.2 Overlapping rules

There is no hidden last-writer-wins behavior.

Independent transfer requests are additive and compete through the shared resolver. Two overlapping rainfall-source rules intentionally add rain; the editor must show the combined contribution.

For coefficients, use an explicit blend/composition expression. A parameter with several uncombined exclusive writers is invalid. The editor offers visible additive modifiers and weighted blends, with range validation, rather than a drag-order-based priority list.

### 10.3 Cross-boundary transport

A spatial mask controls where a rule applies, not whether conservation applies. A rule that transports water across the edge of a selected region must debit the actual source and credit the actual destination, even when the destination is outside the mask.

An impermeable boundary is a separate declared conductance or boundary rule. It is not created accidentally because a chunk, region, or camera view ends.

### 10.4 Authoring example

A player-created dry-region package could expose a region mask, an evaporation coefficient, a temperature response curve, and a rainfall multiplier. The compiler expands its expression once into a domain-aware plan, not into one heap-allocated graph per cell.

The following proposed JSON shape illustrates the canonical document contract for a minimal evaporation rule. The production schema must formalize these fields and validate them; this is a specification example, not a claim that a loader already exists.

```json
{
  "schema_version": 1,
  "package_id": "example.simple_evaporation",
  "requires": ["core.environment.v1"],
  "rule_id": "example.simple_evaporation.main",
  "domain": { "kind": "cells", "grid": "world.surface" },
  "parameters": {
    "rate": { "value": 0.00001, "unit": "1/s", "min": 0.0, "max": 0.1 }
  },
  "nodes": [
    { "id": "water", "op": "read_state", "field": "surface_water" },
    { "id": "coefficient", "op": "parameter", "name": "rate" },
    {
      "id": "requested_rate",
      "op": "multiply",
      "inputs": ["water.value", "coefficient.value"]
    },
    {
      "id": "evaporate",
      "op": "external_sink",
      "resource": "water",
      "from": "surface_water",
      "account": "external.atmosphere",
      "rate": "requested_rate.value"
    }
  ],
  "effects": ["evaporate.request"]
}
```

A full evaporation rule adds typed temperature, humidity, and region nodes before the effect. The engine supplies `dt`; the author does not multiply by it again inside a port declared as a rate.

### 10.5 New player-defined fields

Support registering bounded scalar state or derived fields with units, domains, defaults, and update policies. Examples include a new toxin concentration, divine influence level, or alternative energy reserve.

New conserved resources require an explicit resource schema and budget policy. New rendering interpretations are optional; a generic overlay remains available.

Initial field additions occur while paused through a validated tick-boundary migration. Changing a populated field's unit, domain, or semantic identity requires an explicit migration or a new world. Renaming a label does not change its stable ID.

## 11. Organisms and evolution

### 11.1 Initial organism model

Begin with one herbivore archetype and field-based vegetation. An organism stores stable identity, parent/lineage identity, position, heading, age, genome, phenotype parameters, neural parameters, energy, body water, nutrient stores, health/stress, and reproduction cooldown.

The genome is inherited data. The phenotype is the validated mapping from genome to capacities and response curves. Current dehydration or starvation is runtime state, not an automatically inherited trait.

### 11.2 Initial heritable traits

Include body size, preferred temperature, thermal tolerance breadth, water-conservation efficiency, locomotion efficiency, and neural-network weights/biases.

Every benefit needs an explicit trade-off. A broad thermal tolerance can carry a maintenance cost; a large water reserve can carry a size or movement cost. Avoid one independently mutable “better at everything” coefficient unless the player deliberately authors that rule.

Trait bounds and costs belong to the standard biological rule package. The generic inheritance runtime enforces schema validity, not a hardcoded desert-adaptation objective.

### 11.3 Brain and sensing

Start with a fixed `12 -> 16 -> 6` feed-forward network. Its 310 weights and biases are inherited and mutated. A small dedicated evaluator is sufficient; a general deep-learning framework is not required by this design.

The twelve normalized inputs are energy fraction, hydration fraction, temperature discomfort, local vegetation availability, local water availability, forward and lateral food gradients, forward and lateral water gradients, age fraction, local crowding, and a keyed exploration-noise value.

The six outputs are turn, forward movement, eat intent, drink intent, reproduction intent, and rest tendency. Map outputs to bounded action requests through explicit physiology rules. An output cannot directly set position, refill energy, or spawn an entity.

Use explicit normalization constants and clamp ranges. Sensors see local values and bounded neighborhoods, not the world map or a biome name. Built-in brains can be evaluated in batches because their topology is shared.

### 11.4 Resource and movement semantics

Eating transfers accepted plant biomass/nutrients and produces declared energy and waste outcomes. Drinking moves water from the sampled cell to body water. Body-water losses go to a declared local or external destination.

For the initial tick semantics, eating and drinking target the organism's snapshot location. Movement is committed for the next tick; it does not permit consumption at both origin and destination during one tick.

Actual movement is limited by accepted energy, bounded speed, terrain constraints, and world boundaries. Unmet basal needs can increase stress through receipt-dependent outcome rules. Rest is reduced activity, not a free source of energy.

No pairwise rigid-body physics is required initially. Use a spatial index and bounded crowding/sensing queries rather than all-pairs entity checks.

### 11.5 Reproduction and lifecycle

Initial reproduction is asexual and local. An eligible parent submits at most one birth intent per tick. Eligibility requires maturity, cooldown completion, an approved reproduction output, and sufficient candidate-state resources after ordinary resource processing.

Lifecycle processing is an explicit discrete phase. Death takes precedence over birth. A parent that dies in the candidate state does not reproduce that tick. An accepted birth atomically debits the parent and initializes the offspring with the transferred energy, water, nutrients, and mutated genome.

Offspring do not act until the following tick. Rejected births spend no resources. Population-cap conflicts use a stable, seed-keyed ordering, never worker arrival order or hidden genome fitness.

On death, remaining tracked resources move to the defined detritus or environmental destinations. Entity storage is removed only after ownership transfers are recorded.

### 11.6 Mutation and selection

Mutate bounded trait values and neural weights using seed-keyed noise at birth. Preserve parentage, generation, mutation policy version, and birth location. Do not copy an adult's learned or transient neural state into offspring unless a later rule explicitly declares that inheritance model.

Selection occurs through survival and successful reproduction in the shared world. Log reproductive success for analysis, but do not use a global ranking to replace the population.

Population bottlenecks, drift, extinction, and weak adaptation are legitimate outcomes. Compare multiple seeds and controlled environments before interpreting a trait change as adaptation.

### 11.7 Later topology evolution

Provide a `BrainBackend` boundary so a later bounded graph-genome implementation can replace the fixed network. Topology mutation, recurrent memory, crossover alignment, and species/compatibility handling require separate design and tests.

Neataptic is a conceptual reference for flexible network structure and neuroevolution, not a runtime dependency or a specification for this game's ecology. Its repository describes those capabilities and currently marks the project unmaintained. [R8] Do not label an arbitrary mutation loop “NEAT” without implementing the chosen algorithm's actual contract.

## 12. Rule compiler and graph optimization

### 12.1 Compilation pipeline

```text
Versioned source graph
    -> schema and capability validation
    -> symbol resolution
    -> quantity, unit, domain, and stage checking
    -> writer/effect/conservation validation
    -> cycle detection and bounded-cost analysis
    -> typed intermediate representation
    -> permitted simplification and common-expression sharing
    -> domain partitioning and kernel fusion
    -> buffer-lifetime and deterministic reduction planning
    -> executable plan + diagnostics + source map
```

Compile when rules or relevant schemas change, not every tick. A parameter-value update can reuse a plan when the value is not specialized into code and remains inside its validated range. Otherwise it invalidates the appropriate compilation cache.

### 12.2 Required optimizations

Start with constant folding under the selected numerical semantics; removal of unused pure expressions; common-subexpression elimination for identical pure nodes; uniform-value hoisting; static material/geometry caching; reuse of temporary buffers; and fusion of compatible pointwise chains.

All state writes, resource effects, lifecycle effects, and required diagnostics are graph roots. A transfer cannot be eliminated merely because its result is not displayed. Two identical-looking transfers remain two requests unless the author explicitly combines them.

### 12.3 Floating-point rewrite restrictions

Mathematical equivalence is not always floating-point equivalence. For example, `(x * 2) * 0.5` is not universally replaceable with `x`: intermediate overflow and rounding can matter. Similarly, reassociation and fused multiply-add can change results. LLVM documents these as separately permitted floating-point transformations. [R5]

The reference profile preserves the graph's primitive operation order and prohibits unapproved reassociation, approximate functions, and implicit arithmetic contraction. A rewrite is allowed only when it preserves the profile's semantics or has a valid range-based proof.

Fusion means avoiding unnecessary loops and temporary arrays; it does not automatically authorize different arithmetic. A future relaxed profile must be explicit, versioned, tested against stated tolerances, and excluded from strict replay claims.

### 12.4 Parallel scheduling

Use dependencies to identify parallel work, but do not launch one world-sized pass for every graph layer by default. Fuse compatible local expressions, then partition execution by domain, chunk, and effect boundary.

Neighbor gathers, regional reductions, resource resolution, and lifecycle processing introduce barriers or ownership constraints. They are not all interchangeable pointwise math.

Evaluation order for independent nodes may vary. Reduction order for authoritative floating-point totals must remain canonical. Rayon documents that its ordinary parallel floating-point sums need not be fully deterministic; use it to schedule work, not to define numerical reduction semantics. [R3]

### 12.5 Randomness

Randomness is an explicit graph input keyed by world seed, tick, stable rule/node ID, stable cell/entity ID, and draw index. The algorithm and conversion to numerical values are versioned.

A random node is not ordinary common-subexpression-elimination material. Sharing a draw requires an explicit shared node/stream reference. Parallel scheduling, opening an inspector, or adding an unrelated visual effect must not advance another organism's random stream.

### 12.6 Incremental evaluation

Cache true invariants first: terrain-derived data, unchanged material calculations, constant curves, and uniform parameters.

For later dynamic caching, track state versions, time dependence, random dependence, and spatial dependency radius. A changed field invalidates affected neighborhoods, not only the changed cell. A regional reduction can invalidate every dependent cell in that region.

A constant rate still needs integration every tick. A visually unchanged forest may still respire, age, or lose water. Do not skip a chunk because it looks stationary or because its inputs were unchanged in one earlier frame. Sleeping requires a proven zero-activity condition and complete wake-up dependencies.

### 12.7 Limits of simplification

The compiler simplifies computation inside the defined model. It cannot generally collapse thousands of nonlinear, thresholded, stochastic, resource-constrained ticks into one equivalent operation.

Topology changes, birth/death events, neighbor interactions, and resource competition make the runtime a hybrid simulation, not one unrestricted algebraic formula. Closed-form fast-forwarding is permitted only for specifically proven submodels and is outside the first release.

## 13. Runtime and performance

### 13.1 Execution backends

Implement a straightforward deterministic CPU reference evaluator first. Add an optimized CPU evaluator using the same typed IR and effect semantics. Keep the reference backend available for small-world differential tests.

Dispatch graph operations per kernel/chunk, not through a virtual rule object for every cell. Reuse preallocated buffers. Keep small vectors or fixed-capacity structures for bounded per-entity requests. Avoid allocating a heap object for each flow on every tick.

Use canonical edge buffers for fixed-grid transport. For entities, use stable IDs with dense component storage and a spatial index. Storage compaction must not redefine identity or iteration order.

### 13.2 Authoritative precision and memory

Use `f64` for initial authoritative numerical state and `f32` for presentation buffers where appropriate. This is a correctness-first choice, not proof that double precision eliminates drift.

For scale intuition, twelve double-buffered `f64` fields require:

```text
512 * 512 * 12 * 8 bytes * 2 = 48 MiB
2048 * 2048 * 12 * 8 bytes * 2 = 768 MiB
```

These are raw field-buffer sizes, not total memory estimates. Parameters, halos, temporary arrays, transport requests, entities, genomes, snapshots, and rendering add overhead.

The number of graph nodes is not the number of CPU operations per cell. Neighbor gathers, memory traffic, sorting, resolution, and neural inference must be profiled separately.

### 13.3 Threading and snapshots

The runtime may use a dedicated simulation worker and parallel chunk jobs. The client consumes a bounded queue of immutable presentation snapshots. When the client falls behind, it may discard obsolete presentation snapshots, not simulation ticks or authoritative events.

Do not deep-copy the complete authoritative world every render frame. Publish selected render fields, entity transforms, changed chunks, and requested inspection data. Detailed history is opt-in and bounded.

Headless mode must load the same scenario and rule packages, execute the same runtime, and emit metrics without initializing the renderer.

### 13.4 Acceleration order

After correctness gates pass, measure and improve contiguous data access, allocations, kernel fusion, temporary reuse, chunk parallelism, batched brain evaluation, static caching, and only then more complex backend work.

A future compute backend can investigate `wgpu`, which provides a Rust graphics API with cross-platform backends. [R7] Port only suitable kernels after defining precision, reduction, readback, and synchronization costs. A GPU backend is not automatically faster for sparse lifecycle work, and it does not inherit CPU bitwise reproducibility.

No GPU simulation, JIT compiler, or adaptive-resolution world is required for the first playable release.

## 14. Numerical safety and determinism

### 14.1 Validity rules

Every compiled rule must declare or infer bounded input requirements. Checked runtime operations reject nonfinite outputs, invalid divisions, negative requested resource amounts, invalid capacities, and out-of-bounds IDs.

Declared state ranges produce diagnostics, not arbitrary silent clamping. Clamping is a lawful behavior only when represented explicitly in the graph, or when used for the documented bounded roundoff correction policy.

A compile-time type check cannot establish that a nonlinear coupled world is stable. The editor distinguishes type errors, numeric risks, and observed instability.

### 14.2 Numerical stability

Ship parameter limits for standard transport and diffusion operators. Verify isolated processes before combining them. Compare trajectories at the configured timestep and at half that timestep over equal simulated durations.

Positivity, conservation, and stability are different properties. A donor limiter can preserve nonnegative water while an oversized timestep still produces unrealistic oscillations.

The first release does not silently adapt the timestep. Unsupported coefficients or observed failures pause the simulation with a diagnostic. A later adaptive solver must define error estimation, global time synchronization, deterministic decisions, and cost limits.

### 14.3 Replay contract

The first contract is exact reproducibility for the same engine build and supported execution profile on the same supported platform, with identical scenario, seed, packages, and command log. One-thread and multi-thread runs must agree under that profile.

Use stable traversal keys, fixed logical reduction partitions independent of worker count, a fixed reduction tree, and versioned random operations. Hash maps may be used for lookup but must not define authoritative accumulation or conflict order.

Do not claim cross-platform, CPU/GPU, or cross-version bitwise replay. Numeric libraries, compiler behavior, instruction choices, and backend precision need separate compatibility verification. Store build/profile identifiers so incompatible replays are rejected clearly rather than silently accepted.

### 14.4 Authoritative and diagnostic calculations

Presentation interpolation, labels, and optional telemetry cannot feed back into the authoritative simulation unless they are explicitly promoted to rule inputs with a declared stage and schema.

An inspector recomputing a derived value must use the selected tick's snapshot, rules, and random keys, not current wall-clock values. Debugging must not change the world it is explaining.

## 15. Rule editor and debugging

### 15.1 Minimum editor

Provide node creation, deletion, connection, parameter editing, grouping, duplication, undo/redo, named outputs, save/load, and compile diagnostics. Port labels show quantity, unit, domain, and temporal stage. Invalid connections are explained at the point of interaction.

Offer collapsed templates for rainfall, evaporation, flow, vegetation, metabolism, and reproduction. Players can expand a template to see its actual graph. Templates are not privileged hardcoded simulation systems.

The player must be able to build a small law from primitive nodes, scope it to a region, and apply it without editing Rust.

### 15.2 Applying edits

Draft edits do not change a running experiment. Validation produces a summary of affected state, introduced sources/sinks, expected computational cost, and numerical warnings. The player applies a valid draft at a tick boundary.

Invalid drafts preserve the active law version. Undoing an editor change does not rewind an already simulated world. Restoring a past world requires a checkpoint branch or replay.

### 15.3 Explain-this-value inspection

Selecting a cell shows its inventories, derived values, recent accepted and rejected transfers, relevant region masks, contributing rules, and any limiting capacities.

For a water change, show a ledger such as:

```text
Surface water, previous tick: 12.0 kg
Rain accepted:               +3.0 kg
Inflow accepted:             +1.0 kg
Outflow accepted:            -5.0 kg
Evaporation accepted:        -2.0 kg
Drinking accepted:           -1.0 kg
Surface water, this tick:     8.0 kg
```

A process inspector additionally shows requested amount, accepted amount, limiting reservoir/capacity, and source graph nodes. Keep detailed provenance for selected cells/entities and a bounded recent history rather than recording every node for the entire world forever.

### 15.4 Organism and experiment views

Provide phenotype/genome summaries, current needs, sensed inputs, attempted versus actual actions, parentage, offspring count, death reasons, population history, and trait-distribution charts.

A branch comparison shows rules changed, elapsed simulated duration, resource budgets, population outcomes, and seed. It must distinguish authored initial traits from later inherited changes.

## 16. Rendering and presentation

Use Bevy as the initial client framework for 3D presentation and UI integration; its documented scope includes 2D/3D rendering and an ECS. [R1] The simulation remains an ordinary Rust library, not a set of authoritative Bevy-rendering systems.

Build terrain meshes from elevation and a visual water surface from water depth. This does not create a full volumetric water simulation. Organisms use 2D simulation positions and sample terrain height for presentation. Camera orientation, animation, and mesh level of detail are presentation-only.

Render vegetation through density/material changes and bounded instances. Use continuous visual blending based on fields rather than swapping entire terrain behavior when a label changes. Optional map labels can use hysteresis or time smoothing to avoid flicker, without affecting rules.

Support a top-down diagnostic view and orbiting/zooming 3D view. Include readable overlays for temperature, surface-water depth, soil moisture, vegetation, nutrients, regional scope, and flow direction. Labels and numeric inspection must complement color rather than relying on color alone.

The simulation clock must be independent of frame rate. Bevy provides fixed-time scheduling facilities, but this project retains ownership of the integration quantum and headless clock. [R2]

## 17. Persistence, replay, and rule changes

### 17.1 Save contents

A save includes format version, engine/build profile, integer tick, time quantum, world/grid configuration, authoritative fields, organism state, genomes and lineage data, stable-ID counters, random algorithm/stream state as applicable, pending commands, active rule packages and content hashes, parameters, schema versions, region masks, and external-account ledger totals.

Compiled execution plans are disposable caches, not the only saved representation of a world's laws. Store canonical source graphs so plans can be reconstructed under a compatible engine.

Serialize with explicit schema versions and migrations; Serde supplies Rust serialization/deserialization infrastructure but does not provide the game's migration policy. [R6]

### 17.2 Replay and branching

Record commands with stable sequence IDs and the simulation tick at which they apply. Periodic checkpoints bound replay cost. A branch references a checkpoint and a different subsequent command stream.

Support pause/resume and save/load equivalence first. Full arbitrary timeline scrubbing is not necessary for the first release. A checkpoint-based “run the same world with this changed law” workflow is sufficient.

### 17.3 Rule hot-swap

Compile a candidate rule set without modifying active state. Preserve stable field/node IDs where meanings remain compatible. Validate numerical limits and required capabilities before activation.

Parameter and graph edits apply atomically at a tick boundary. New fields require defaults for existing cells/entities. Removing populated conserved reservoirs requires a declared transfer or explicit external sink. Incompatible domain or unit changes require a validated migration or a new experiment.

If validation fails, keep both the old active law and committed world unchanged. The client displays the failure in the draft editor.

## 18. Rule-package security

Treat imported graphs and saves as untrusted data. Use a bounded language with no filesystem, network, process execution, arbitrary pointers, native dynamic libraries, or unrestricted host calls.

Validate graph size, expansion size, nesting, field count, spatial radius, entity query limits, transient memory estimates, numeric literal size/range, and version/capability requirements before activation. Runtime instruction/effect budgets prevent a valid-looking graph from generating unbounded work.

A node's cost depends on its domain cardinality and spatial footprint; 100 nodes over a million cells is not the same budget as 100 uniform nodes. Query operators must expose overflow behavior. Resource-changing queries cannot silently drop excess targets without a declared selection policy.

Imported packages cannot override the resolver, determinism profile, budget checks, or migration rules. Explicit external-resource effects are allowed only through their declared sandbox capabilities.

A budget violation pauses the experiment transactionally. Do not randomly skip rules to preserve frame rate. The client can continue rendering the last valid snapshot while presenting the error.

## 19. Rust workspace and dependency policy

### 19.1 Proposed module boundaries

Start with a small workspace; modules can become separate crates only when the boundary is useful.

```text
crates/
  sim_core/
    schema/          Quantity, unit, domain, state, stable IDs
    graph/           Source graph, typed IR, validation
    compiler/        Analysis, optimization, execution plans
    runtime/         Fields, ticks, effects, resolver, lifecycle
    biology/         Genome storage, inheritance, brain interface
    persistence/     Saves, replay commands, migrations
    diagnostics/     Budgets, provenance, state hashes
  sim_cli/
    main.rs          Headless execution and experiment metrics
  game_client/
    main.rs          Bevy integration
    editor/          Graph editor and world tools
    presentation/    Terrain, organisms, overlays, cameras
assets/
  rules/core/        Standard environmental and biological graphs
  scenarios/        Seeded fixtures and guided scenarios
tests/
  fixtures/         Small worlds and invalid/valid graph packages
benches/
  manifests/        Reproducible benchmark scenarios
```

The dependency direction is `game_client -> sim_core` and `sim_cli -> sim_core`. `sim_core` must not import the game client or require Bevy. Rule assets are authored data; trusted capability implementations live in the runtime.

### 19.2 Dependency choices

| Dependency | Intended use | Restriction |
|---|---|---|
| Rust standard library | Core data structures and reference evaluator | No requirement for a heavyweight ML framework. |
| Bevy | Rendering, cameras, client-side interaction | Not the source of simulation semantics. [R1] |
| Serde and a JSON format implementation | Graph documents, manifests, save metadata | Explicit game schemas and migrations remain required. [R6] |
| Rayon | Parallel scheduling for CPU work | Authoritative reduction order is defined by the engine. [R3] |
| `uom`, optional | Developer-side unit helpers/reference checks | Player graph units still need runtime schema validation. [R4] |
| `wgpu`, deferred | Evaluate a later compute backend | Not required for initial authoritative simulation. [R7] |

Pin a compatible toolchain and dependency set when creating the repository, commit the application lockfile, and record the build fingerprint in benchmark/replay metadata. Do not treat the word `latest` in an online documentation URL as a reproducible build specification.

A generic evolution library is not a required dependency. The initial algorithm is local reproduction plus mutation; evaluate external libraries later behind the brain/genome interface rather than letting a generation-based optimizer own the world lifecycle.

## 20. Initial configuration

These are proposed starting values for implementation and benchmarking. They are not measured performance claims or calibrated ecological parameters.

| Setting | Baseline |
|---|---|
| Interactive world | 256 x 256 cells. |
| Cell spacing | 10 meters on each horizontal axis. |
| Chunk size | 32 x 32 interior cells; halo width follows the compiled stencil. |
| Integration timestep | 0.25 simulated seconds. |
| Authoritative precision | `f64`. |
| Initial organism count | 500 in the guided scenario; 2,000 in the medium benchmark. |
| Default population cap | 10,000; cap events are visible in metrics. |
| Brain | 12 inputs, 16 hidden units, 6 outputs; fixed topology. |
| Mutation | Per-scalar probability 0.02; bounded perturbation scaled to each trait's declared range. |
| Default flow boundary | Closed. |
| Weather | Prescribed seeded profiles and region masks; explicit external water accounting. |
| Presentation | Top-down diagnostics first, then 3D height-field terrain. |
| Speed controls | Pause, single-step, 1x, 10x, and fastest possible. |

Mutation distributions and neural activation functions must have a versioned deterministic implementation. The initial package uses bounded initialization and mutation; no rule may produce invalid capacities or neural parameter counts.

At seed creation, record whether an organism came from random initialization, an authored starter genome, or a previous evolved lineage. Use a simple authored forager only as a labeled control fixture when debugging ecology; do not present it as an evolved result.

### 20.1 Default authoring limits

Start with a maximum of 4,096 nodes per expanded world-law graph, 64 registered cell-state fields, radius 4 for general neighborhood queries, and 32 candidate neighbors per organism query. Fixed transport stencils use their separately verified bounds.

These are initial product limits, not mathematical necessities. A package's per-tick memory/effect estimate must also fit the world's configured budget. Larger limits require benchmark evidence; reducing them cannot silently alter an already loaded experiment.

### 20.2 Numeric diagnostics

For standard water and nutrient test fixtures, begin with a per-tick ledger tolerance:

```text
allowed_error = 1e-9 kg + 1e-12 * max(previous_total, total_absolute_exchange)
```

For the 100,000-tick conservation soak fixture, use a cumulative gate of `1e-6 kg + 1e-8 * reference_mass_scale`. Define that scale in the fixture manifest from initial inventory and accumulated absolute external exchanges. Log the actual error, not only pass/fail.

These tolerances are acceptance targets to validate during implementation. Any revision must document the observed error source; do not broaden tolerances merely to hide a conservation defect. New quantities declare their own physically meaningful absolute tolerances.

## 21. Implementation milestones

Implement in the order below. Each milestone has a runnable outcome and a correctness gate. Do not start a GPU backend or full climate system to compensate for an unverified CPU model.

### M0 — Semantic foundation

**Build:** a minimal Rust library, headless test harness, stable IDs, field schemas, units/domains, fixed clock, source graph format, and canonical serialization.

**Gate:** valid/invalid graph fixtures demonstrate quantity, domain, and stage checking. Identical manifests produce identical initialized states. `sim_core` builds and runs without Bevy.

### M1 — Reference rule executor

**Build:** snapshot reads, pure operators, cycle detection, a stable execution plan, rate integration, transactional candidate buffers, and one-cell/two-cell examples.

**Gate:** changing rule registration order does not change a result; explicit feedback works across ticks; algebraic cycles fail with useful diagnostics; failed ticks leave committed state untouched.

### M2 — Resource-safe hydrology

**Build:** the shared transfer/reaction resolver, ledger, rainfall, evaporation, infiltration, groundwater release, edge-based surface transport, boundary policies, and region scopes.

**Gate:** oversubscription and capacity fixtures pass; cross-chunk results match unchunked results; closed-world mass is conserved; open boundaries produce matching ledger entries; head-based basin tests behave correctly.

### M3 — Living landscape

**Build:** temperature forcing, bounded diffusion, vegetation growth, plant nutrient accounting, mortality, decomposition, and dispersal in the same rule representation. Add field inspection and a top-down client.

**Gate:** a seeded drought-and-rain scenario changes water and vegetation without a biome switch. Plants do not grow from missing inputs, and disappeared vegetation does not reappear without a declared seed/propagule source.

### M4 — Evolving organisms

**Build:** spatial indexing, observations, the fixed neural evaluator, physiology/action rules, local births, mutation, lifecycle accounting, lineages, and population metrics.

**Gate:** resource-funded births/deaths conserve tracked inventories; worker-count changes do not alter the run; initial trait variants show different survival/reproduction under a controlled stress fixture; the interface distinguishes inherited traits from transient condition.

### M5 — Player law editor

**Build:** primitive node editing, templates, port type feedback, region masks, custom fields, compile/apply, undo/redo, cost display, and accepted-versus-requested inspection.

**Gate:** a player can create a new regional law from nodes, alter a formula rather than only a preset value, save it, reload it, and observe a documented effect. Invalid edits leave the active experiment untouched.

### M6 — Complete playable sandbox

**Build:** 3D terrain/water presentation, organism visualization, continuous landscape blending, the guided river scenario, save/load, checkpoint branching, experiment comparison, and basic onboarding.

**Gate:** the complete player loop works without source-code edits. Rendering and headless runs agree. A saved-and-resumed run matches uninterrupted execution under the replay contract.

### M7 — Profiled CPU optimization and hardening

**Build:** compatible kernel fusion, temporary reuse, chunk parallelism, batched brains, invariant caching, bounded provenance storage, package fuzzing, and benchmark reporting.

**Gate:** reference-versus-optimized differential tests pass, required numerical contracts remain intact, and published benchmark manifests meet the declared product targets or document a scope reduction.

A milestone is complete only when its gates run in the repository. This document defines those gates; it does not report that they have already passed.

## 22. Verification and acceptance tests

### 22.1 Compiler and semantic tests

| ID | Fixture and required result |
|---|---|
| T01 | Reject adding water to temperature; identify the offending ports. |
| T02 | Reject mixing cell/entity domains or unrelated grid identities without an explicit operator. |
| T03 | Reject a pure algebraic cycle; accept explicit initialized feedback. |
| T04 | Reject two exclusive next-value writers; allow explicitly combined rate contributions. |
| T05 | Reject negative resource requests and undeclared external creation. |
| T06 | Preserve separate random streams and duplicate effect instances during optimization. |
| T07 | Reject a same-tick feedback edge from an accepted receipt into an earlier request. |
| T08 | Reject graphs exceeding expanded-node, spatial, memory, or effect budgets. |

### 22.2 Resource and world tests

| ID | Fixture and required result |
|---|---|
| T09 | A 10 kg source with two 8 kg requests accepts 5 kg each; destination gains total 10 kg. |
| T10 | A multi-input process scales every input/output by its limiting factor. |
| T11 | Two inflows to a nearly full reservoir cannot exceed its capacity; rejected material remains at the sources. |
| T12 | Rain and evaporation exchange exactly their accepted amounts with named external accounts, within numeric tolerance. |
| T13 | Equal hydraulic heads produce zero transport request, even with different terrain elevations. |
| T14 | Chunk-boundary transport matches the same world stored without chunk boundaries. |
| T15 | A seeded closed-water fixture passes 100,000 ticks without unexplained budget drift or material negative inventories. |
| T16 | A smooth diffusion/flow fixture improves toward a finer-step reference when the timestep is halved; compare equal simulated duration. |
| T17 | Region overlap follows explicit addition/blending, not editor insertion order. |
| T18 | Cross-region transport debits and credits both actual endpoints. |

### 22.3 Organism, replay, and product tests

| ID | Fixture and required result |
|---|---|
| T19 | Several organisms drinking from one cell share the same limited inventory with environmental consumers. |
| T20 | A partially funded movement request produces only the allowed motion; the organism cannot eat at both endpoints in one tick. |
| T21 | Birth transfers parent resources atomically; a rejected birth costs nothing; newborns first act next tick. |
| T22 | Death disposes of all tracked water/nutrients, and a dying parent does not reproduce in the same lifecycle phase. |
| T23 | Mutation preserves genome validity, changes some inherited values across a sufficiently large fixture, and leaves parent genomes unchanged. |
| T24 | Controlled hot/dry and cool/wet fixtures expose the specified trait trade-offs; measure multiple seeds rather than require a universal winner. |
| T25 | Same-build runs with 1, 2, and 8 workers produce identical authoritative state hashes. |
| T26 | Save/load continuation equals uninterrupted execution; incompatible build/profile metadata is reported. |
| T27 | Reference and optimized executors agree under the selected numerical contract on generated small valid graphs. |
| T28 | Invalid graph hot-swap or failed tick leaves the old rules and world intact. |
| T29 | Camera changes, overlays, inspection, and rendering disabled do not change authoritative state. |
| T30 | A player-authored rule survives graph save/load and produces the same seeded results. |
| T31 | The guided river scenario exhibits drying and recovery through budgets, without authoritative biome transitions. |
| T32 | An imported malformed graph/save is rejected without host file/network access, panic, or unbounded allocation. |

### 22.4 Test methods

Use unit tests for quantity/operator rules; property-based tests for bounded small graphs and transfer networks; golden fixtures for diagnostics; differential tests between executors; fuzzing of graph/save loaders; and long-running conservation and stability tests.

Keep structural/lifecycle tests separate from smooth numerical convergence tests. Changing timestep can move discrete thresholds and alter ecological trajectories; a pointwise equality test is not appropriate for every long evolutionary run.

Use regression fixtures with known seeds and explicit laws. Record the seed, graph hash, numerical profile, compiler version, and minimized input for every failure.

## 23. Performance gates

### 23.1 Benchmark manifests

| Manifest | World | Organisms | Purpose |
|---|---:|---:|---|
| `tiny-reference` | 32 x 32 | 0–100 | Exact diagnostics and differential tests. |
| `hydrology-medium` | 256 x 256 | 0 | Transport, resolution, and water budgets. |
| `ecosystem-medium` | 256 x 256 | 2,000 | Standard-pack gameplay workload. |
| `ecosystem-large` | 512 x 512 | 10,000 | Scaling, memory, and population-pressure study. |

Each manifest pins the seed, graph/package hashes, parameter set, numerical profile, initial populations, warm-up count, measured tick count, and hardware/build details. Use 100 warm-up ticks followed by at least 1,000 measured ticks for routine reports; preserve longer soak tests separately.

Report median and p95 tick time, maximum tick time, peak resident memory, compile/hot-swap latency, accepted-process throughput, neural evaluation time, resolver time, snapshot cost, and frame time separately.

### 23.2 Initial product targets

On one documented reference desktop chosen during M0, target `ecosystem-medium` at p95 no more than 25 ms per integration tick and less than 1 GiB simulation resident memory. With the baseline timestep, 40 ticks per wall-clock second corresponds to 10x simulated speed.

For a typical 500-node law graph, target graph validation/planning within 250 ms, excluding shader/asset compilation. Target a 60-frame-per-second 1080p presentation under the published visual quality preset, independently of whether maximum simulation speed is reached.

These are engineering targets, not evidence of current capability. Large-world performance is a scaling study, not an initial release guarantee. Results must name the hardware and workload; “millions of tiles” without the rules and entities is not a useful benchmark.

### 23.3 Performance failure policy

Profile before changing architecture. First reduce avoidable allocations, data movement, redundant evaluation, and unbounded diagnostics. Then reassess default world/population limits or quality settings through an explicit product decision.

Never meet a benchmark by dropping resource checks, freezing offscreen organisms, changing the timestep without disclosure, or skipping active rules. The UI may lower presentation update frequency without changing simulation time.

## 24. Risks and deferred features

| Risk | Mitigation / decision |
|---|---|
| A universal language delays the actual game | Implement the smallest shared operator/effect set that expresses the first environmental and organism packs. |
| Correct types create a false sense of physical correctness | Keep numerical validation, budget tests, and explicit toy-model limits. |
| Rule execution order leaks into outcomes | Snapshot reads, fixed semantic phases, canonical reductions, and shared resource resolution. |
| Emergence is hard to understand | Requested/accepted flow inspection, provenance, labeled controls, and experiment branching. |
| All organisms die or one lineage dominates | Make extinction visible; use controlled fixtures and configurable trade-offs, not hidden rescue behavior. |
| Optimizations change the world | Maintain the reference executor and profile-specific differential tests. |
| Dynamic graphs overwhelm authoring UX | Collapsible templates, units on ports, bounded queries, useful diagnostics, and regional previews. |
| Saves break as schemas evolve | Stable IDs, explicit migrations, original source graphs, and compatibility checks. |
| Large worlds exceed memory/CPU budgets | Start small, measure representative workloads, and keep rendering decoupled. |

Deferred extensions include recurrent and evolving neural topology; sexual reproduction; predator-prey interactions; plant genomes; layered soil/atmosphere; internally tracked humidity/cloud transport; erosion; snow; fire; diseases; custom native capabilities; GPU kernels; adaptive resolution; and multiplayer.

Each extension must preserve or explicitly version the type system, resource contract, tick semantics, save format, and replay scope. True 3D simulation is not needed to deliver the core game and must not be introduced solely because the renderer is 3D.

## 25. Definition of done

The first playable release is complete when the player can:

1. Create a seeded world whose environmental behavior comes from fields and rules rather than hardcoded biome names.
2. Create, inspect, scope, save, and apply a typed law graph through the game interface.
3. Observe resource-driven terrain and vegetation changes, plus organisms that act, survive, reproduce, and inherit mutations.
4. Explain local changes through accepted resource transactions and organism state.
5. Save, resume, and branch an experiment under the documented replay contract.

The engineering release gate additionally requires passing semantic/resource/lifecycle tests, validated conservation tolerances, reference-versus-optimized checks, bounded import behavior, a renderer-independent runtime, and reproducible performance reports.

Do not call the release complete merely because terrain renders, a generic graph evaluator exists, or a hand-authored neural agent moves. The defining experience is **editing a law and observing an explainable, persistent consequence in a living world**.

## 26. Engineering references

These primary project/documentation sources were checked on 2026-09-29. They support the implementation context and numerical cautions noted in the document; they do not validate this proposed game's performance or ecological outcomes. Versioned dependency resolution remains a repository task.

- **[R1] Bevy official project site:** rendering/ECS scope. <https://bevy.org/>
- **[R2] Bevy Time `Fixed` documentation:** fixed-step scheduling facilities. <https://docs.rs/bevy_time/latest/bevy_time/struct.Fixed.html>
- **[R3] Rayon `ParallelIterator` documentation:** parallel execution and the unspecified reduction order of ordinary floating-point sums. <https://docs.rs/rayon/latest/rayon/iter/trait.ParallelIterator.html#method.sum>
- **[R4] `uom` thermodynamic temperature documentation:** absolute temperature and temperature-interval distinction. <https://docs.rs/uom/latest/uom/si/thermodynamic_temperature/index.html>
- **[R5] LLVM Language Reference, fast-math flags:** reassociation, contraction, and other permitted floating-point transformations. <https://llvm.org/docs/LangRef.html#fast-math-flags>
- **[R6] Serde official overview:** Rust serialization/deserialization framework. <https://serde.rs/>
- **[R7] `wgpu` official repository:** Rust graphics API and supported backend context. <https://github.com/gfx-rs/wgpu>
- **[R8] Neataptic original repository:** flexible neural-network/neuroevolution inspiration and project maintenance notice. <https://github.com/wagenaartje/neataptic>

---

**Development priority:** establish the meaning of one tick, prove the shared resource resolver, make the first ecosystem run headlessly, and then expose those same laws to the player. Optimize the implementation without quietly changing the universe it computes.
