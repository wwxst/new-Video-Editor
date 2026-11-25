// Arcline direct compositor (docs/03 §2, §4).
// Working space: linear light, premultiplied alpha, RGBA16F. Sources arrive as RGBA8 sRGB and are
// linearized by the sampler (Rgba8UnormSrgb views).

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// ---- full-screen passes ---------------------------------------------------------------------

struct Params {
    a: vec4<f32>,
    b: vec4<f32>,
    c: vec4<f32>,
    // xy = 1 / texture size, zw = texture size
    size: vec4<f32>,
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_samp: sampler;
@group(0) @binding(2) var<uniform> prm: Params;

// ---- transformed quad: draws a prepared source into a layer ----------------------------------
// ndc = a.xy * u + a.zw * v + b.xy for u, v in [0, 1] (uv origin top-left); c.x = opacity
// multiplier applied to the premultiplied color.

@vertex
fn vs_quad(@builtin(vertex_index) i: u32) -> VsOut {
    let uv = vec2<f32>(f32(i & 1u), f32((i >> 1u) & 1u));
    var out: VsOut;
    let p = prm.a.xy * uv.x + prm.a.zw * uv.y + prm.b.xy;
    out.pos = vec4<f32>(p, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_quad(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(src_tex, src_samp, in.uv) * prm.c.x;
}

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> VsOut {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    var out: VsOut;
    out.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

fn unpremul(c: vec4<f32>) -> vec3<f32> {
    if (c.a <= 0.0) { return vec3<f32>(0.0); }
    return c.rgb / c.a;
}

// sRGB source → linear premultiplied working space
@fragment
fn fs_prepare(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(src_tex, src_samp, in.uv);
    return vec4<f32>(c.rgb * c.a, c.a);
}

// core.color.correct: a = (exposure, contrast, saturation, temperature)
@fragment
fn fs_color_correct(in: VsOut) -> @location(0) vec4<f32> {
    let s = textureSample(src_tex, src_samp, in.uv);
    var c = unpremul(s) * exp2(prm.a.x);
    let k = prm.a.w / 100.0 * 0.1;
    c = c * vec3<f32>(1.0 + k, 1.0, 1.0 - k);
    let mid = 0.18;
    c = mid * pow(max(c, vec3<f32>(0.0)) / mid, vec3<f32>(prm.a.y));
    let l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    c = max(mix(vec3<f32>(l), c, prm.a.z), vec3<f32>(0.0));
    return vec4<f32>(c * s.a, s.a);
}

// core.stylize.vignette: a = (amount, midpoint, roundness, aspect)
@fragment
fn fs_vignette(in: VsOut) -> @location(0) vec4<f32> {
    let s = textureSample(src_tex, src_samp, in.uv);
    var d = in.uv - vec2<f32>(0.5);
    d.x = d.x * mix(prm.a.w, 1.0, prm.a.z);
    let r = length(d) / 0.7071;
    let f = 1.0 - prm.a.x * smoothstep(prm.a.y, 1.0, r);
    return vec4<f32>(s.rgb * f, s.a);
}

// core.key.chroma: a = (key_r, key_g, key_b, tolerance), b = (softness, spill, -, -)
@fragment
fn fs_chroma(in: VsOut) -> @location(0) vec4<f32> {
    let s = textureSample(src_tex, src_samp, in.uv);
    var c = unpremul(s);
    let diff = distance(c, prm.a.xyz);
    let k = smoothstep(prm.a.w, prm.a.w + max(prm.b.x, 0.0001), diff);
    let m = max(c.r, c.b);
    if (c.g > m) {
        c.g = mix(c.g, m, prm.b.y);
    }
    let a = s.a * k;
    return vec4<f32>(c * a, a);
}

// core.stylize.sharpen: a = (amount, radius_px, -, -)
@fragment
fn fs_sharpen(in: VsOut) -> @location(0) vec4<f32> {
    let o = prm.size.xy * prm.a.y;
    let c = textureSample(src_tex, src_samp, in.uv);
    let n = textureSample(src_tex, src_samp, in.uv + vec2<f32>(0.0, -o.y));
    let s_ = textureSample(src_tex, src_samp, in.uv + vec2<f32>(0.0, o.y));
    let e = textureSample(src_tex, src_samp, in.uv + vec2<f32>(o.x, 0.0));
    let w = textureSample(src_tex, src_samp, in.uv + vec2<f32>(-o.x, 0.0));
    let avg = (n + s_ + e + w) * 0.25;
    let r = c + (c - avg) * prm.a.x;
    return vec4<f32>(max(r.rgb, vec3<f32>(0.0)), c.a);
}

// core.blur.gaussian (separable): a = (radius_px, dir_x, dir_y, -)
@fragment
fn fs_blur(in: VsOut) -> @location(0) vec4<f32> {
    let radius = prm.a.x;
    if (radius < 0.5) {
        return textureSample(src_tex, src_samp, in.uv);
    }
    let sigma = max(radius / 2.0, 0.5);
    let taps = i32(min(ceil(radius), 64.0));
    let dir = prm.a.yz * prm.size.xy;
    var acc = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var i = -taps; i <= taps; i = i + 1) {
        let x = f32(i);
        let w = exp(-(x * x) / (2.0 * sigma * sigma));
        acc = acc + textureSample(src_tex, src_samp, in.uv + dir * x) * w;
        wsum = wsum + w;
    }
    return acc / wsum;
}

// working space → display (Rgba8UnormSrgb target encodes sRGB); opaque output
@fragment
fn fs_resolve(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(src_tex, src_samp, in.uv);
    return vec4<f32>(c.rgb, 1.0);
}
