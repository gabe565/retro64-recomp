// Direct3D 6 fixed-function rasterisation stage (texture stage 0, specular
// add, alpha test). Transform and lighting happen on the CPU (render/tnl.rs);
// blending and depth testing are pipeline state. Translated to MSL and SPIR-V
// at build time by naga (crates/runtime/build.rs).

struct VIn {
    @location(0) pos: vec4<f32>,
    @location(1) diffuse: vec4<f32>,
    @location(2) specular: vec4<f32>,
    @location(3) uv: vec2<f32>,
}

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) diffuse: vec4<f32>,
    @location(1) specular: vec4<f32>,
    @location(2) uv: vec2<f32>,
}

@vertex
fn vs_main(v: VIn) -> VOut {
    var o: VOut;
    o.pos = v.pos;
    o.diffuse = v.diffuse;
    o.specular = v.specular;
    o.uv = v.uv;
    return o;
}

// Mirrors render::gpu::StageUniforms.
struct Stage {
    // D3DTSS_COLOROP, COLORARG1, COLORARG2, D3DRENDERSTATE_SPECULARENABLE
    color: vec4<u32>,
    // D3DTSS_ALPHAOP, ALPHAARG1, ALPHAARG2, unused
    alpha: vec4<u32>,
    // alpha-test function (0 = disabled, else D3DCMPFUNC), D3DRENDERSTATE_ALPHAREF
    test: vec4<u32>,
    // D3DRENDERSTATE_TEXTUREFACTOR
    tfactor: vec4<f32>,
}

@group(3) @binding(0) var<uniform> st: Stage;
@group(2) @binding(0) var tex: texture_2d<f32>;
@group(2) @binding(1) var samp: sampler;

// D3DTA_* argument for stage 0 (CURRENT is the diffuse colour at stage 0).
fn arg(a: u32, diffuse: vec4<f32>, texel: vec4<f32>, spec: vec4<f32>) -> vec4<f32> {
    var v = diffuse;
    switch (a & 0xfu) {
        case 2u: { v = texel; }
        case 3u: { v = st.tfactor; }
        case 4u: { v = spec; }
        default: { v = diffuse; }
    }
    if ((a & 0x10u) != 0u) { v = vec4<f32>(1.0) - v; }   // D3DTA_COMPLEMENT
    if ((a & 0x20u) != 0u) { v = vec4<f32>(v.a); }       // D3DTA_ALPHAREPLICATE
    return v;
}

// D3DTEXTUREOP for stage 0.
fn op(o: u32, a1: vec4<f32>, a2: vec4<f32>, diffuse: vec4<f32>, texel: vec4<f32>) -> vec4<f32> {
    switch (o) {
        case 3u: { return a2; }                                          // SELECTARG2
        case 4u: { return a1 * a2; }                                     // MODULATE
        case 5u: { return min(a1 * a2 * 2.0, vec4<f32>(1.0)); }          // MODULATE2X
        case 6u: { return min(a1 * a2 * 4.0, vec4<f32>(1.0)); }          // MODULATE4X
        case 7u: { return min(a1 + a2, vec4<f32>(1.0)); }                // ADD
        case 8u: { return clamp(a1 + a2 - vec4<f32>(0.5), vec4<f32>(0.0), vec4<f32>(1.0)); } // ADDSIGNED
        case 12u: { return mix(a2, a1, diffuse.a); }                     // BLENDDIFFUSEALPHA
        case 13u: { return mix(a2, a1, texel.a); }                       // BLENDTEXTUREALPHA
        default: { return a1; }                                          // SELECTARG1
    }
}

fn alpha_pass(func: u32, a: f32, reference: u32) -> bool {
    let v = u32(round(clamp(a, 0.0, 1.0) * 255.0));
    switch (func) {
        case 1u: { return false; }
        case 2u: { return v < reference; }
        case 3u: { return v == reference; }
        case 4u: { return v <= reference; }
        case 5u: { return v > reference; }
        case 6u: { return v != reference; }
        case 7u: { return v >= reference; }
        default: { return true; }
    }
}

@fragment
fn fs_main(i: VOut) -> @location(0) vec4<f32> {
    // Untextured draws bind a 1×1 white texture.
    let texel = textureSample(tex, samp, i.uv);
    var c = i.diffuse;
    if (st.color.x != 1u) {   // D3DTOP_DISABLE leaves the diffuse colour
        let rgb = op(st.color.x,
            arg(st.color.y, i.diffuse, texel, i.specular),
            arg(st.color.z, i.diffuse, texel, i.specular), i.diffuse, texel);
        var a = i.diffuse.a;
        if (st.alpha.x != 1u) {
            a = op(st.alpha.x,
                arg(st.alpha.y, i.diffuse, texel, i.specular),
                arg(st.alpha.z, i.diffuse, texel, i.specular), i.diffuse, texel).a;
        }
        c = vec4<f32>(rgb.rgb, a);
    }
    if (st.color.w != 0u) {
        c = vec4<f32>(min(c.rgb + i.specular.rgb, vec3<f32>(1.0)), c.a);
    }
    if (st.test.x != 0u && !alpha_pass(st.test.x, c.a, st.test.y)) {
        discard;
    }
    return c;
}
