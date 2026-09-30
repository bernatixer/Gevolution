// Terrain: blends ground textures per vertex from simulation state.
// Vertex color = (lush grass, dry grass, soil, rock); uv_b = (sand/mud, forest floor).

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
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

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var layers: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var layers_sampler: sampler;

// Two scales blended to hide tiling.
fn tex(i: i32, uv: vec2<f32>) -> vec3<f32> {
    let a = textureSample(layers, layers_sampler, uv, i).rgb;
    let b = textureSample(layers, layers_sampler, uv * 0.23 + vec2<f32>(0.37, 0.11), i).rgb;
    return mix(a, b, 0.45);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    let uv = in.world_position.xz / 6.0;
    var w0 = vec4<f32>(1.0, 0.0, 0.0, 0.0);
    var w1 = vec2<f32>(0.0, 0.0);
#ifdef VERTEX_COLORS
    w0 = in.color;
#endif
#ifdef VERTEX_UVS_B
    w1 = in.uv_b;
#endif
    let grass = tex(0, uv) * vec3<f32>(0.86, 1.04, 0.78);
    let forest = tex(1, uv);
    let soil = tex(2, uv);
    let dry = tex(3, uv);
    let rock = tex(4, uv * 0.6);
    let sand = tex(5, uv);
    // Height-aware blending: brighter texels win at transitions for a natural edge.
    let straw = mix(grass * vec3<f32>(1.35, 1.08, 0.52), dry, 0.45);
    var c = grass * w0.r + straw * w0.g + soil * w0.b + rock * w0.a + sand * w1.x + forest * w1.y;
    let total = w0.r + w0.g + w0.b + w0.a + w1.x + w1.y;
    c = c / max(total, 0.001);
    // Large-scale tonal variation so open ground doesn't read as one flat color.
    let q = in.world_position.xz;
    let macro_n = 0.5 + 0.25 * sin(q.x * 0.011 + sin(q.y * 0.007) * 2.0) + 0.25 * sin(q.y * 0.013 + sin(q.x * 0.009) * 2.0);
    c = c * mix(vec3<f32>(0.82, 0.86, 0.80), vec3<f32>(1.12, 1.06, 0.92), macro_n);

    pbr_input.material.base_color = vec4<f32>(c, 1.0);
    pbr_input.material.perceptual_roughness = 0.92;
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
