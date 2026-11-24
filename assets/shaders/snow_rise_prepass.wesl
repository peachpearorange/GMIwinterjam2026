// The rising snow in the prepass and shadows, moved up and held at its deepest the
// same way as in snow_rise.wesl.

import bevy_pbr::{
    prepass::io::VertexOutput,
    render::{mesh_functions, view_transformations::position_world_to_clip},
};

struct RisingSnow {
    /// How high all the rising snow has come.
    level: f32,
    /// Unused: rounds the struct up to a multiple of 16 bytes, which WebGL2 needs.
    padding_1: f32,
    padding_2: f32,
    padding_3: f32,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> rising: RisingSnow;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    /// Where the snow would be had it risen from the ground at all.
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    /// The ground's direction, and how high the snow gets at most.
    @location(8) deepest: vec4<f32>,
}

@vertex
fn vertex(in: Vertex) -> VertexOutput {
    let risen = in.position.y + rising.level;
    let held = risen >= in.deepest.w;
    let local = vec4(in.position.x, min(risen, in.deepest.w), in.position.z, 1.0);
    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    var out: VertexOutput;
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, local);
    out.position = position_world_to_clip(out.world_position.xyz);
@if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
    out.unclipped_depth = out.position.z;
    out.position.z = min(out.position.z, 1.0);
}
@if(VERTEX_UVS_A)
    out.uv = in.uv;
@if(NORMAL_PREPASS_OR_DEFERRED_PREPASS)
    out.world_normal = mesh_functions::mesh_normal_local_to_world(
        select(in.normal, in.deepest.xyz, held),
        in.instance_index
    );
@if(MOTION_VECTOR_PREPASS)
    out.previous_world_position = mesh_functions::mesh_position_local_to_world(
        mesh_functions::get_previous_world_from_local(in.instance_index),
        local
    );
@if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = in.instance_index;
@if(VISIBILITY_RANGE_DITHER)
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        in.instance_index, world_from_local[3]);
    return out;
}
