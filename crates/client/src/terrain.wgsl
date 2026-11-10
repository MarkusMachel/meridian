// The 3D view. The flat map (provinces, borders, roads...) is first drawn into an overlay texture
// seen from straight above; here it is laid over a terrain mesh raised from the height raster,
// lit from the north-west, with animated water, trees in forests and buildings in towns.
// Positions are metres relative to the point the camera looks at (small numbers, f32 is enough).

struct G3 {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,       // xyz: eye relative to the target; w: time in seconds
    ov: vec4<f32>,        // overlay centre (x, y, relative), overlay pixels per metre, unused
    ov_half: vec4<f32>,   // overlay half size in pixels (x, y), 1 / overlay texture size, depth_t
    grid: vec4<f32>,      // terrain grid corner (x, y, relative), grid step (x, y) in metres
    hmap: vec4<f32>,      // height raster left and top edge (relative), width and height in metres
    hpx: vec4<f32>,       // one raster pixel in metres (x, y), unused
    misc: vec4<f32>,      // exaggeration, metres per screen pixel, grid size (cells per side), haze
    center: vec4<i32>,    // the target in world metres (x, y), unused
    trees: vec4<i32>,     // first tree cell (x, y), cells per row, cell size in metres
    blds: vec4<i32>,      // the same for buildings
    sizes: vec4<f32>,     // tree size, building size (metres; 0 = none), unused
};
@group(0) @binding(0) var<uniform> g3: G3;
@group(0) @binding(1) var overlay: texture_2d<f32>;
@group(0) @binding(2) var urban_mask: texture_2d<f32>;
@group(0) @binding(3) var heights: texture_2d<f32>;
@group(0) @binding(4) var land: texture_2d<f32>;
@group(0) @binding(5) var samp: sampler;

const LIGHT: vec3<f32> = vec3<f32>(-0.42, 0.52, 0.74);   // from the north-west, high
const SHALLOW: vec3<f32> = vec3<f32>(0.30, 0.50, 0.56);
const DEEP: vec3<f32> = vec3<f32>(0.09, 0.20, 0.32);
const SKY: vec3<f32> = vec3<f32>(0.62, 0.72, 0.80);
const HAZE: vec3<f32> = vec3<f32>(0.70, 0.76, 0.82);

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

fn h_uv(rel: vec2<f32>) -> vec2<f32> {
    return vec2<f32>((rel.x - g3.hmap.x) / g3.hmap.z, (g3.hmap.y - rel.y) / g3.hmap.w);
}

/// Height in metres (sea floor negative).
fn height_raw(rel: vec2<f32>) -> f32 {
    let uv = h_uv(rel);
    let h = textureSampleLevel(heights, samp, uv, 0.0).r;
    // beyond the raster: open ocean
    return select(-4000.0, h, all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0)));
}

/// Drawn height of the ground: the sea is flat at zero.
fn ground_z(rel: vec2<f32>) -> f32 {
    return max(height_raw(rel), 0.0) * g3.misc.x;
}

fn overlay_uv(rel: vec2<f32>) -> vec2<f32> {
    let px = vec2<f32>(g3.ov_half.x + (rel.x - g3.ov.x) * g3.ov.z, g3.ov_half.y - (rel.y - g3.ov.y) * g3.ov.z);
    return px * g3.ov_half.z;
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

/// Paper grain, as on the flat map (shader.wgsl), so the blend into 3D does not jump.
fn paper_grain(w: vec2<f32>, k: f32) -> f32 {
    let stains = vnoise(w / 900000.0) * 0.6 + vnoise(w / 310000.0) * 0.4;
    let fibre = vnoise(w / 45000.0) * 0.6 + vnoise(w / 16000.0) * 0.4;
    return 1.0 + ((stains - 0.5) * 0.06 + (fibre - 0.5) * 0.05) * k;
}

/// Fade distant ground toward a light haze (only in the tilted view).
fn hazed(rgb: vec3<f32>, pos: vec3<f32>) -> vec3<f32> {
    let d = length(pos - g3.eye.xyz);
    let base = length(g3.eye.xyz);
    let k = smoothstep(base * 1.1, base * 3.0, d) * g3.misc.w;
    return mix(rgb, HAZE, k);
}

// ---------------------------------------------------------------- terrain

struct TOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) rel: vec2<f32>,
    @location(1) z: f32,
};

@vertex
fn vs_terrain(@builtin(vertex_index) vi: u32) -> TOut {
    let n = u32(g3.misc.z);
    let q = vi / 6u;
    let k = vi % 6u;
    var ox = array<u32, 6>(0u, 1u, 1u, 0u, 1u, 0u);
    var oy = array<u32, 6>(0u, 0u, 1u, 0u, 1u, 1u);
    let gx = q % n + ox[k];
    let gy = q / n + oy[k];
    let rel = g3.grid.xy + vec2<f32>(f32(gx), f32(gy)) * g3.grid.zw;
    let z = ground_z(rel);
    var o: TOut;
    o.clip = g3.view_proj * vec4<f32>(rel, z, 1.0);
    o.rel = rel;
    o.z = z;
    return o;
}

/// Brightness of the ground from its slope: 1 on flat ground, brighter facing the light.
fn land_shade(rel: vec2<f32>) -> f32 {
    let e = g3.hpx.xy;
    let hx = (max(height_raw(rel + vec2<f32>(e.x, 0.0)), 0.0) - max(height_raw(rel - vec2<f32>(e.x, 0.0)), 0.0)) / (2.0 * e.x);
    let hy = (max(height_raw(rel + vec2<f32>(0.0, e.y)), 0.0) - max(height_raw(rel - vec2<f32>(0.0, e.y)), 0.0)) / (2.0 * e.y);
    let ex = g3.misc.x;
    let n = normalize(vec3<f32>(-hx * ex, -hy * ex, 1.0));
    let l = normalize(LIGHT);
    return clamp(dot(n, l) / l.z, 0.45, 1.3);
}

/// Slope of the sea surface: a few long swells in different directions, their strength varying
/// from place to place, moving with time. Returns d(height)/dx, d(height)/dy.
fn wave_slope(w: vec2<f32>, t: f32, scale: f32) -> vec2<f32> {
    let dirs = array<vec2<f32>, 5>(vec2<f32>(0.80, 0.60), vec2<f32>(-0.50, 0.87), vec2<f32>(0.97, -0.26), vec2<f32>(-0.28, -0.96), vec2<f32>(0.31, 0.95));
    let lens = array<f32, 5>(1.0, 0.71, 0.53, 0.37, 0.29);
    let slopes = array<f32, 5>(0.050, 0.040, 0.025, 0.015, 0.010);
    var s = vec2<f32>(0.0);
    for (var k = 0; k < 5; k++) {
        let len = lens[k] * scale;
        // bend the crests with noise so they are not straight lines, and vary their strength
        let warp = vec2<f32>(vnoise(w / (len * 2.1) + vec2<f32>(f32(k) * 7.0, 1.0)), vnoise(w / (len * 2.1) + vec2<f32>(5.0, f32(k) * 3.0))) - 0.5;
        let q = w + warp * len * 1.4;
        let patchy = smoothstep(0.25, 0.85, vnoise(w / (len * 4.3) + vec2<f32>(f32(k) * 17.0, 3.0)));
        let phase = dot(dirs[k], q) / len * 6.2831853 - t * (1.3 - 0.15 * f32(k));
        s += dirs[k] * cos(phase) * slopes[k] * (0.3 + 1.4 * patchy);
    }
    return s;
}

fn water(rel: vec2<f32>, pos: vec3<f32>, depth: f32) -> vec3<f32> {
    var col = mix(SHALLOW, DEEP, sqrt(clamp(depth / 4000.0, 0.0, 1.0)));
    let w = vec2<f32>(g3.center.xy) + rel;
    // calmer with distance: far waves are smaller than a pixel and would only shimmer
    let far = clamp(length(g3.eye.xyz) * 1.3 / length(pos - g3.eye.xyz), 0.2, 1.0);
    let sl = wave_slope(w, g3.eye.w, max(g3.misc.y * 70.0, 3000.0)) * 0.75 * far;
    let n = normalize(vec3<f32>(-sl.x, -sl.y, 1.0));
    let l = normalize(LIGHT);
    let v = normalize(g3.eye.xyz - pos);
    let spec = pow(max(dot(n, normalize(l + v)), 0.0), 60.0) * 0.28;
    // the sea looks lighter, reflecting the sky, when seen at a low angle
    let sheen = pow(1.0 - max(dot(n, v), 0.0), 4.0) * 0.45;
    col = col * (0.82 + 0.25 * dot(n, l));
    return mix(col, SKY, sheen) + vec3<f32>(spec);
}

@fragment
fn fs_terrain(i: TOut) -> @location(0) vec4<f32> {
    let t = g3.ov_half.w;
    let o = textureSampleLevel(overlay, samp, overlay_uv(i.rel), 0.0);
    let pos = vec3<f32>(i.rel, i.z);
    let depth = -height_raw(i.rel);
    let world = vec2<f32>(g3.center.xy) + i.rel;
    let paper = paper_sea_color(depth, fwidth(depth)) * paper_grain(world, 1.0 - t);
    let sea = mix(paper, water(i.rel, pos, depth), t);
    // the overlay is drawn over a transparent sea, so its alpha says how much land (or line) covers
    let land = o.rgb * mix(1.0, land_shade(i.rel), t);
    let rgb = sea * (1.0 - o.a) + land;
    return vec4<f32>(hazed(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), pos), 1.0);
}

// ---------------------------------------------------------------- models (trees, buildings)

struct MOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) pos: vec3<f32>,
};

fn cell_base(cells: vec4<i32>, ii: u32) -> vec3<i32> {
    let nx = u32(cells.z);
    let c = vec2<i32>(cells.x + i32(ii % nx), cells.y + i32(ii / nx));
    return vec3<i32>(c, cells.w);
}

fn hidden() -> MOut {
    var o: MOut;
    o.clip = vec4<f32>(0.0, 0.0, -2.0, 1.0);   // outside the depth range: dropped
    o.color = vec3<f32>(0.0);
    o.pos = vec3<f32>(0.0);
    return o;
}

fn lit(color: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    return color * (0.50 + 0.62 * max(dot(normalize(normal), normalize(LIGHT)), 0.0));
}

fn is_urban(rel: vec2<f32>) -> bool {
    return textureSampleLevel(urban_mask, samp, overlay_uv(rel), 0.0).r > 0.5;
}

// A tree is a six-sided cone: 6 triangles, 18 vertices.
@vertex
fn vs_tree(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> MOut {
    let cb = cell_base(g3.trees, ii);
    let c = cb.xy;
    let s = f32(cb.z);
    let base = vec2<f32>(c * cb.z - g3.center.xy) + vec2<f32>(hash2(c), hash2(c + vec2<i32>(7, 13))) * s;
    let ground = textureSampleLevel(land, samp, h_uv(base), 0.0).rgb;
    let density = smoothstep(0.08, 0.16, ground.g - ground.r) * smoothstep(0.72, 0.62, ground.r);
    if (hash2(c + vec2<i32>(11, 2)) >= density * 0.9 || height_raw(base) <= 0.0 || is_urban(base)) {
        return hidden();
    }
    let size = g3.sizes.x * (0.7 + 0.6 * hash2(c + vec2<i32>(3, 5)));
    let r = size * 0.42;
    let top = size * 1.25;
    let face = vi / 3u;
    let corner = vi % 3u;
    let tau = 6.2831853;
    let a0 = f32(face) / 6.0 * tau + hash2(c) * tau;
    let a1 = a0 + tau / 6.0;
    let am = (a0 + a1) * 0.5;
    var p: vec3<f32>;
    if (corner == 0u) {
        p = vec3<f32>(0.0, 0.0, top);
    } else if (corner == 1u) {
        p = vec3<f32>(cos(a0) * r, sin(a0) * r, size * 0.12);
    } else {
        p = vec3<f32>(cos(a1) * r, sin(a1) * r, size * 0.12);
    }
    let normal = vec3<f32>(cos(am) * top, sin(am) * top, r);
    let shade = 0.75 + 0.5 * hash2(c + vec2<i32>(17, 1));
    let green = vec3<f32>(0.16, 0.30, 0.13) * shade;
    let world = vec3<f32>(base, ground_z(base)) + p;
    var o: MOut;
    o.clip = g3.view_proj * vec4<f32>(world, 1.0);
    o.color = lit(green, normal);
    o.pos = world;
    return o;
}

// A building is a box without a floor: roof and four walls, 30 vertices.
@vertex
fn vs_building(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> MOut {
    let cb = cell_base(g3.blds, ii);
    let c = cb.xy;
    let s = f32(cb.z);
    let base = vec2<f32>(c * cb.z - g3.center.xy) + (vec2<f32>(hash2(c + vec2<i32>(5, 9)), hash2(c + vec2<i32>(2, 4))) * 0.5 + 0.25) * s;
    if (!is_urban(base) || hash2(c + vec2<i32>(1, 8)) > 0.8) {
        return hidden();
    }
    let w = g3.sizes.y * (0.6 + 0.4 * hash2(c + vec2<i32>(6, 6)));
    let d = g3.sizes.y * (0.6 + 0.4 * hash2(c + vec2<i32>(4, 1)));
    let hgt = g3.sizes.y * (0.5 + 1.7 * hash2(c + vec2<i32>(9, 3)) * hash2(c + vec2<i32>(3, 9)));
    // corners of each face (x, y, z in 0..1) as two triangles
    let face = vi / 6u;
    let k = vi % 6u;
    var u = array<f32, 6>(0.0, 1.0, 1.0, 0.0, 1.0, 0.0);
    var v = array<f32, 6>(0.0, 0.0, 1.0, 0.0, 1.0, 1.0);
    var p: vec3<f32>;
    var normal: vec3<f32>;
    switch face {
        case 0u: { p = vec3<f32>(u[k], v[k], 1.0); normal = vec3<f32>(0.0, 0.0, 1.0); }
        case 1u: { p = vec3<f32>(u[k], 0.0, v[k]); normal = vec3<f32>(0.0, -1.0, 0.0); }
        case 2u: { p = vec3<f32>(1.0, u[k], v[k]); normal = vec3<f32>(1.0, 0.0, 0.0); }
        case 3u: { p = vec3<f32>(1.0 - u[k], 1.0, v[k]); normal = vec3<f32>(0.0, 1.0, 0.0); }
        default: { p = vec3<f32>(0.0, 1.0 - u[k], v[k]); normal = vec3<f32>(-1.0, 0.0, 0.0); }
    }
    let local = vec3<f32>((p.x - 0.5) * w, (p.y - 0.5) * d, p.z * hgt);
    var color = vec3<f32>(0.80, 0.77, 0.70) * (0.85 + 0.2 * hash2(c + vec2<i32>(8, 8)));
    if (face == 0u) {
        color = mix(vec3<f32>(0.55, 0.30, 0.24), vec3<f32>(0.42, 0.42, 0.44), hash2(c + vec2<i32>(2, 2)));
    }
    let world = vec3<f32>(base, ground_z(base)) + local;
    var o: MOut;
    o.clip = g3.view_proj * vec4<f32>(world, 1.0);
    o.color = lit(color, normal);
    o.pos = world;
    return o;
}

@fragment
fn fs_model(i: MOut) -> @location(0) vec4<f32> {
    return vec4<f32>(hazed(i.color, i.pos), 1.0);
}
