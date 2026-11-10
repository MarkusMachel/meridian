// Map shaders. World coordinates are whole metres (i32); subtracting the camera centre as
// integers first keeps full precision, and only the small difference becomes a float.

struct Globals {
    center: vec2<i32>,
    viewport: vec2<f32>,
    inv_scale: f32,      // pixels per metre
    selected: u32,
    hover: u32,
    relief_on: u32,          // 1 = shade with the relief raster
    relief_org: vec2<i32>,   // left and top edge of the relief and land rasters (metres)
    relief_span: vec2<f32>,  // width and height in metres
    land_mix: f32,           // 0 = real land colours only, 1 = province colour only
    detail: f32,             // 0..1: strength of the fine ground texture (close zoom)
    px_scale: f32,           // target pixels per screen pixel (above 1 when drawing the 3D overlay)
    relief_k: f32,           // strength of the shaded relief (fades out as real 3D lighting takes over)
    paper: f32,              // 0..1: paper look of the zoomed-out map
    _p1: f32,
    _p2: f32,
    _p3: f32,
};
@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var<storage, read> colors: array<u32>;
@group(0) @binding(2) var relief: texture_2d<f32>;
@group(0) @binding(3) var relief_sampler: sampler;
@group(0) @binding(4) var land: texture_2d<f32>;
@group(0) @binding(5) var heights: texture_2d<f32>;

// Value noise for the ground texture: a hash per integer lattice point, smoothly interpolated.
fn hash2(p: vec2<i32>) -> f32 {
    var h = u32(p.x) * 374761393u + u32(p.y) * 668265263u;
    h = (h ^ (h >> 13u)) * 1274126177u;
    h = h ^ (h >> 16u);
    return f32(h & 0xffffu) / 65535.0;
}

fn vnoise(p: vec2<f32>) -> f32 {
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash2(i);
    let b = hash2(i + vec2<i32>(1, 0));
    let c = hash2(i + vec2<i32>(0, 1));
    let d = hash2(i + vec2<i32>(1, 1));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

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
    let d = vec2<f32>(g.center - g.relief_org);
    let uv = vec2<f32>((d.x + i.rel.x) / g.relief_span.x, (-d.y - i.rel.y) / g.relief_span.y);
    // the province (owner) colour as a tint over the real land colours: forests, deserts, ice show through
    let ground = textureSampleLevel(land, relief_sampler, uv, 0.0).rgb;
    // multiply: the owner colour keeps its hue, the land darkens it over forest and warms it over
    // desert (1.2 keeps light deserts from washing out); then blend toward the plain land colour
    let tinted = clamp(i.color.rgb * ground * 1.2, vec3<f32>(0.0), vec3<f32>(1.0));
    var rgb = mix(ground * 0.9, tinted, g.land_mix);
    if (g.land_mix >= 1.0) {
        rgb = i.color.rgb;   // flat colours (debug mode): no land texture
    }
    if (g.relief_on == 1u) {
        let r = textureSampleLevel(relief, relief_sampler, uv, 0.0).r * (255.0 / 128.0);
        rgb = rgb * mix(1.0, mix(1.0, r, 0.7) * 0.92, g.relief_k);
    }
    if (g.detail > 0.0) {
        // fine mottling of the ground when zoomed in, so close up it is not a flat colour
        let w = vec2<f32>(g.center) + i.rel;
        let n = vnoise(w / 1600.0) * 0.55 + vnoise(w / 450.0) * 0.30 + vnoise(w / 140.0) * 0.15;
        rgb = rgb * (1.0 + (n - 0.5) * 0.16 * g.detail);
    }
    if (g.paper > 0.0) {
        rgb = rgb * paper_grain(vec2<f32>(g.center) + i.rel);
    }
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), i.color.a);
}

/// The zoomed-out map as printed paper: broad faint stains and fine fibres, fixed to the world.
fn paper_grain(w: vec2<f32>) -> f32 {
    let stains = vnoise(w / 900000.0) * 0.6 + vnoise(w / 310000.0) * 0.4;
    let fibre = vnoise(w / 45000.0) * 0.6 + vnoise(w / 16000.0) * 0.4;
    return 1.0 + ((stains - 0.5) * 0.06 + (fibre - 0.5) * 0.05) * g.paper;
}

/// The sea of the printed map: lighter on the shelf, a shade darker past each depth line, with a
/// thin contour at 200, 1,000, 3,000 and 5,000 metres, as in an atlas. `fw` is how much the depth
/// changes across one pixel (keeps the contour lines one pixel wide).
fn paper_sea_color(depth: f32, fw: f32) -> vec3<f32> {
    let lines = array<f32, 4>(200.0, 1000.0, 3000.0, 5000.0);
    var band = 0.0;
    var line = 0.0;
    for (var k = 0; k < 4; k++) {
        band += step(lines[k], depth);
        line = max(line, 1.0 - smoothstep(0.0, max(fw, 1.0) * 1.2, abs(depth - lines[k])));
    }
    let c = vec3<f32>(0.71, 0.80, 0.83) - vec3<f32>(0.045, 0.040, 0.025) * band;
    return c * (1.0 - 0.10 * line);
}


// The sea of the flat map: one triangle that covers the screen, with the paper grain.
struct SeaOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_sea(@builtin(vertex_index) vi: u32) -> SeaOut {
    var xy = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var o: SeaOut;
    o.clip = vec4<f32>(xy[vi], 0.0, 1.0);
    o.ndc = xy[vi];
    return o;
}

@fragment
fn fs_sea(i: SeaOut) -> @location(0) vec4<f32> {
    let rel = i.ndc * g.viewport * 0.5 / g.inv_scale;
    let d = vec2<f32>(g.center - g.relief_org);
    let uv = vec2<f32>((d.x + rel.x) / g.relief_span.x, (-d.y - rel.y) / g.relief_span.y);
    let inside = all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0));
    let depth = select(4000.0, -textureSampleLevel(heights, relief_sampler, uv, 0.0).r, inside);
    let rgb = paper_sea_color(depth, fwidth(depth)) * paper_grain(vec2<f32>(g.center) + rel);
    return vec4<f32>(rgb, 1.0);
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
    let h = style.width_px * 0.5 * g.px_scale;
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
    if (style.dash_px > 0.0 && fract(i.along / (style.dash_px * g.px_scale)) > 0.6) {
        discard;
    }
    return style.color;
}

// Solid fills in one colour (built-up areas): the style's colour, no per-vertex data but position.
@vertex
fn vs_solid(@location(0) pos: vec2<i32>) -> @builtin(position) vec4<f32> {
    return vec4<f32>(to_ndc(pos), 0.0, 1.0);
}

@fragment
fn fs_solid() -> @location(0) vec4<f32> {
    return style.color;
}

// Built-up areas as a mask for the 3D buildings.
@fragment
fn fs_mask() -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 1.0, 1.0, 1.0);
}
