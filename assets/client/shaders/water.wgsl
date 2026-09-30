// Water: gentle animated ripples on top of the standard PBR surface.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
    mesh_view_bindings::globals,
}

#ifdef PREPASS_PIPELINE
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput},
    pbr_deferred_functions::deferred_output,
}
#else
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}
#endif

struct WaterParams {
    strength: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> water: WaterParams;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let t = globals.time;
    let p = in.world_position.xz;
    let dx = sin(p.x * 0.09 + t * 1.3) + 0.6 * sin(p.y * 0.21 - t * 1.7) + 0.35 * sin((p.x + p.y) * 0.37 + t * 2.3);
    let dz = cos(p.y * 0.08 + t * 1.1) + 0.6 * cos(p.x * 0.19 + t * 1.5) + 0.35 * cos((p.x - p.y) * 0.41 - t * 2.1);
    let n = normalize(pbr_input.N + vec3<f32>(dx, 0.0, dz) * water.strength);
    pbr_input.N = n;
    pbr_input.world_normal = n;
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

#ifdef PREPASS_PIPELINE
    let out = deferred_output(in, pbr_input);
#else
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
#endif
    return out;
}
