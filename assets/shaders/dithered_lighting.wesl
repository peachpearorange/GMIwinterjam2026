// Dithered lighting over the finished picture: the screen in big square pixels, each
// one's brightness snapped to a few steps, with a pattern of dots between one step and
// the next. Colors keep their hue, only brightness is stepped.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct DitheredLighting {
    /// How many big pixels tall the screen is.
    rows: f32,
    /// How many steps of brightness there are, black and white included.
    levels: f32,
    /// The dot pattern is 2^pattern_bits big pixels along each side.
    pattern_bits: u32,
    /// 0 for hard steps without dots, 1 for dots all the way between steps.
    strength: f32,
}

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var screen_sampler: sampler;
@group(0) @binding(2) var<uniform> settings: DitheredLighting;

/// Roughly how bright a linear brightness looks, and back.
const DISPLAY_GAMMA: f32 = 2.2;

/// The ordered (Bayer) pattern's threshold for big pixel `cell`, inside 0..1. Each bit
/// of the coordinates picks a quarter of a quarter, from the coarsest down.
fn pattern_threshold(cell: vec2<u32>) -> f32 {
    var rank = 0u;
    for (var bit = 0u; bit < settings.pattern_bits; bit++) {
        let on = (cell >> vec2(bit)) & vec2(1u);
        rank = (rank << 2u) | ((on.x ^ on.y) << 1u) | on.y;
    }
    return (f32(rank) + 0.5) / f32(1u << (2u * settings.pattern_bits));
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let screen = vec2<f32>(textureDimensions(screen_texture));
    // Whole screen pixels, so every big pixel is as big as the others.
    let cell_size = max(round(screen.y / max(settings.rows, 1.0)), 1.0);
    let cell = floor(in.position.xy / cell_size);
    let color =
        textureSampleLevel(screen_texture, screen_sampler, (cell + 0.5) * cell_size / screen, 0.0);

    let luminance = dot(color.rgb, vec3(0.2126, 0.7152, 0.0722));
    let steps = max(settings.levels, 2.0) - 1.0;
    // Stepped by how bright it looks, so the steps look evenly apart.
    let brightness = pow(clamp(luminance, 0.0, 1.0), 1.0 / DISPLAY_GAMMA) * steps;
    let below = floor(brightness);
    let threshold = mix(0.5, pattern_threshold(vec2<u32>(cell)), settings.strength);
    let stepped = (below + step(threshold, brightness - below)) / steps;
    let scale = pow(stepped, DISPLAY_GAMMA) / max(luminance, 0.00001);
    return vec4(color.rgb * scale, color.a);
}
