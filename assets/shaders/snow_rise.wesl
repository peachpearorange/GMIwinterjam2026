// The rising snow in the main pass. Every vertex is moved up by how high all the snow
// has come, and held where the snow is as deep as it gets, lying along the ground
// there. So the snow rises and stops smoothly without rebuilding any mesh.

import bevy_pbr::render::{
    forward_io::VertexOutput,
    mesh_functions,
    view_transformations::position_world_to_clip,
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
    let local = vec3(in.position.x, min(risen, in.deepest.w), in.position.z);
    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    var out: VertexOutput;
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4(local, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(
        select(in.normal, in.deepest.xyz, held),
        in.instance_index
    );
@if(VERTEX_UVS_A)
    out.uv = in.uv;
@if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = in.instance_index;
@if(VISIBILITY_RANGE_DITHER)
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        in.instance_index, world_from_local[3]);
    return out;
}
