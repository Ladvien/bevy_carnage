// **Flesh: a StandardMaterial extension.** See `src/flesh.rs` for the model and the sources.
//
// Forward path only. In the prepass this writes the ordinary gbuffer; the wrap term is a
// post-lighting correction and a deferred lighting pass would not see it.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
    pbr_types,
    mesh_view_bindings as view_bindings,
    mesh_view_types,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    shadows,
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

struct FleshParams {
    bands: vec4<f32>,
    wet: vec4<f32>,
    sss: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> flesh: FleshParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var sss_lut: texture_2d<f32>;
// One sampler for all five: linear, clamped, level zero — see `FleshExtension`.
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var flesh_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var blood_lut: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var wet_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var flay_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var dermis_map: texture_2d<f32>;

const MODE_CAP: f32 = 0.0;
const MODE_CANVAS: f32 = 1.0;
const MODE_CLOTH: f32 = 2.0;
const FLAG_WET: u32 = 1u;
const FLAG_FLAY: u32 = 2u;
const FLAG_DERMIS: u32 = 4u;
const SSS_ROWS: f32 = 16.0;
const LAYERS: f32 = 5.0;
const SSS_MIN_RADIUS_MM: f32 = 0.5;
const SSS_MAX_RADIUS_MM: f32 = 64.0;
const BLOOD_LUT_MAX_MM: f32 = 1.0;
const PI: f32 = 3.141592653589793;

// Which of the five tissue blocks a depth fraction falls in: 0 skin … 4 marrow.
fn layer_of(depth_frac: f32) -> f32 {
    var layer = 0.0;
    if depth_frac >= flesh.bands.x { layer = 1.0; }
    if depth_frac >= flesh.bands.y { layer = 2.0; }
    if depth_frac >= flesh.bands.z { layer = 3.0; }
    if depth_frac >= flesh.bands.w { layer = 4.0; }
    return layer;
}

// The table row for a sphere radius in mm, log-spaced like `row_radius_mm` in `flesh.rs`.
fn curvature_row(radius_mm: f32) -> f32 {
    if radius_mm >= SSS_MAX_RADIUS_MM {
        return 0.0;
    }
    let lo = log(SSS_MAX_RADIUS_MM);
    let hi = log(SSS_MIN_RADIUS_MM);
    let t = clamp((log(max(radius_mm, SSS_MIN_RADIUS_MM)) - lo) / (hi - lo), 0.0, 1.0);
    return t * (SSS_ROWS - 1.0);
}

// Penner's lookup: the diffuse response of `layer` at `ndotl` on a sphere of `radius_mm`.
fn wrap_diffuse(layer: f32, ndotl: f32, radius_mm: f32) -> vec3<f32> {
    let row = layer * SSS_ROWS + curvature_row(radius_mm) + 0.5;
    let uv = vec2<f32>(ndotl * 0.5 + 0.5, row / (SSS_ROWS * LAYERS));
    return textureSampleLevel(sss_lut, flesh_sampler, uv, 0.0).rgb;
}

// The film's reflectance over `base`, from the black/white rows of the blood table, interpolated by
// the base per channel. Row pairs: arterial (0, 1), venous (2, 3); the saturation dial picks a mix.
fn blood_over(base: vec3<f32>, depth_mm: f32) -> vec3<f32> {
    let u = clamp(depth_mm / BLOOD_LUT_MAX_MM, 0.0, 1.0);
    let art_black = textureSampleLevel(blood_lut, flesh_sampler, vec2<f32>(u, 0.125), 0.0).rgb;
    let art_white = textureSampleLevel(blood_lut, flesh_sampler, vec2<f32>(u, 0.375), 0.0).rgb;
    let ven_black = textureSampleLevel(blood_lut, flesh_sampler, vec2<f32>(u, 0.625), 0.0).rgb;
    let ven_white = textureSampleLevel(blood_lut, flesh_sampler, vec2<f32>(u, 0.875), 0.0).rgb;
    // `wet.w` is SO₂; venous 0.75 → 0, arterial 0.97 → 1.
    let a = clamp((flesh.wet.w - 0.75) / 0.22, 0.0, 1.0);
    let black = mix(ven_black, art_black, a);
    let white = mix(ven_white, art_white, a);
    return black + (white - black) * base;
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    let mode = flesh.sss.z;
    let flags = u32(flesh.sss.w);

    // ---- The canvases: what is wet, what is peeled. ------------------------------------------
    var amount = 0.0;      // film depth byte, 0..1
    var wetness = 0.0;     // fresh 1 → dry 0
    var depth_frac = 0.0;  // depth over the layer span
    var peeled = 0.0;      // 1 where the skin is gone
#ifdef VERTEX_UVS_A
    if (flags & FLAG_WET) != 0u {
        let w = textureSample(wet_map, flesh_sampler, in.uv);
        amount = w.r;
        wetness = w.a;
    }
    if (flags & FLAG_FLAY) != 0u {
        let f = textureSample(flay_map, flesh_sampler, in.uv);
        depth_frac = f.r;
        peeled = f.a;
    }
    // What lies under intact skin: a bruise's chromophores or a burn's eschar modulate the skin's
    // own colour (the map is a ratio, linear), and go with the skin when it is peeled.
    if (flags & FLAG_DERMIS) != 0u {
        let d = textureSample(dermis_map, flesh_sampler, in.uv);
        let show = d.a * (1.0 - peeled);
        let ratio = mix(vec3<f32>(1.0), d.rgb, show);
        pbr_input.material.base_color = vec4<f32>(pbr_input.material.base_color.rgb * ratio, pbr_input.material.base_color.a);
    }
#endif
#ifdef VERTEX_UVS_B
    if mode == MODE_CAP {
        depth_frac = in.uv_b.x;
    }
#endif
    let layer = layer_of(depth_frac);

    // ---- Blood: composite the film over whatever is under it, on the GPU. -------------------
    // The wetmap's own albedo image already carries this film over a grey; reading the amount byte
    // and compositing here instead keeps the hue of the base — skin, a peeled tissue, a weave.
    if amount > 0.0 {
        let depth_mm = amount * flesh.wet.z;
        let base = pbr_input.material.base_color.rgb;
        pbr_input.material.base_color = vec4<f32>(blood_over(base, depth_mm), pbr_input.material.base_color.a);
        // A wet film smooths whatever it lies on; a dried one is matte again.
        pbr_input.material.perceptual_roughness = mix(pbr_input.material.perceptual_roughness, 0.25, wetness);
    }

    // ---- The wet clear coat: a specular layer of liquid above the diffuse colour. --------------
    let coat = wetness * flesh.wet.x * step(0.0001, amount);
    pbr_input.material.clearcoat = coat;
    pbr_input.material.clearcoat_perceptual_roughness = flesh.wet.y;

    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

#ifdef PREPASS_PIPELINE
    let out = deferred_output(in, pbr_input);
#else
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);

    // ---- Subsurface: the wrap the diffusion profile adds past the terminator. -----------------
    let strength = flesh.sss.x;
    if strength > 0.0 && mode != MODE_CLOTH {
        let N = pbr_input.N;
        let P = pbr_input.world_position.xyz;
        // Curvature from screen-space derivatives (Penner 2011): |dN| / |dP| is 1/r in mesh units.
        let dn = length(fwidth(N));
        let dp = max(length(fwidth(P)), 1.0e-6);
        let radius_units = dp / max(dn, 1.0e-6);
        let radius_mm = radius_units * flesh.sss.y;

        let diffuse_color = pbr_input.material.base_color.rgb * (1.0 - pbr_input.material.metallic);
        var extra = vec3<f32>(0.0);
        let n_directional_lights = view_bindings::lights.n_directional_lights;
        for (var i: u32 = 0u; i < n_directional_lights; i = i + 1u) {
            let light = &view_bindings::lights.directional_lights[i];
            let L = (*light).direction_to_light;
            let ndotl = dot(N, L);
            var shadow: f32 = 1.0;
            if ((pbr_input.flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) != 0u
                    && ((*light).flags & mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u) {
                let view_z = dot(vec4<f32>(
                    view_bindings::view.view_from_world[0].z,
                    view_bindings::view.view_from_world[1].z,
                    view_bindings::view.view_from_world[2].z,
                    view_bindings::view.view_from_world[3].z
                ), pbr_input.world_position);
                shadow = shadows::fetch_directional_shadow(i, pbr_input.world_position, pbr_input.world_normal, view_z, pbr_input.frag_coord.xy);
            }
            let wrap = wrap_diffuse(layer, ndotl, radius_mm);
            let lambert = vec3<f32>(max(ndotl, 0.0));
            extra += diffuse_color * (wrap - lambert) * (*light).color.rgb * shadow / PI;
        }
        // No separate ambient term, deliberately: under uniform ambient light a convex surface's
        // subsurface response integrates to its total diffuse reflectance, and that reflectance *is*
        // the base colour the ambient term already multiplies. Adding the profile's hue on top
        // counted it twice and turned skin in shadow terracotta (measured, 0.4.0).
        // `apply_pbr_lighting` scales every light term by the view's exposure at the end; these are
        // light terms too.
        out.color = vec4<f32>(out.color.rgb + extra * strength * view_bindings::view.exposure, out.color.a);
    }

    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
#endif

    return out;
}
