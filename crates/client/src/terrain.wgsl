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

/// How wooded the ground is (0..1), read from the land-cover colours: green clearly above red,
/// and not too light. Checked on samples of taiga, rainforest, farmland, steppe and desert.
fn forest_at(rel: vec2<f32>) -> f32 {
    let c = textureSampleLevel(land, samp, h_uv(rel), 0.0).rgb;
    return smoothstep(0.08, 0.16, c.g - c.r) * smoothstep(0.72, 0.62, c.r);
}

const FOREST_FLOOR: vec3<f32> = vec3<f32>(0.56, 0.66, 0.54);

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
    // forest floor: the ground under the trees is darker and mottled, so a forest reads as one
    // mass from far away and the trees do not stand on bare bright ground close up
    let wood = forest_at(i.rel) * (0.75 + 0.5 * vnoise(world / 2600.0)) * smoothstep(0.25, 0.6, vnoise(world / 9000.0) + forest_at(i.rel) * 0.6);
    let floor_tint = mix(vec3<f32>(1.0), FOREST_FLOOR, clamp(wood, 0.0, 1.0) * 0.8 * t);
    // towns: a grey, slightly mottled ground under the buildings
    let town = textureSampleLevel(urban_mask, samp, overlay_uv(i.rel), 0.0).r * t;
    let street = mix(vec3<f32>(1.0), vec3<f32>(0.80, 0.79, 0.78) * (0.9 + 0.2 * vnoise(world / 700.0)), town * 0.7);
    let land = o.rgb * floor_tint * street * mix(1.0, land_shade(i.rel), t);
    let rgb = sea * (1.0 - o.a) + land;
    return vec4<f32>(hazed(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), pos), 1.0);
}

// ---------------------------------------------------------------- models (trees, buildings)
//
// The look aimed for is the Hearts of Iron IV map: forests are dense carpets of small, dark,
// muted trees (pointed conifers in the north and on mountains, rounded crowns elsewhere), and towns
// are tight clusters of low grey houses with dark pitched roofs, with taller flat-roofed blocks in
// the middle of big cities. Everything is generated here from the instance number; there are no
// model files.

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

/// Flat shading with a strong ambient part, so the shaded side stays readable (muted, not black).
fn lit(color: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    return color * (0.62 + 0.62 * max(dot(normalize(normal), normalize(LIGHT)), 0.0));
}

fn urban_at(rel: vec2<f32>) -> f32 {
    return textureSampleLevel(urban_mask, samp, overlay_uv(rel), 0.0).r;
}

/// Latitude in degrees from Equal Earth y (rough: within about two degrees, enough to pick trees).
fn latitude_deg(world_y: f32) -> f32 {
    let u = clamp(world_y / 8392000.0, -1.0, 1.0);
    return 90.0 * u * (0.80 + 0.20 * u * u);
}

// A tree is two six-sided cones, 12 triangles, 36 vertices. A conifer stacks two pointed tiers;
// a broadleaf tree is a lumpy crown (a cone on top of an upside-down cone sharing one uneven ring).
@vertex
fn vs_tree(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> MOut {
    let cb = cell_base(g3.trees, ii);
    let c = cb.xy;
    let s = f32(cb.z);
    let base = vec2<f32>(c * cb.z - g3.center.xy) + vec2<f32>(hash2(c), hash2(c + vec2<i32>(7, 13))) * s;
    let world_xy = vec2<f32>(g3.center.xy) + base;
    // woods have clearings and ragged edges: thin the trees out with slow noise
    let cover = forest_at(base) * smoothstep(0.22, 0.5, vnoise(world_xy / 9000.0) + forest_at(base) * 0.6);
    let ground_h = height_raw(base);
    if (g3.sizes.x <= 0.0 || hash2(c + vec2<i32>(11, 2)) >= cover * 1.35 || ground_h <= 0.0 || urban_at(base) > 0.3) {
        return hidden();
    }
    // which kind: conifers toward the poles and up the mountains, rainforest near the equator
    let lat = abs(latitude_deg(world_xy.y));
    let cold = smoothstep(49.0, 60.0, lat + ground_h / 60.0 + (vnoise(world_xy / 40000.0) - 0.5) * 12.0);
    let conifer = hash2(c + vec2<i32>(23, 4)) < cold;
    let jungle = 1.0 - smoothstep(14.0, 24.0, lat);
    let size = g3.sizes.x * (0.75 + 0.5 * hash2(c + vec2<i32>(3, 5)));

    let face = vi / 3u;
    let corner = vi % 3u;
    let tier = face / 6u;     // 0: upper cone, 1: lower cone
    let j = face % 6u;
    let tau = 6.2831853;
    let turn = hash2(c) * tau;
    var apex: vec3<f32>;
    var ring_z: f32;
    var rad: f32;
    var top: f32;
    if (conifer) {
        top = size * 1.45;
        if (tier == 0u) { apex = vec3<f32>(0.0, 0.0, top); ring_z = size * 0.62; rad = size * 0.27; }
        else { apex = vec3<f32>(0.0, 0.0, size * 0.98); ring_z = size * 0.10; rad = size * 0.40; }
    } else {
        top = size * (0.95 + 0.25 * jungle);
        let lean = (vec2<f32>(hash2(c + vec2<i32>(31, 7)), hash2(c + vec2<i32>(5, 41))) - 0.5) * size * 0.25;
        ring_z = top * 0.50;
        rad = size * 0.56;
        if (tier == 0u) { apex = vec3<f32>(lean, top); } else { apex = vec3<f32>(0.0, 0.0, top * 0.10); }
    }
    // the two ring corners of this face; a broadleaf ring is uneven so crowns look lumpy
    let k0 = i32(j);
    let k1 = i32((j + 1u) % 6u);
    var r0 = rad;
    var r1 = rad;
    if (!conifer) {
        r0 = rad * (0.72 + 0.5 * hash2(c + vec2<i32>(k0 * 3, 19)));
        r1 = rad * (0.72 + 0.5 * hash2(c + vec2<i32>(k1 * 3, 19)));
    }
    let a0 = f32(k0) / 6.0 * tau + turn;
    let a1 = f32(k1) / 6.0 * tau + turn;
    let p0 = vec3<f32>(cos(a0) * r0, sin(a0) * r0, ring_z);
    let p1 = vec3<f32>(cos(a1) * r1, sin(a1) * r1, ring_z);
    var p = apex;
    if (corner == 1u) { p = p0; } else if (corner == 2u) { p = p1; }
    // face normal, pointing away from the trunk
    var normal = cross(p0 - apex, p1 - apex);
    let outward = vec3<f32>(cos((a0 + a1) * 0.5), sin((a0 + a1) * 0.5), 0.0);
    if (dot(normal, outward) < 0.0) { normal = -normal; }

    // muted greens: blue-dark conifers, olive broadleaf, deeper green rainforest
    var green = mix(vec3<f32>(0.27, 0.39, 0.19), vec3<f32>(0.17, 0.34, 0.16), jungle);
    if (conifer) { green = vec3<f32>(0.19, 0.32, 0.22); }
    green *= 0.82 + 0.36 * hash2(c + vec2<i32>(17, 1));
    // darker toward the bottom of the crown, where neighbours shade it
    let ao = mix(0.62, 1.0, smoothstep(0.0, top * 0.8, p.z));
    let world = vec3<f32>(base, ground_z(base)) + p;
    var o: MOut;
    o.clip = g3.view_proj * vec4<f32>(world, 1.0);
    o.color = lit(green, normal) * ao;
    o.pos = world;
    return o;
}

// A building is four walls, two roof slopes and two gable ends: 42 vertices. With a ridge height
// of zero the roof is flat (the tall blocks in a city centre).
@vertex
fn vs_building(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> MOut {
    let cb = cell_base(g3.blds, ii);
    let c = cb.xy;
    let s = f32(cb.z);
    let centre = vec2<f32>(c * cb.z - g3.center.xy) + 0.5 * s;
    let base = centre + (vec2<f32>(hash2(c + vec2<i32>(5, 9)), hash2(c + vec2<i32>(2, 4))) - 0.5) * s * 0.22;
    // how deep inside the town this spot is: the mask a few plots away in eight directions
    var inside = 0.0;
    for (var k = 0; k < 8; k++) {
        let a = f32(k) * 0.7853982;
        inside += urban_at(base + vec2<f32>(cos(a), sin(a)) * s * 5.0);
    }
    inside = inside / 8.0;
    // a few empty plots (squares, yards), more of them toward the edge
    let empty = hash2(c + vec2<i32>(1, 8)) > mix(0.62, 0.90, inside);
    if (g3.sizes.y <= 0.0 || urban_at(base) < 0.5 || empty || height_raw(base) <= 0.0) {
        return hidden();
    }
    let unit = g3.sizes.y;
    // centre of a big town: taller blocks with flat roofs
    let core = smoothstep(0.93, 1.0, inside);
    let tower = hash2(c + vec2<i32>(9, 3)) < core * 0.45;
    var w = unit * (0.95 + 0.45 * hash2(c + vec2<i32>(6, 6)));     // along the ridge
    var d = unit * (0.62 + 0.25 * hash2(c + vec2<i32>(4, 1)));
    var wall = unit * (0.30 + 0.22 * hash2(c + vec2<i32>(3, 9)));
    var ridge = d * 0.42;
    if (tower) {
        w = unit * (0.85 + 0.45 * hash2(c + vec2<i32>(6, 6)));
        d = unit * (0.75 + 0.35 * hash2(c + vec2<i32>(4, 1)));
        wall = unit * (0.65 + 1.0 * hash2(c + vec2<i32>(3, 9)) * hash2(c + vec2<i32>(12, 5)));
        ridge = 0.0;
    }
    // houses of one neighbourhood (8 x 8 plots) face the same way
    let block = vec2<i32>(c.x >> 3, c.y >> 3);
    let ang = floor(hash2(block + vec2<i32>(40, 2)) * 4.0) * 0.3927 + f32((c.x + c.y) & 1) * 1.5708 * step(0.7, hash2(c + vec2<i32>(13, 13)));
    let ca = cos(ang);
    let sa = sin(ang);

    var p: vec3<f32>;        // local position: x along the ridge, y across, z up
    var normal: vec3<f32>;
    var is_roof = false;
    var u6 = array<f32, 6>(0.0, 1.0, 1.0, 0.0, 1.0, 0.0);
    var v6 = array<f32, 6>(0.0, 0.0, 1.0, 0.0, 1.0, 1.0);
    if (vi < 24u) {
        // walls
        let face = vi / 6u;
        let k = vi % 6u;
        let uu = u6[k] - 0.5;
        let vv = v6[k] * wall;
        switch face {
            case 0u: { p = vec3<f32>(uu * w, -0.5 * d, vv); normal = vec3<f32>(0.0, -1.0, 0.0); }
            case 1u: { p = vec3<f32>(0.5 * w, uu * d, vv); normal = vec3<f32>(1.0, 0.0, 0.0); }
            case 2u: { p = vec3<f32>(-uu * w, 0.5 * d, vv); normal = vec3<f32>(0.0, 1.0, 0.0); }
            default: { p = vec3<f32>(-0.5 * w, -uu * d, vv); normal = vec3<f32>(-1.0, 0.0, 0.0); }
        }
    } else if (vi < 36u) {
        // two roof slopes, from the eaves (a little wider than the walls) up to the ridge
        let side = select(-1.0, 1.0, vi >= 30u);
        let k = (vi - 24u) % 6u;
        let along = (u6[k] - 0.5) * w * 1.06;
        let up = v6[k];
        p = vec3<f32>(along, side * 0.53 * d * (1.0 - up), wall + ridge * up);
        normal = vec3<f32>(0.0, side * ridge, 0.5 * d + 0.0001);
        is_roof = true;
    } else {
        // gable ends: the triangles under the roof at both ends
        let end = select(-1.0, 1.0, vi >= 39u);
        let k = (vi - 36u) % 3u;
        var q = vec2<f32>(-0.5 * d, wall);
        if (k == 1u) { q = vec2<f32>(0.5 * d, wall); } else if (k == 2u) { q = vec2<f32>(0.0, wall + ridge); }
        p = vec3<f32>(end * 0.5 * w, q.x, q.y);
        normal = vec3<f32>(end, 0.0, 0.0);
    }
    let local = vec3<f32>(p.x * ca - p.y * sa, p.x * sa + p.y * ca, p.z);
    let n = vec3<f32>(normal.x * ca - normal.y * sa, normal.x * sa + normal.y * ca, normal.z);

    // muted palette: plaster and concrete walls, slate, brown and dull red roofs
    let tone = hash2(c + vec2<i32>(8, 8));
    var color = mix(vec3<f32>(0.66, 0.64, 0.59), vec3<f32>(0.78, 0.75, 0.68), tone);
    if (tower) { color = mix(vec3<f32>(0.58, 0.58, 0.57), vec3<f32>(0.72, 0.71, 0.68), tone); }
    if (is_roof) {
        let pick = hash2(c + vec2<i32>(2, 2));
        color = vec3<f32>(0.30, 0.31, 0.34);                                  // slate
        if (pick > 0.45) { color = vec3<f32>(0.37, 0.31, 0.27); }             // brown tile
        if (pick > 0.80) { color = vec3<f32>(0.44, 0.32, 0.28); }             // dull red tile
        if (tower) { color = vec3<f32>(0.40, 0.40, 0.42) * (0.9 + 0.2 * tone); }
    } else {
        // walls are darker near the ground, as if the street were in shadow
        color *= mix(0.72, 1.0, smoothstep(0.0, wall, p.z));
    }
    let world = vec3<f32>(base, ground_z(base)) + local;
    var o: MOut;
    o.clip = g3.view_proj * vec4<f32>(world, 1.0);
    o.color = lit(color, n);
    o.pos = world;
    return o;
}

@fragment
fn fs_model(i: MOut) -> @location(0) vec4<f32> {
    return vec4<f32>(hazed(i.color, i.pos), 1.0);
}
