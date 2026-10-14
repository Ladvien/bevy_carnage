// **Flesh: a StandardMaterial extension.** See `src/flesh.rs` for the model and the sources.
//
// Forward path only. In the prepass this writes the ordinary gbuffer; the wrap term is a
// post-lighting correction and a deferred lighting pass would not see it.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_bindings,
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
    cut: vec4<f32>,
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
const MODE_FLOOR: f32 = 3.0;
const FLAG_WET: u32 = 1u;
const FLAG_FLAY: u32 = 2u;
const FLAG_DERMIS: u32 = 4u;
const SSS_ROWS: f32 = 16.0;
const LAYERS: f32 = 5.0;
const SSS_MIN_RADIUS_MM: f32 = 0.5;
const SSS_MAX_RADIUS_MM: f32 = 64.0;
const BLOOD_LUT_MAX_MM: f32 = 1.0;
const PI: f32 = 3.141592653589793;

// **The film depth at which a wet map's rim is drawn**, as a share of the canvas's full depth: 4 %,
// 0.08 mm at the shipped 2 mm. Below it the smoothed field is the blur's own skirt, not blood. This
// crate's own number; at 8 % the thin smear a blade leaves along a cut's lips was cut away.
const WET_EDGE: f32 = 0.04;

// **How much higher a film's surface is drawn than its depth**: 6. A pool's edge is a meniscus a
// millimetre or two wide, far narrower than a canvas texel, so the smoothed depth climbs over a
// texel what the blood climbs over its meniscus; drawn at its true depth, a 2 mm pool on a floor of
// 12 mm texels tilts its edge 10° and catches no light. This crate's own number.
const FILM_RELIEF: f32 = 6.0;

// **The steepest a film's surface is drawn**, as the cosine of its tilt from what it lies on: 25°.
// Without a cap the relief that lifts a floor's 12 mm texels stood a skin's 3.6 mm ones on end; at 35°
// the gaps between a wound's runs, and between the clots in a crater seen close, shaded as dark
// crevices. This crate's own number.
const FILM_MAX_TILT_COS: f32 = 0.906;

// **The share of `FleshParams::cut`'s film a crater's graze wears**: 0.04, reached over the first
// `GRAZE_RAMP` (6 %) of the layer span below the skin, about the skin's own thickness. A fresh
// abrasion is raw and moist — the papillary dermis bleeds and weeps from the first moment — so skin a
// crater only grazed is reddened by a thin film of its own ooze and shines a little; the ramp lets the
// shallowest scuff at the graze's edge fade into the skin rather than stop at the edge of the canvas's
// peeled mask. Bare, as until 0.5.53, the graze was the dermis's pale pink mottled with cream where the
// fat came through, and round every crater it read as a pale halo; at 0.08 with no ramp it was a hard
// red disc. The fact is forensic; the numbers are this crate's own.
const GRAZE_FILM: f32 = 0.04;
const GRAZE_RAMP: f32 = 0.06;

// The cubic B-spline weights of the four texels around a sample at fraction `v` between the middle two.
fn bspline(v: f32) -> vec4<f32> {
    let n = vec4<f32>(1.0, 2.0, 3.0, 4.0) - v;
    let s = n * n * n;
    let x = s.x;
    let y = s.y - 4.0 * s.x;
    let z = s.z - 4.0 * s.y + 6.0 * s.x;
    return vec4<f32>(x, y, z, 6.0 - x - y - z) * (1.0 / 6.0);
}

// **The wet map, reconstructed as a cubic B-spline** in four bilinear taps (Sigg & Hadwiger, "Fast
// third-order texture filtering", GPU Gems 2, ch. 20). A bilinear read of a canvas whose texels are
// wet or not draws every rim as the texels' staircase; the spline is smooth to its second derivative,
// so a level set of it is a curve.
fn wet_smooth(uv: vec2<f32>) -> vec4<f32> {
    let size = vec2<f32>(textureDimensions(wet_map));
    let p = uv * size - 0.5;
    let f = fract(p);
    let i = p - f;
    let xc = bspline(f.x);
    let yc = bspline(f.y);
    let c = vec4<f32>(i.x - 0.5, i.x + 1.5, i.y - 0.5, i.y + 1.5);
    let s = vec4<f32>(xc.x + xc.y, xc.z + xc.w, yc.x + yc.y, yc.z + yc.w);
    let o = (c + vec4<f32>(xc.y, xc.w, yc.y, yc.w) / s) / vec4<f32>(size.x, size.x, size.y, size.y);
    let s0 = textureSample(wet_map, flesh_sampler, vec2<f32>(o.x, o.z));
    let s1 = textureSample(wet_map, flesh_sampler, vec2<f32>(o.y, o.z));
    let s2 = textureSample(wet_map, flesh_sampler, vec2<f32>(o.x, o.w));
    let s3 = textureSample(wet_map, flesh_sampler, vec2<f32>(o.y, o.w));
    let sx = s.x / (s.x + s.y);
    let sy = s.z / (s.z + s.w);
    return mix(mix(s3, s2, sx), mix(s1, s0, sx), sy);
}

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

// **The normal of a surface lifted above the one at `N`**, given how fast the lift changes across the
// screen (`dh`, world units per pixel in x and y), so it needs no tangents (Mikkelsen, "Bump Mapping
// Unparametrized Surfaces on the GPU", 2010). Derivatives: call it only in uniform control flow.
fn lifted_normal(N: vec3<f32>, P: vec3<f32>, dh: vec2<f32>) -> vec3<f32> {
    let dpx = dpdx(P);
    let dpy = dpdy(P);
    let r1 = cross(dpy, N);
    let r2 = cross(N, dpx);
    let det = dot(dpx, r1);
    let grad = sign(det) * (dh.x * r1 + dh.y * r2);
    let n = normalize(abs(det) * N - grad);
    let c = dot(n, N);
    if c >= FILM_MAX_TILT_COS {
        return n;
    }
    // Turned toward `N` until it leans no further than the cap allows.
    let side = normalize(n - N * c);
    return N * FILM_MAX_TILT_COS + side * sqrt(1.0 - FILM_MAX_TILT_COS * FILM_MAX_TILT_COS);
}

// **How many directions and how far a crater's floor looks for its walls.** Eight directions, each
// out to 1, 3, 7 and 15 texels: on the mannequin's 256-texel skins, 3.6 mm a texel, that is 54 mm,
// past the rim of a 30 mm wound.
const CAVITY_DIRS: u32 = 8u;
const CAVITY_STEPS: u32 = 4u;

// **How much of the room a point in a crater sees**, `[0, 1]`: one minus the mean, over
// `CAVITY_DIRS` directions, of the sine of the highest wall it sees that way. In a section across a
// hollow, the cosine-weighted share of a point's view that clears both walls is
// `1 − (sin h₁ + sin h₂) / 2` for their elevations `h₁, h₂` (the crossed-strings result a wound bed's
// lips and floor are lit by), and the mean over directions is that, all the way round. The walls are
// the flaymap's own depth, `depth_mm` here against the depth at each tap, `mm_per_uv` the canvas's
// size on the body. It leaves out the light the walls throw back.
fn crater_light(uv: vec2<f32>, depth_mm: f32, mm_per_uv: f32) -> f32 {
    let size = vec2<f32>(textureDimensions(pbr_bindings::metallic_roughness_texture));
    let mm_per_texel = mm_per_uv / max(size.x, 1.0);
    var seen = 0.0;
    for (var i = 0u; i < CAVITY_DIRS; i = i + 1u) {
        let a = (f32(i) + 0.5) * (2.0 * PI / f32(CAVITY_DIRS));
        let dir = vec2<f32>(cos(a), sin(a));
        var wall = 0.0;
        for (var k = 0u; k < CAVITY_STEPS; k = k + 1u) {
            let texels = f32((2u << k) - 1u);
            let tap = textureSampleLevel(pbr_bindings::metallic_roughness_texture, pbr_bindings::metallic_roughness_sampler, uv + dir * texels / size, 0.0).r;
            let rise = depth_mm - tap * flesh.cut.y;
            let run = texels * mm_per_texel;
            wall = max(wall, rise / max(sqrt(rise * rise + run * run), 1.0e-6));
        }
        seen += wall;
    }
    return clamp(1.0 - seen / f32(CAVITY_DIRS), 0.0, 1.0);
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
    var film_n = pbr_input.N;            // the normal of a film's own surface
    var film_cn = pbr_input.clearcoat_N; // and of the coat on it
    var film_dh = vec2<f32>(0.0);        // how fast the film's surface rises across the screen
    var flay_mm_per_uv = 0.0;            // the flaymap's size on the body, for a crater's walls
#ifdef VERTEX_UVS_A
    if (flags & FLAG_WET) != 0u {
        // Smooth inside, and its rim drawn crisp at `WET_EDGE`: a film's edge is a line, and the
        // B-spline alone would leave a soft skirt a texel wide. `fwidth` is the smoothed amount's
        // change over one pixel, so the rim is a pixel wide however far the canvas is magnified, and
        // anti-aliased rather than stepped where it is minified.
        let w = wet_smooth(in.uv);
        let aa = max(fwidth(w.r), 1.0e-4);
        amount = w.r * clamp((w.r - WET_EDGE) / aa + 0.5, 0.0, 1.0);
        wetness = w.a;
#ifndef PREPASS_PIPELINE
        // **A film has a surface.** Its depth, smooth to its second derivative, is a height above
        // what it lies on, and that height tilts the normal: a pool's edge curves down to the floor
        // and catches the light as a meniscus does, a run stands up off the skin, and a pool's
        // unevenness breaks a light's mirror image into a glint. Before 0.5.47 a film was flat however
        // deep, a decal of one colour to its rim, and a pool at a light's mirror angle was one white
        // sticker. A weave holds its blood between the yarns, so cloth stays flat. Leveling a pool
        // above a quarter of its depth, as a liquid at rest would be, was tried and made that sticker
        // again.
        //
        // The slope is the spline's own, a central difference a quarter of a texel either way, carried
        // to the screen by the UV's derivatives, which are constant across a triangle; taken by `dpdx`
        // of the height instead it was a step per 2×2 pixels, and a run on a distant arm glittered. It
        // fades out where a texel is under a pixel and a surface that fine cannot be drawn.
        if mode != MODE_CLOTH {
            let size = vec2<f32>(textureDimensions(wet_map));
            let du = vec2<f32>(0.25 / size.x, 0.0);
            let dv = vec2<f32>(0.0, 0.25 / size.y);
            let slope = vec2<f32>(
                (wet_smooth(in.uv + du).r - wet_smooth(in.uv - du).r) / (2.0 * du.x),
                (wet_smooth(in.uv + dv).r - wet_smooth(in.uv - dv).r) / (2.0 * dv.y),
            );
            let uv_x = dpdx(in.uv);
            let uv_y = dpdy(in.uv);
            let px_per_texel = 1.0 / max(max(length(uv_x * size), length(uv_y * size)), 1.0e-6);
            let lift = flesh.wet.z / max(flesh.sss.y, 1.0e-6) * FILM_RELIEF * smoothstep(0.35, 1.0, px_per_texel);
            film_dh = vec2<f32>(dot(slope, uv_x), dot(slope, uv_y)) * lift;
        }
#endif
    }
    if (flags & FLAG_FLAY) != 0u {
        // The canvas's roughness image is in the base's metallic-roughness slot (see
        // `FleshTables::material`): `R` the depth over the span, `A` whether the texel is peeled.
        // `pbr_bindings` is the non-bindless layout, which is the only one this material has:
        // `ExtendedMaterial` goes bindless only when its extension does, and `FleshExtension` does not.
        let f = textureSample(pbr_bindings::metallic_roughness_texture, pbr_bindings::metallic_roughness_sampler, in.uv);
        depth_frac = f.r;
        peeled = f.a;
        // The canvas's albedo is the flat colour it was dressed with wherever the skin is intact.
        // Swap that for the skin as authored (texture, factor, vertex colours) by exactly the intact
        // share of the texel. A peeled texel keeps the canvas's tissue, and an untextured skin, whose
        // authored colour *is* the flat one, comes out as the canvas drew it.
        let canvas = textureSample(flay_map, flesh_sampler, in.uv).rgb;
        let flat = pbr_bindings::material.base_color.rgb;
        let authored = pbr_input.material.base_color.rgb;
        pbr_input.material.base_color = vec4<f32>(canvas + (1.0 - peeled) * (authored - flat), pbr_input.material.base_color.a);
#ifndef PREPASS_PIPELINE
        // The canvas's size on the body, millimetres per UV, from how far the surface and its UV move
        // across a pixel. Taken here, where the flags are uniform: a derivative cannot be taken in
        // the per-pixel branch that uses it.
        flay_mm_per_uv = length(fwidth(pbr_input.world_position.xyz)) / max(length(fwidth(in.uv)), 1.0e-9) * flesh.sss.y;
#endif
    }
    // What lies under intact skin: a bruise's chromophores or a burn's eschar modulate the skin's
    // own colour (the map is a ratio, linear), and go with the skin when it is peeled. On cloth the
    // same slot holds the holes shot through it: `rgb` the ratio of the ring around each, shown by
    // twice `a`, and the cloth gone wherever `a` is over one half — the alpha the mask cuts on.
    if (flags & FLAG_DERMIS) != 0u {
        let d = textureSample(dermis_map, flesh_sampler, in.uv);
        if mode == MODE_CLOTH {
            let ratio = mix(vec3<f32>(1.0), d.rgb, clamp(d.a * 2.0, 0.0, 1.0));
            let kept = 1.0 - step(0.5, d.a);
            pbr_input.material.base_color = vec4<f32>(pbr_input.material.base_color.rgb * ratio, pbr_input.material.base_color.a * kept);
        } else {
            let show = d.a * (1.0 - peeled);
            let ratio = mix(vec3<f32>(1.0), d.rgb, show);
            pbr_input.material.base_color = vec4<f32>(pbr_input.material.base_color.rgb * ratio, pbr_input.material.base_color.a);
        }
    }
#ifndef PREPASS_PIPELINE
    // **Blood in a crater lies level.** It fills a hollow the skin was peeled from, so the relief
    // is a film's on intact skin and none of it shows over a peeled texel; lifted there, the clots of
    // a crater seen close stood up as cobbles with dark crevices between them.
    if (flags & FLAG_WET) != 0u && mode != MODE_CLOTH {
        let dh = film_dh * (1.0 - peeled);
        film_n = lifted_normal(pbr_input.N, pbr_input.world_position.xyz, dh);
        film_cn = lifted_normal(pbr_input.clearcoat_N, pbr_input.world_position.xyz, dh);
    }
#endif
#endif
#ifdef VERTEX_UVS_B
    if mode == MODE_CAP {
        depth_frac = in.uv_b.x;
    }
#endif
    // ---- Exposed tissue, wet with its own blood (`FleshParams::cut`). ------------------------
    // A cut face, and a crater's walls where the skin is peeled. Thickest where the vessels are, in
    // the muscle, by the tissue's own depth below the skin (`UV_1.x` on a cap, the flaymap's depth on
    // a canvas). A cap keeps a quarter of it over the fat at its rim, so a piece reads as meat with a
    // paler edge and the bands still show; a crater's graze keeps `GRAZE_FILM` of it, the ooze of a
    // fresh abrasion, so the dermis reads raw and the fat under it reddens, and the deep wound is
    // dark with blood, where bare it was a ring of yellow fat. Its liquid shines. A value noise over
    // `UV_1` was tried first, for patches, and on the preset's pieces it drew stripes across each
    // face, not patches.
    var cut_coat = 0.0;
    var exposed = 0.0;
    var rim = 0.0;
#ifdef VERTEX_UVS_B
    if mode == MODE_CAP {
        exposed = 1.0;
        rim = 0.25;
    }
#endif
    if mode == MODE_CANVAS {
        exposed = peeled;
        rim = GRAZE_FILM * smoothstep(0.0, GRAZE_RAMP, depth_frac);
    }
    if flesh.cut.x > 0.0 && exposed > 0.0 {
        let thick = smoothstep(0.08, 0.5, depth_frac);
        let film_mm = flesh.cut.x * mix(rim, 1.0, thick);
        // How wet it shines: a half where a cap's rim film lies, a little on a graze, all of it deep.
        let shine = mix(2.0 * rim, 1.0, thick) * exposed;
        let tissue = pbr_input.material.base_color.rgb;
        pbr_input.material.base_color = vec4<f32>(mix(tissue, blood_over(tissue, film_mm), exposed), pbr_input.material.base_color.a);
        pbr_input.material.perceptual_roughness = mix(pbr_input.material.perceptual_roughness, 0.45, shine);
        cut_coat = flesh.wet.x * shine;
    }
    let layer = layer_of(depth_frac);

    // ---- Blood: composite the film over whatever is under it, on the GPU. -------------------
    // The wetmap's own albedo image already carries this film over a grey; reading the amount byte
    // and compositing here instead keeps the hue of the base — skin, a peeled tissue, a weave.
    // **How much liquid there is to shine**, as a share of enough to mirror: nothing under two per cent
    // of the film's depth, all of it from a quarter of it up. A smear a few microns deep follows the
    // surface it lies on and only a pool smooths it. Every film used to count as a pool, and a wet
    // surface went to a roughness of 0.25 and a coat of 1.0 at the first texel of blood. A flat pool in
    // a light's mirror angle then clipped to white, so 0.5.23 took the gloss to 0.45. Since 0.5.47 a
    // film's surface has relief, which breaks that mirror into a glint, and the gloss is 0.3. A liquid's
    // own 0.1 was tried: with no environment to reflect, the sheen that says "wet" shrank to a speck
    // and runs and pools read as matte paint.
    let pool = smoothstep(0.02, 0.25, amount);
    // Blood in a weave is held between the yarns rather than stood on them, so a soaked shirt is
    // darker and a little smoother, never a mirror: 0.7 and a third of the coat, where a film is 0.3.
    let weave = mode == MODE_CLOTH;
    let wet_rough = select(0.3, 0.7, weave);
    if amount > 0.0 {
        let depth_mm = amount * flesh.wet.z;
        let base = pbr_input.material.base_color.rgb;
        pbr_input.material.base_color = vec4<f32>(blood_over(base, depth_mm), pbr_input.material.base_color.a);
        // A wet film smooths whatever it lies on; a dried one is matte again.
        pbr_input.material.perceptual_roughness = mix(pbr_input.material.perceptual_roughness, wet_rough, wetness * pool);
        pbr_input.N = film_n;
        pbr_input.clearcoat_N = film_cn;
    }

    // ---- The wet clear coat: a specular layer of liquid above the diffuse colour. --------------
    let coat = max(wetness * flesh.wet.x * pool * select(1.0, 1.0 / 3.0, weave), cut_coat);
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
    if strength > 0.0 && mode < MODE_CLOTH {
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

#ifdef VERTEX_UVS_A
    // ---- A crater is lit through its mouth. -------------------------------------------------
    // Every light term, the coat's and the subsurface's with them: a crater's floor sees the room
    // only between its walls (`crater_light`). Drawn as open skin, a wound pooled with blood was a
    // flat red decal whatever its depth.
    if (flags & FLAG_FLAY) != 0u && mode == MODE_CANVAS && depth_frac > 0.0 {
        let lit = crater_light(in.uv, depth_frac * flesh.cut.y, flay_mm_per_uv);
        out.color = vec4<f32>(out.color.rgb * lit, out.color.a);
    }
#endif

    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
#endif

    return out;
}
