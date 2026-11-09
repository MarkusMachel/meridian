// Map shaders. World coordinates are whole metres (i32); subtracting the camera centre as
// integers first keeps full precision, and only the small difference becomes a float.

struct Globals {
    center: vec2<i32>,
    viewport: vec2<f32>,
    inv_scale: f32,      // pixels per metre
    selected: u32,
    hover: u32,
    _pad: u32,
    relief_org: vec2<i32>,   // left and top edge of the relief raster (metres)
    relief_span: vec2<f32>,  // width and height in metres; 0 = no relief
};
@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var<storage, read> colors: array<u32>;
@group(0) @binding(2) var relief: texture_2d<f32>;
@group(0) @binding(3) var relief_sampler: sampler;

fn to_ndc(p: vec2<i32>) -> vec2<f32> {
    let rel = vec2<f32>(p - g.center) * g.inv_scale;
    return rel / (g.viewport * 0.5);
}

struct FillIn {
    @location(0) pos: vec2<i32>,
    @location(1) prov: u32,
};
struct FillOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) rel: vec2<f32>,   // metres from the camera centre
};

@vertex
fn vs_fill(v: FillIn) -> FillOut {
    var c = unpack4x8unorm(colors[v.prov]);
    if (v.prov == g.selected) {
        c = vec4<f32>(mix(c.rgb, vec3<f32>(1.0, 1.0, 1.0), 0.55), 1.0);
    } else if (v.prov == g.hover) {
        c = vec4<f32>(mix(c.rgb, vec3<f32>(1.0, 1.0, 1.0), 0.25), 1.0);
    }
    var o: FillOut;
    o.clip = vec4<f32>(to_ndc(v.pos), 0.0, 1.0);
    o.color = c;
    o.rel = vec2<f32>(v.pos - g.center);
    return o;
}

@fragment
fn fs_fill(i: FillOut) -> @location(0) vec4<f32> {
    var rgb = i.color.rgb;
    if (g.relief_span.x > 0.0) {
        let d = vec2<f32>(g.center - g.relief_org);
        let uv = vec2<f32>((d.x + i.rel.x) / g.relief_span.x, (-d.y - i.rel.y) / g.relief_span.y);
        let r = textureSampleLevel(relief, relief_sampler, uv, 0.0).r * (255.0 / 128.0);
        rgb = clamp(rgb * mix(1.0, r, 0.7) * 0.92, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    return vec4<f32>(rgb, i.color.a);
}

// Lines: one instance per segment, drawn as a quad of 6 vertices.
struct Style {
    color: vec4<f32>,
    width_px: f32,
    dash_px: f32,        // dash period in pixels; 0 = solid
};
@group(1) @binding(0) var<uniform> style: Style;

struct LineIn {
    @location(0) a: vec2<i32>,
    @location(1) b: vec2<i32>,
    @location(2) dist: f32,   // metres along the line at the start of this segment
};
struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) along: f32,  // pixels along the line
};

@vertex
fn vs_line(@builtin(vertex_index) vi: u32, l: LineIn) -> LineOut {
    let pa = vec2<f32>(l.a - g.center) * g.inv_scale;
    let pb = vec2<f32>(l.b - g.center) * g.inv_scale;
    var d = pb - pa;
    let len = length(d);
    if (len < 1e-6) { d = vec2<f32>(1.0, 0.0); } else { d = d / len; }
    let n = vec2<f32>(-d.y, d.x);
    let h = style.width_px * 0.5;
    // corners: (end, side) for two triangles
    var end = array<f32, 6>(0.0, 0.0, 1.0, 0.0, 1.0, 1.0);
    var side = array<f32, 6>(-1.0, 1.0, -1.0, 1.0, 1.0, -1.0);
    let e = end[vi];
    let s = side[vi];
    // extend each end by half the width so consecutive segments overlap at corners
    let base = mix(pa - d * h, pb + d * h, e);
    let p = base + n * (s * h);
    var o: LineOut;
    o.clip = vec4<f32>(p / (g.viewport * 0.5), 0.0, 1.0);
    o.along = l.dist * g.inv_scale + mix(-h, len + h, e);
    return o;
}

@fragment
fn fs_line(i: LineOut) -> @location(0) vec4<f32> {
    if (style.dash_px > 0.0 && fract(i.along / style.dash_px) > 0.6) {
        discard;
    }
    return style.color;
}
