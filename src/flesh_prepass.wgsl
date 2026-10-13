// **The flesh prepass: Bevy 0.19's `pbr_prepass.wgsl`, plus the holes in a cloth.** Copied from
// `bevy_pbr-0.19.0/src/render/pbr_prepass.wgsl` unchanged but for `hole_discard`, which cuts a
// cloth's holes (see `ClothHoles` in `src/preset.rs`) out of the depth prepass and the shadow maps
// as `flesh.wgsl` cuts them out of the colour. Bevy runs a material's prepass fragment shader for a
// depth or shadow pass only when the material may discard, which a cloth does once its first hole
// has switched it to `AlphaMode::Mask`; a normal or motion-vector prepass runs it for every flesh
// material, and for those it is Bevy's own. Re-copy it when the pin moves.

#import bevy_pbr::{
    pbr_prepass_functions,
    pbr_bindings,
    pbr_bindings::material,
    pbr_types,
    pbr_functions,
    pbr_functions::SampleBias,
    prepass_io,
    mesh_bindings::mesh,
    mesh_view_bindings::view,
}

#import bevy_render::bindless::{bindless_samplers_filtering, bindless_textures_2d}

#ifdef MESHLET_MESH_MATERIAL_PASS
#import bevy_pbr::meshlet_visibility_buffer_resolve::resolve_vertex_output
#endif

#ifdef BINDLESS
#import bevy_pbr::pbr_bindings::material_indices
#endif  // BINDLESS

struct FleshParams {
    bands: vec4<f32>,
    wet: vec4<f32>,
    sss: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> flesh: FleshParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var flesh_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var dermis_map: texture_2d<f32>;

const MODE_CLOTH: f32 = 2.0;
const FLAG_DERMIS: u32 = 4u;

// A cloth's hole is where its dermis slot's alpha is over one half; see `flesh.wgsl`.
fn hole_discard(in: prepass_io::VertexOutput) {
#ifdef VERTEX_UVS_A
    if flesh.sss.z == MODE_CLOTH && (u32(flesh.sss.w) & FLAG_DERMIS) != 0u {
        if textureSampleLevel(dermis_map, flesh_sampler, in.uv, 0.0).a > 0.5 {
            discard;
        }
    }
#endif
}

#ifdef PREPASS_FRAGMENT
@fragment
fn fragment(
#ifdef MESHLET_MESH_MATERIAL_PASS
    @builtin(position) frag_coord: vec4<f32>,
#else
    in: prepass_io::VertexOutput,
    @builtin(front_facing) is_front: bool,
#endif
) -> prepass_io::FragmentOutput {
#ifdef MESHLET_MESH_MATERIAL_PASS
    let in = resolve_vertex_output(frag_coord);
    let is_front = true;
#else   // MESHLET_MESH_MATERIAL_PASS

#ifdef BINDLESS
    let slot = mesh[in.instance_index].material_and_lightmap_bind_group_slot & 0xffffu;
    let flags = pbr_bindings::material_array[material_indices[slot].material].flags;
    let uv_transform = pbr_bindings::material_array[material_indices[slot].material].uv_transform;
#else   // BINDLESS
    let flags = pbr_bindings::material.flags;
    let uv_transform = pbr_bindings::material.uv_transform;
#endif  // BINDLESS

    // If we're in the crossfade section of a visibility range, conditionally
    // discard the fragment according to the visibility pattern.
#ifdef VISIBILITY_RANGE_DITHER
    pbr_functions::visibility_range_dither(in.position, in.visibility_range_dither);
#endif  // VISIBILITY_RANGE_DITHER

    hole_discard(in);
    pbr_prepass_functions::prepass_alpha_discard(in);
#endif  // MESHLET_MESH_MATERIAL_PASS

    var out: prepass_io::FragmentOutput;

#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.frag_depth = in.unclipped_depth;
#endif // UNCLIPPED_DEPTH_ORTHO_EMULATION

#ifdef NORMAL_PREPASS
    // NOTE: Unlit bit not set means == 0 is true, so the true case is if lit
    if (flags & pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        let double_sided = (flags & pbr_types::STANDARD_MATERIAL_FLAGS_DOUBLE_SIDED_BIT) != 0u;

        let world_normal = pbr_functions::prepare_world_normal(
            in.world_normal,
            double_sided,
            is_front,
        );

        var normal = world_normal;

#ifdef VERTEX_UVS
#ifdef VERTEX_TANGENTS
#ifdef STANDARD_MATERIAL_NORMAL_MAP

// TODO: Transforming UVs mean we need to apply derivative chain rule for meshlet mesh material pass
#ifdef STANDARD_MATERIAL_NORMAL_MAP_UV_B
        let uv = (uv_transform * vec3(in.uv_b, 1.0)).xy;
#else
        let uv = (uv_transform * vec3(in.uv, 1.0)).xy;
#endif

        // Fill in the sample bias so we can sample from textures.
        var bias: SampleBias;
#ifdef MESHLET_MESH_MATERIAL_PASS
        bias.ddx_uv = in.ddx_uv;
        bias.ddy_uv = in.ddy_uv;
#else   // MESHLET_MESH_MATERIAL_PASS
        bias.mip_bias = view.mip_bias;
#endif  // MESHLET_MESH_MATERIAL_PASS

        let Nt =
#ifdef MESHLET_MESH_MATERIAL_PASS
            textureSampleGrad(
#else   // MESHLET_MESH_MATERIAL_PASS
            textureSampleBias(
#endif  // MESHLET_MESH_MATERIAL_PASS
#ifdef BINDLESS
                bindless_textures_2d[material_indices[slot].normal_map_texture],
                bindless_samplers_filtering[material_indices[slot].normal_map_sampler],
#else   // BINDLESS
                pbr_bindings::normal_map_texture,
                pbr_bindings::normal_map_sampler,
#endif  // BINDLESS
                uv,
#ifdef MESHLET_MESH_MATERIAL_PASS
                bias.ddx_uv,
                bias.ddy_uv,
#else   // MESHLET_MESH_MATERIAL_PASS
                bias.mip_bias,
#endif  // MESHLET_MESH_MATERIAL_PASS
            ).rgb;
        let TBN = pbr_functions::calculate_tbn_mikktspace(normal, in.world_tangent);

        normal = pbr_functions::apply_normal_mapping(
            flags,
            TBN,
            double_sided,
            is_front,
            Nt,
        );

#endif  // STANDARD_MATERIAL_NORMAL_MAP
#endif  // VERTEX_TANGENTS
#endif  // VERTEX_UVS

        out.normal = vec4(normal * 0.5 + vec3(0.5), 1.0);
    } else {
        out.normal = vec4(in.world_normal * 0.5 + vec3(0.5), 1.0);
    }
#endif // NORMAL_PREPASS

#ifdef MOTION_VECTOR_PREPASS
#ifdef MESHLET_MESH_MATERIAL_PASS
    out.motion_vector = in.motion_vector;
#else
    out.motion_vector = pbr_prepass_functions::calculate_motion_vector(in.world_position, in.previous_world_position);
#endif
#endif

    return out;
}
#else
@fragment
fn fragment(in: prepass_io::VertexOutput) {
    hole_discard(in);
    pbr_prepass_functions::prepass_alpha_discard(in);
}
#endif // PREPASS_FRAGMENT
