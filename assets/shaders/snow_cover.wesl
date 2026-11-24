// One look of settled snow. Every look's materials draw the same mesh, whose second UV
// holds when snow started settling at each vertex, and each keeps only the spots where
// that long ago means its own look, or, fading in, the look before it. So the snow
// comes in without rebuilding any mesh. Each pixel of a look's texture comes in as one,
// timed by when snow started at its middle.

import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
};

struct SnowLook {
    /// World units one pixel of the look's texture covers, 0 without a texture.
    texel: vec2<f32>,
    /// Which look this is, 1 for the first partial one.
    look: f32,
    /// Seconds since play started.
    clock: f32,
    /// Seconds each look lasts.
    stage_seconds: f32,
    /// The last look, which stays.
    full_look: f32,
    /// 1 when this draws the look fading in over the one before it.
    fading_in: f32,
    /// Unused: rounds the struct up to a multiple of 16 bytes, which WebGL2 needs.
    padding: f32,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> snow_look: SnowLook;

/// When snow started settling at the middle of the texture pixel world `xz` is in,
/// from `started` there. It changes evenly over each triangle, so how it changes across
/// the screen tells how it changes over the ground.
fn started_at_texel_middle(xz: vec2<f32>, started: f32) -> f32 {
    let across = mat2x2<f32>(dpdx(xz), dpdy(xz));
    let det = determinant(across);
    let step = vec2(dpdx(started), dpdy(started));
    // Solves step = transpose(across) * slope for the slope over the ground.
    let slope = vec2(
        across[1].y * step.x - across[0].y * step.y,
        -across[1].x * step.x + across[0].x * step.y
    ) / select(det, 1.0, det == 0.0);
    let texel = max(snow_look.texel, vec2(1e-6));
    let middle = (floor(xz / texel) + 0.5) * texel;
    let usable = all(snow_look.texel > vec2(0.0)) && abs(det) > 1e-12;
    return select(started, started + dot(slope, middle - xz), usable);
}

/// How opaque this look is where snow started settling at `started`: fully once it has
/// come in, partly while fading in through the look before it, and dropped otherwise.
fn own_look_opacity(started: f32) -> f32 {
    let age = (snow_look.clock - started) / snow_look.stage_seconds;
    let look = clamp(floor(age), 0.0, snow_look.full_look);
    var opacity = 1.0;
    if snow_look.fading_in > 0.5 {
        if look != snow_look.look - 1.0 {
            discard;
        }
        opacity = clamp(age - look, 0.0, 1.0);
    } else if look != snow_look.look {
        discard;
    }
    return opacity;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    let opacity = own_look_opacity(started_at_texel_middle(in.uv, in.uv_b.x));
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color
    );
    // Fading in, a texture's holes stay sharp and the rest gets more opaque.
    if snow_look.fading_in > 0.5 {
        pbr_input.material.base_color.a =
            select(0.0, opacity, pbr_input.material.base_color.a >= 0.5);
    }
    var out: FragmentOutput;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
