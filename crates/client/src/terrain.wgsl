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
@group(0) @binding(2) var urban_mask: texture_2d<f32>;   // r: town, g: keep clear (road, railway, river), b: river
@group(0) @binding(3) var heights: texture_2d<f32>;
@group(0) @binding(4) var land: texture_2d<f32>;
@group(0) @binding(5) var samp: sampler;
@group(0) @binding(6) var wet: texture_2d<f32>;          // the big wetlands, in latitude and longitude (wetlands.rs)

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

/// How steep the ground is: metres of height per metre, in x and in y.
fn slope_at(rel: vec2<f32>) -> vec2<f32> {
    let e = g3.hpx.xy;
    let hx = (max(height_raw(rel + vec2<f32>(e.x, 0.0)), 0.0) - max(height_raw(rel - vec2<f32>(e.x, 0.0)), 0.0)) / (2.0 * e.x);
    let hy = (max(height_raw(rel + vec2<f32>(0.0, e.y)), 0.0) - max(height_raw(rel - vec2<f32>(0.0, e.y)), 0.0)) / (2.0 * e.y);
    return vec2<f32>(hx, hy);
}

/// How likely the ground is farmed (0..1), guessed from the land-cover colour and the relief:
/// some green in it (not desert sand, not ice), not forest, not high, not steep. There is no
/// farmland layer in the map yet; when there is, this should read it instead.
fn arable_at(rel: vec2<f32>) -> f32 {
    let c = textureSampleLevel(land, samp, h_uv(rel), 0.0).rgb;
    let h = height_raw(rel);
    let green = smoothstep(-0.03, 0.04, c.g - c.r) * smoothstep(0.90, 0.78, c.r);
    let low = 1.0 - smoothstep(1500.0, 2600.0, h);
    let flat = 1.0 - smoothstep(0.05, 0.14, length(slope_at(rel)));
    return green * (1.0 - forest_at(rel)) * low * flat * step(0.0, h);
}

/// Is this ground marsh (0..1)? Read from the wetlands picture at latitude and longitude `ll`
/// (degrees), with the edge frayed by noise so a marsh is not a clean ellipse.
fn wet_at(world: vec2<f32>, ll: vec2<f32>) -> f32 {
    let q = ll + (vec2<f32>(vnoise(world / 23000.0), vnoise(world / 23000.0 + vec2<f32>(9.0, 4.0))) - 0.5) * 0.5;
    let v = textureSampleLevel(wet, samp, vec2<f32>((q.y + 180.0) / 360.0, (90.0 - q.x) / 180.0), 0.0).r;
    return smoothstep(0.35, 0.65, v + (vnoise(world / 6000.0) - 0.5) * 0.3);
}

// Farmland, painted on the ground as a patchwork of fields, in the way of each part of the world:
//   Europe and most places: blocks at all angles, cut into strips of wheat, green crops, plough
//     and stubble, with dark hedges between
//   Russia: very large straight-edged fields
//   North America, Australia: big square sections, some with the green circle of a pivot sprinkler
//   South, East and South-East Asia: small paddies, green or flooded
//   Middle East, Africa: small plots in earth colours
// Returned as a tint to multiply the ground by (1 = no change), so borders, roads and the owner's
// colour drawn on the ground stay visible.
fn field_tint(world: vec2<f32>, kind: i32) -> vec3<f32> {
    let plot = f32(g3.blds.w);
    var scale = 2.0;
    var straight = false;
    var hedge = 0.40;
    var palette = 0;          // 0 temperate, 1 paddies, 2 dry earth
    var pivots = false;
    switch kind {
        case 1: { scale = 4.0; straight = true; hedge = 0.2; }
        case 2: { scale = 1.4; palette = 2; hedge = 0.25; }
        case 3: { scale = 1.3; palette = 2; hedge = 0.25; }
        case 4, 5, 6: { scale = 1.2; palette = 1; hedge = 0.30; }
        case 7: { scale = 3.0; straight = true; hedge = 0.2; pivots = true; }
        case 8: { scale = 2.6; hedge = 0.25; }
        default: {}
    }
    let size = plot * scale;
    let p = world / size;
    let cell = vec2<i32>(floor(p));
    let f = p - floor(p) - 0.5;
    var ang = hash2(cell + vec2<i32>(13, 7)) * 3.14159;
    if (straight) { ang = select(0.0, 1.5708, hash2(cell + vec2<i32>(13, 7)) > 0.5); }
    let q = vec2<f32>(f.x * cos(ang) + f.y * sin(ang), f.y * cos(ang) - f.x * sin(ang));
    // strips across the block, most of them cut once or twice along their length
    let n = 2.0 + floor(hash2(cell + vec2<i32>(5, 3)) * 3.99);
    let u = q.x * n + 8.0;
    let strip = i32(floor(u));
    let m = 1.0 + floor(hash2(cell * 7 + vec2<i32>(strip, 31)) * 2.99);
    let v = q.y * m + 8.0 + hash2(cell * 5 + vec2<i32>(strip, 2));
    let sub = i32(floor(v));
    let pick = hash2(cell * vec2<i32>(31, 17) + vec2<i32>(strip * 7 + sub * 13, strip * 3 - sub * 5));
    var col = vec3<f32>(0.80, 0.72, 0.44);                               // ripe grain
    if (palette == 0) {
        if (pick > 0.26) { col = vec3<f32>(0.50, 0.62, 0.34); }          // green crop
        if (pick > 0.50) { col = vec3<f32>(0.41, 0.54, 0.30); }          // darker green: pasture, beet
        if (pick > 0.66) { col = vec3<f32>(0.55, 0.45, 0.33); }          // ploughed
        if (pick > 0.80) { col = vec3<f32>(0.78, 0.74, 0.56); }          // stubble
        if (pick > 0.95) { col = vec3<f32>(0.84, 0.78, 0.32); }          // rapeseed
    } else if (palette == 1) {
        col = vec3<f32>(0.46, 0.60, 0.40);
        if (pick > 0.35) { col = vec3<f32>(0.56, 0.66, 0.44); }
        if (pick > 0.60) { col = vec3<f32>(0.48, 0.58, 0.56); }          // flooded
        if (pick > 0.82) { col = vec3<f32>(0.66, 0.66, 0.42); }
    } else {
        col = vec3<f32>(0.72, 0.63, 0.45);
        if (pick > 0.35) { col = vec3<f32>(0.56, 0.60, 0.36); }
        if (pick > 0.60) { col = vec3<f32>(0.63, 0.53, 0.37); }
        if (pick > 0.82) { col = vec3<f32>(0.78, 0.71, 0.51); }
    }
    // field edges: hedges, tracks and ditches
    let fu = u - floor(u);
    let fv = v - floor(v);
    var edge = min(min(fu, 1.0 - fu) / n, min(fv, 1.0 - fv) / m);
    edge = min(edge, 0.5 - max(abs(f.x), abs(f.y)));
    if (pivots && hash2(cell + vec2<i32>(77, 1)) < 0.30) {
        // a pivot sprinkler: a green circle in a square of stubble, one sector of it another crop
        let rr = length(f);
        col = vec3<f32>(0.74, 0.70, 0.52);
        if (rr < 0.46) {
            col = vec3<f32>(0.42, 0.58, 0.32);
            if (fract(atan2(f.y, f.x) / 6.2831853 + hash2(cell + vec2<i32>(3, 3))) < 0.3) { col = vec3<f32>(0.62, 0.66, 0.38); }
        }
        edge = min(abs(rr - 0.46), 0.5 - max(abs(f.x), abs(f.y)));
    }
    let px = g3.misc.y / size;                      // one screen pixel, in blocks
    let line = 1.0 - smoothstep(px * 0.5, px * 1.6, edge);
    col = col * (1.0 - hedge * line) * (0.95 + 0.10 * hash2(cell + vec2<i32>(strip, sub)));
    return clamp(col / vec3<f32>(0.68, 0.66, 0.46), vec3<f32>(0.55), vec3<f32>(1.35));
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
    let sl = slope_at(rel);
    let ex = g3.misc.x;
    let n = normalize(vec3<f32>(-sl.x * ex, -sl.y * ex, 1.0));
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
    // (the ground of a town, with its streets, is already in the overlay: `fs_urban` in shader.wgsl)
    let mask = textureSampleLevel(urban_mask, samp, overlay_uv(i.rel), 0.0);
    // roads, railways and rivers run through the woods in the open, not under the dark floor
    let tint = mix(floor_tint, vec3<f32>(1.0), mask.g);
    var land = o.rgb * tint * mix(1.0, land_shade(i.rel), t);
    // farmland: a patchwork of fields, close up, outside towns and off the roads
    let open_ground = t * (1.0 - mask.r) * (1.0 - mask.g);
    if (open_ground > 0.01 && depth < 0.0) {
        let ll = lat_lon(world);
        let marsh = wet_at(world, ll);
        let field_fade = (1.0 - smoothstep(900.0, 1600.0, g3.misc.y)) * open_ground * (1.0 - marsh);
        let farmed = arable_at(i.rel);
        if (field_fade * farmed > 0.01) {
            let k = farmed * (1.0 - smoothstep(56.0, 62.0, abs(ll.x))) * field_fade;
            land *= mix(vec3<f32>(1.0), field_tint(world, culture(ll)), k * 0.9);
        }
        // marsh: dark wet ground, mottled, with pools and channels of open water among the reeds
        if (marsh > 0.01) {
            let grain = f32(g3.blds.w);
            let k = marsh * open_ground;
            land *= mix(vec3<f32>(1.0), vec3<f32>(0.74, 0.84, 0.70) * (0.88 + 0.24 * vnoise(world / (grain * 0.22))), k);
            let pools = smoothstep(0.50, 0.57, vnoise(world / (grain * 0.9)) * 0.55 + vnoise(world / (grain * 0.31)) * 0.45);
            let channels = 1.0 - smoothstep(0.015, 0.035, abs(vnoise(world / (grain * 1.7) + vec2<f32>(3.0, 8.0)) - 0.5));
            land = mix(land, water(i.rel, pos, 25.0) * o.a, max(pools, channels) * k * 0.9);
        }
    }
    // rivers are water: the same moving surface as the sea, in a shallow colour
    land = mix(land, water(i.rel, pos, 45.0) * o.a, mask.b * t);
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

/// Latitude and longitude in degrees (x: latitude, y: longitude) of a point in Equal Earth metres:
/// the projection run backwards (Newton's method on y, four steps are plenty).
fn lat_lon(w: vec2<f32>) -> vec2<f32> {
    let radius = 6371008.8;
    let a1 = 1.340264;
    let a2 = -0.081106;
    let a3 = 0.000893;
    let a4 = 0.003796;
    let yy = clamp(w.y / radius, -1.3173, 1.3173);
    var th = yy / a1;
    for (var k = 0; k < 4; k++) {
        let t2 = th * th;
        let t6 = t2 * t2 * t2;
        let f = th * (a1 + a2 * t2 + t6 * (a3 + a4 * t2)) - yy;
        let df = a1 + 3.0 * a2 * t2 + t6 * (7.0 * a3 + 9.0 * a4 * t2);
        th = th - f / df;
    }
    let t2 = th * th;
    let t6 = t2 * t2 * t2;
    let df = a1 + 3.0 * a2 * t2 + t6 * (7.0 * a3 + 9.0 * a4 * t2);
    let lon = w.x * 0.8660254 * df / (radius * max(cos(th), 0.05));
    let lat = asin(clamp(sin(th) / 0.8660254, -1.0, 1.0));
    return vec2<f32>(lat, lon) * 57.29578;
}

// Parts of the world with their own look of town (houses and landmark). These are broad boxes of
// latitude and longitude, good enough to tell Europe from the Gulf; they are a display choice only.
const EUROPE: i32 = 0;        // pitched terracotta and slate roofs; church with a spire
const RUSSIA: i32 = 1;        // metal roofs, many grey blocks; church with an onion dome
const MIDDLE_EAST: i32 = 2;   // flat roofs, sand and white walls; mosque with dome and minaret
const AFRICA: i32 = 3;        // low houses, tin roofs, earth walls; small white church
const SOUTH_ASIA: i32 = 4;    // flat concrete roofs, pale walls; temple tower
const EAST_ASIA: i32 = 5;     // dark tile roofs, many tall blocks; pagoda
const SE_ASIA: i32 = 6;       // steep red-brown roofs; gilded stupa
const NEW_WORLD: i32 = 7;     // North America and Australia: shingle roofs; skyscrapers
const LATIN: i32 = 8;         // pastel walls, tile and flat roofs; church with two towers

fn culture(ll: vec2<f32>) -> i32 {
    let lat = ll.x;
    let lon = ll.y;
    if (lon < -30.0) {
        if (lat > 30.0) { return NEW_WORLD; }
        return LATIN;
    }
    if (lon > 110.0 && lat < -10.0) { return NEW_WORLD; }
    if (lon > 97.0 && lon < 150.0 && lat > 20.0 && (lat < 46.0 || (lon > 120.0 && lat < 54.0))) { return EAST_ASIA; }
    if (lon > 92.0 && lat <= 28.0) { return SE_ASIA; }
    if (lon > 72.0 && lon <= 92.0 && lat < 32.0) { return SOUTH_ASIA; }
    if (lon > -18.0 && lon < 52.0 && lat <= 12.0) { return AFRICA; }
    // the south and east shores of the Mediterranean, Turkey, Arabia, Iran, Central Asia
    var north = 35.5;
    if (lon > 10.0) { north = 34.0; }
    if (lon > 26.0) { north = 42.0; }
    if (lon > 45.0) { north = 45.0; }
    if (lon > -18.0 && lon <= 80.0 && lat > 12.0 && lat < north) { return MIDDLE_EAST; }
    if (lon > 27.0) { return RUSSIA; }
    return EUROPE;
}

/// The part of the world a town plot belongs to. Borders are blurred a little with slow noise so
/// styles do not change along a ruler-straight line.
fn culture_at(world_xy: vec2<f32>) -> i32 {
    let wobble = vec2<f32>(vnoise(world_xy / 180000.0), vnoise(world_xy / 180000.0 + vec2<f32>(40.0, 9.0))) - 0.5;
    return culture(lat_lon(world_xy) + wobble * 3.0);
}

/// How houses look in one part of the world.
struct HouseStyle {
    pitch: f32,              // ridge height as a share of the depth (0 = flat roofs only)
    flat: f32,               // share of houses with a flat roof
    wall_a: vec3<f32>,
    wall_b: vec3<f32>,
    wall_c: vec3<f32>,       // the odd one out
    roof_a: vec3<f32>,       // districts lean to roof_a or roof_b
    roof_b: vec3<f32>,
    roof_c: vec3<f32>,
    flat_roof: vec3<f32>,
    towers: f32,             // how many tall blocks, against Europe = 1
    tower_h: f32,            // how tall
    tower_wall: vec3<f32>,
};

fn house_style(kind: i32) -> HouseStyle {
    var st: HouseStyle;
    // Europe
    st.pitch = 0.44; st.flat = 0.0;
    st.wall_a = vec3<f32>(0.70, 0.69, 0.66); st.wall_b = vec3<f32>(0.82, 0.78, 0.69); st.wall_c = vec3<f32>(0.74, 0.67, 0.56);
    st.roof_a = vec3<f32>(0.49, 0.33, 0.27); st.roof_b = vec3<f32>(0.32, 0.33, 0.37); st.roof_c = vec3<f32>(0.40, 0.32, 0.27);
    st.flat_roof = vec3<f32>(0.50, 0.50, 0.50);
    st.towers = 1.0; st.tower_h = 1.0; st.tower_wall = vec3<f32>(0.72, 0.70, 0.66);
    switch kind {
        case 1: {   // Russia
            st.pitch = 0.38; st.flat = 0.15;
            st.wall_a = vec3<f32>(0.66, 0.65, 0.63); st.wall_b = vec3<f32>(0.78, 0.74, 0.62); st.wall_c = vec3<f32>(0.52, 0.42, 0.34);
            st.roof_a = vec3<f32>(0.42, 0.44, 0.46); st.roof_b = vec3<f32>(0.27, 0.36, 0.31); st.roof_c = vec3<f32>(0.43, 0.29, 0.25);
            st.towers = 1.6; st.tower_h = 1.1; st.tower_wall = vec3<f32>(0.68, 0.68, 0.67);
        }
        case 2: {   // Middle East
            st.pitch = 0.0; st.flat = 1.0;
            st.wall_a = vec3<f32>(0.80, 0.72, 0.58); st.wall_b = vec3<f32>(0.88, 0.85, 0.78); st.wall_c = vec3<f32>(0.72, 0.62, 0.48);
            st.flat_roof = vec3<f32>(0.78, 0.73, 0.63);
            st.towers = 0.8; st.tower_h = 0.9; st.tower_wall = vec3<f32>(0.82, 0.78, 0.70);
        }
        case 3: {   // Africa south of the Sahara
            st.pitch = 0.18; st.flat = 0.25;
            st.wall_a = vec3<f32>(0.70, 0.56, 0.42); st.wall_b = vec3<f32>(0.80, 0.76, 0.68); st.wall_c = vec3<f32>(0.62, 0.46, 0.34);
            st.roof_a = vec3<f32>(0.56, 0.57, 0.58); st.roof_b = vec3<f32>(0.50, 0.35, 0.27); st.roof_c = vec3<f32>(0.44, 0.44, 0.45);
            st.flat_roof = vec3<f32>(0.62, 0.58, 0.52);
            st.towers = 0.5; st.tower_h = 0.8;
        }
        case 4: {   // South Asia
            st.pitch = 0.20; st.flat = 0.8;
            st.wall_a = vec3<f32>(0.86, 0.83, 0.76); st.wall_b = vec3<f32>(0.84, 0.74, 0.62); st.wall_c = vec3<f32>(0.66, 0.76, 0.80);
            st.roof_a = vec3<f32>(0.50, 0.35, 0.28); st.roof_b = vec3<f32>(0.52, 0.52, 0.52); st.roof_c = vec3<f32>(0.44, 0.36, 0.30);
            st.flat_roof = vec3<f32>(0.64, 0.63, 0.60);
            st.towers = 1.0; st.tower_h = 1.0; st.tower_wall = vec3<f32>(0.80, 0.78, 0.72);
        }
        case 5: {   // East Asia
            st.pitch = 0.32; st.flat = 0.25;
            st.wall_a = vec3<f32>(0.80, 0.80, 0.78); st.wall_b = vec3<f32>(0.66, 0.66, 0.66); st.wall_c = vec3<f32>(0.58, 0.48, 0.40);
            st.roof_a = vec3<f32>(0.27, 0.28, 0.30); st.roof_b = vec3<f32>(0.29, 0.35, 0.44); st.roof_c = vec3<f32>(0.38, 0.31, 0.27);
            st.towers = 1.5; st.tower_h = 1.35; st.tower_wall = vec3<f32>(0.74, 0.75, 0.76);
        }
        case 6: {   // South-East Asia
            st.pitch = 0.58; st.flat = 0.2;
            st.wall_a = vec3<f32>(0.82, 0.78, 0.68); st.wall_b = vec3<f32>(0.58, 0.46, 0.35); st.wall_c = vec3<f32>(0.86, 0.84, 0.78);
            st.roof_a = vec3<f32>(0.50, 0.30, 0.24); st.roof_b = vec3<f32>(0.45, 0.36, 0.30); st.roof_c = vec3<f32>(0.50, 0.50, 0.50);
            st.towers = 0.9; st.tower_h = 1.0;
        }
        case 7: {   // North America, Australia
            st.pitch = 0.36; st.flat = 0.1;
            st.wall_a = vec3<f32>(0.84, 0.83, 0.80); st.wall_b = vec3<f32>(0.78, 0.72, 0.62); st.wall_c = vec3<f32>(0.58, 0.40, 0.34);
            st.roof_a = vec3<f32>(0.31, 0.31, 0.33); st.roof_b = vec3<f32>(0.38, 0.32, 0.28); st.roof_c = vec3<f32>(0.44, 0.45, 0.46);
            st.towers = 1.5; st.tower_h = 1.7; st.tower_wall = vec3<f32>(0.56, 0.62, 0.68);
        }
        case 8: {   // Latin America
            st.pitch = 0.30; st.flat = 0.5;
            st.wall_a = vec3<f32>(0.86, 0.82, 0.70); st.wall_b = vec3<f32>(0.84, 0.68, 0.60); st.wall_c = vec3<f32>(0.66, 0.76, 0.82);
            st.roof_a = vec3<f32>(0.52, 0.33, 0.26); st.roof_b = vec3<f32>(0.46, 0.34, 0.28); st.roof_c = vec3<f32>(0.56, 0.55, 0.53);
            st.flat_roof = vec3<f32>(0.66, 0.64, 0.60);
            st.towers = 1.0; st.tower_h = 1.1; st.tower_wall = vec3<f32>(0.78, 0.76, 0.72);
        }
        default: {}
    }
    return st;
}

// A tree is two six-sided cones, 12 triangles, 36 vertices, put together in one of four shapes:
//   BROADLEAF  a lumpy crown: a cone on top of an upside-down cone sharing one uneven ring
//   CONIFER    two stacked pointed tiers (`slim` makes it a narrow spruce or a cypress)
//   UMBRELLA   a thin trunk under a wide flat crown (acacia, stone pine)
//   PALM       a tall thin trunk under a star of fronds
const BROADLEAF: i32 = 0;
const CONIFER: i32 = 1;
const UMBRELLA: i32 = 2;
const PALM: i32 = 3;

struct Crown {
    p: vec3<f32>,        // position relative to the foot of the tree
    normal: vec3<f32>,
    top: f32,            // height of the tree
    trunk: bool,         // this vertex belongs to a trunk
};

fn crown(vi: u32, c: vec2<i32>, size: f32, shape: i32, tall: f32, slim: f32) -> Crown {
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
    var uneven = 0.0;         // how lumpy the ring is
    var star = 1.0;           // radius of every second ring corner (palm fronds)
    var trunk = false;
    switch shape {
        case 1: {
            top = size * 1.45 * tall;
            if (tier == 0u) { apex = vec3<f32>(0.0, 0.0, top); ring_z = top * 0.43; rad = size * 0.27 * slim; }
            else { apex = vec3<f32>(0.0, 0.0, top * 0.68); ring_z = top * 0.07; rad = size * 0.40 * slim; }
        }
        case 2: {
            top = size * 0.95 * tall;
            if (tier == 0u) { apex = vec3<f32>(0.0, 0.0, top); ring_z = top * 0.70; rad = size * 0.66; uneven = 0.4; }
            else { apex = vec3<f32>(0.0); ring_z = top * 0.74; rad = size * 0.07; trunk = true; }
        }
        case 3: {
            top = size * 1.30 * tall;
            if (tier == 0u) { apex = vec3<f32>(0.0, 0.0, top); ring_z = top * 0.80; rad = size * 0.60; star = 0.28; }
            else { apex = vec3<f32>(0.0); ring_z = top * 0.90; rad = size * 0.055; trunk = true; }
        }
        default: {
            top = size * 0.95 * tall;
            let lean = (vec2<f32>(hash2(c + vec2<i32>(31, 7)), hash2(c + vec2<i32>(5, 41))) - 0.5) * size * 0.25;
            ring_z = top * 0.50;
            rad = size * 0.56;
            uneven = 0.5;
            if (tier == 0u) { apex = vec3<f32>(lean, top); } else { apex = vec3<f32>(0.0, 0.0, top * 0.10); }
        }
    }
    // the two ring corners of this face
    let k0 = i32(j);
    let k1 = i32((j + 1u) % 6u);
    var r0 = rad * (1.0 - uneven * 0.56 + uneven * hash2(c + vec2<i32>(k0 * 3, 19)));
    var r1 = rad * (1.0 - uneven * 0.56 + uneven * hash2(c + vec2<i32>(k1 * 3, 19)));
    if ((k0 & 1) == 1) { r0 *= star; }
    if ((k1 & 1) == 1) { r1 *= star; }
    let a0 = f32(k0) / 6.0 * tau + turn;
    let a1 = f32(k1) / 6.0 * tau + turn;
    let p0 = vec3<f32>(cos(a0) * r0, sin(a0) * r0, ring_z);
    let p1 = vec3<f32>(cos(a1) * r1, sin(a1) * r1, ring_z);
    var out: Crown;
    out.p = apex;
    if (corner == 1u) { out.p = p0; } else if (corner == 2u) { out.p = p1; }
    // face normal, pointing away from the trunk
    out.normal = cross(p0 - apex, p1 - apex);
    let outward = vec3<f32>(cos((a0 + a1) * 0.5), sin((a0 + a1) * 0.5), 0.0);
    if (dot(out.normal, outward) < 0.0) { out.normal = -out.normal; }
    out.top = top;
    out.trunk = trunk;
    return out;
}

/// Shade a tree of leaf colour `green`: a little variety from tree to tree, darker toward the
/// bottom of the crown where neighbours shade it, brown trunks.
fn crown_color(k: Crown, c: vec2<i32>, green: vec3<f32>) -> vec3<f32> {
    if (k.trunk) {
        return lit(vec3<f32>(0.36, 0.29, 0.22), k.normal) * 0.9;
    }
    let leaf = green * (0.82 + 0.36 * hash2(c + vec2<i32>(17, 1)));
    return lit(leaf, k.normal) * mix(0.62, 1.0, smoothstep(0.0, k.top * 0.8, k.p.z));
}

// Forests by climate, from latitude, height and how green or dry the ground colour is:
//   far north and high mountains: dark, narrow spruce, thinning out toward the tree line
//   temperate: rounded broadleaf trees with some conifers among them
//   Mediterranean and other dry warm country: scattered stone pines, cypresses and olive-grey trees
//   savanna: scattered flat-topped acacias, some palms
//   rainforest: a tall, dense, deep green canopy with a few giants standing above it
@vertex
fn vs_tree(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> MOut {
    let cb = cell_base(g3.trees, ii);
    let c = cb.xy;
    let s = f32(cb.z);
    let base = vec2<f32>(c * cb.z - g3.center.xy) + vec2<f32>(hash2(c), hash2(c + vec2<i32>(7, 13))) * s;
    let ground_h = height_raw(base);
    if (g3.sizes.x <= 0.0 || ground_h <= 0.0) {
        return hidden();
    }
    let world_xy = vec2<f32>(g3.center.xy) + base;
    let ll = lat_lon(world_xy);
    let lat = abs(ll.x);
    let marsh = wet_at(world_xy, ll) > 0.5;
    let ground = textureSampleLevel(land, samp, h_uv(base), 0.0).rgb;
    let lush = forest_at(base);
    // dry open country with some green in it: a tree here and there (warm latitudes only)
    let scrub = smoothstep(0.02, 0.07, ground.g - ground.r) * smoothstep(0.82, 0.70, ground.r) * (1.0 - smoothstep(40.0, 46.0, lat));
    // woods have clearings and ragged edges: thin the trees out with slow noise
    let tree_line = 1.0 - 0.6 * smoothstep(62.0, 69.0, lat + ground_h / 80.0);
    let wooded = lush * smoothstep(0.22, 0.5, vnoise(world_xy / 9000.0) + lush * 0.6) * 1.35 * tree_line;
    // and farmland has a tree here and there, along the hedges and by the farms
    let scattered = max(scrub * 0.13, arable_at(base) * 0.022 * (1.0 - smoothstep(56.0, 62.0, lat)));
    var cover = max(wooded, scattered);
    // marsh: reed beds with a stunted tree now and then; in the tropics, low swamp forest and mangrove
    if (marsh) { cover = select(0.42, 0.78, lat < 19.0); }
    let m = textureSampleLevel(urban_mask, samp, overlay_uv(base), 0.0);
    if (hash2(c + vec2<i32>(11, 2)) >= cover || m.r > 0.3 || m.g > 0.25) {
        return hidden();
    }
    let open = wooded < scattered || lush < 0.5;      // scattered trees, not a wood
    let pick = hash2(c + vec2<i32>(23, 4));
    let cold = smoothstep(52.0, 62.0, lat + ground_h / 60.0 + (vnoise(world_xy / 40000.0) - 0.5) * 12.0);
    let tropic = 1.0 - smoothstep(14.0, 24.0, lat);
    var size = g3.sizes.x * (0.75 + 0.5 * hash2(c + vec2<i32>(3, 5)));
    var shape = BROADLEAF;
    var tall = 1.0;
    var slim = 1.0;
    var green = vec3<f32>(0.27, 0.39, 0.19);                 // temperate broadleaf
    if (marsh) {
        if (tropic > 0.5) {
            size *= 0.72;
            green = vec3<f32>(0.16, 0.32, 0.18);
        } else if (pick < 0.75) {
            // a clump of reeds: short, thin, straw-green
            shape = CONIFER;
            tall = 0.42;
            slim = 0.55;
            size *= 0.85;
            green = vec3<f32>(0.56, 0.57, 0.34);
        } else {
            // a stunted tree: a small spruce in the north, a willow or alder elsewhere
            shape = select(BROADLEAF, CONIFER, hash2(c + vec2<i32>(2, 77)) < cold);
            size *= 0.6;
            green = select(vec3<f32>(0.33, 0.40, 0.24), vec3<f32>(0.19, 0.30, 0.22), shape == CONIFER);
        }
    } else if (pick < max(cold, 0.10 * (1.0 - tropic))) {
        // spruce and fir: narrower, taller and bluer the farther north
        let north = smoothstep(54.0, 66.0, lat + ground_h / 60.0);
        shape = CONIFER;
        tall = 1.0 + 0.18 * north;
        slim = 1.0 - 0.28 * north;
        green = mix(vec3<f32>(0.19, 0.32, 0.22), vec3<f32>(0.15, 0.27, 0.23), north);
        size *= 1.0 - 0.25 * smoothstep(62.0, 69.0, lat + ground_h / 80.0);
    } else if (tropic > 0.5 && !open) {
        // rainforest: tall crowns, and now and then a giant above the canopy
        tall = 1.25;
        green = mix(vec3<f32>(0.14, 0.33, 0.15), vec3<f32>(0.21, 0.40, 0.15), hash2(c + vec2<i32>(44, 6)));
        if (hash2(c + vec2<i32>(9, 61)) < 0.10) { size *= 1.45; tall = 1.45; }
    } else if (open && lat < 30.0) {
        // savanna: acacias, with palms toward the equator
        shape = select(UMBRELLA, PALM, pick > 1.0 - 0.3 * tropic);
        green = select(vec3<f32>(0.40, 0.43, 0.22), vec3<f32>(0.27, 0.41, 0.19), shape == PALM);
        size *= 1.15;
    } else if (open && lat < 44.0) {
        // Mediterranean: stone pines, cypresses, olive-grey trees
        green = vec3<f32>(0.37, 0.41, 0.27);
        if (pick > 0.75) { shape = CONIFER; slim = 0.5; tall = 1.15; green = vec3<f32>(0.16, 0.27, 0.19); }
        else if (pick > 0.35) { shape = UMBRELLA; green = vec3<f32>(0.24, 0.36, 0.20); }
        size *= 1.1;
    } else {
        // temperate: lighter and yellower in some woods than in others
        green = mix(green, vec3<f32>(0.33, 0.42, 0.18), vnoise(world_xy / 30000.0) * (1.0 - tropic));
        green = mix(green, vec3<f32>(0.17, 0.34, 0.16), tropic);
    }
    let k = crown(vi, c, size, shape, tall, slim);
    let world = vec3<f32>(base, ground_z(base)) + k.p;
    var o: MOut;
    o.clip = g3.view_proj * vec4<f32>(world, 1.0);
    o.color = crown_color(k, c, green);
    o.pos = world;
    return o;
}

// The town plan (the same four functions as in shader.wgsl, which draws the streets; see there).
fn plan_swap(c: vec2<i32>) -> bool {
    return hash2(vec2<i32>(c.x >> 4u, c.y >> 4u) + vec2<i32>(91, 17)) < 0.5;
}
fn cross_street(u: i32) -> bool {
    return (u & 3) == 0 || hash2(vec2<i32>(u, 77)) < 0.16;
}
fn street_half(k: i32) -> f32 {
    return select(0.10, 0.15, (k & 15) == 0);
}
fn is_park(q: vec2<i32>) -> bool {
    return hash2(vec2<i32>(q.x >> 2u, q.y >> 1u) + vec2<i32>(53, 29)) < 0.09;
}

struct HouseV {
    p: vec3<f32>,        // local position: x along the ridge, y across, z up
    normal: vec3<f32>,
    is_roof: bool,
};

/// Vertex `vi` (0..41) of a house: four walls, two roof slopes, two gable ends. `w` is its length
/// along the ridge, `d` its depth, `wall` the wall height, `ridge` the roof height above the walls
/// (0 = flat roof), `eaves` how far the roof overhangs at the ends (1 = not at all).
fn house_vertex(vi: u32, w: f32, d: f32, wall: f32, ridge: f32, eaves: f32) -> HouseV {
    var p: vec3<f32>;        // local position: x along the ridge (the street), y across, z up
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
        let run = (u6[k] - 0.5) * w * eaves;
        let up = v6[k];
        p = vec3<f32>(run, side * 0.54 * d * (1.0 - up), wall + ridge * up);
        normal = vec3<f32>(0.0, side * ridge, 0.5 * d + 0.0001);
        is_roof = true;
    } else {
        // gable ends: the triangles under the roof at both ends
        let end = select(-1.0, 1.0, vi >= 39u);
        let k = (vi - 36u) % 3u;
        var g2 = vec2<f32>(-0.5 * d, wall);
        if (k == 1u) { g2 = vec2<f32>(0.5 * d, wall); } else if (k == 2u) { g2 = vec2<f32>(0.0, wall + ridge); }
        p = vec3<f32>(end * 0.5 * w, g2.x, g2.y);
        normal = vec3<f32>(end, 0.0, 0.0);
    }
    return HouseV(p, normal, is_roof);
}

// Out in the farmland about one plot in twenty has a farm: one long low house with its barn under
// the same roof, in the colours of its part of the world.
fn farmstead(vi: u32, c: vec2<i32>, origin: vec2<f32>, s: f32) -> MOut {
    if (hash2(c + vec2<i32>(61, 23)) > 0.05) {
        return hidden();
    }
    let foot = origin + (0.25 + 0.5 * vec2<f32>(hash2(c + vec2<i32>(6, 6)), hash2(c + vec2<i32>(4, 1)))) * s;
    let m = textureSampleLevel(urban_mask, samp, overlay_uv(foot), 0.0);
    let ll = lat_lon(vec2<f32>(g3.center.xy) + foot);
    if (m.r > 0.2 || m.g > 0.3 || arable_at(foot) * (1.0 - smoothstep(56.0, 62.0, abs(ll.x))) * (1.0 - wet_at(vec2<f32>(g3.center.xy) + foot, ll)) < 0.5) {
        return hidden();
    }
    let st = house_style(culture(ll));
    let unit = g3.sizes.y;
    let tone = hash2(c + vec2<i32>(8, 8));
    let w = unit * (0.42 + 0.22 * hash2(c + vec2<i32>(3, 9)));
    let d = unit * 0.26;
    let wall = unit * 0.14;
    let flat_roof = st.flat > 0.7;
    let ridge = select(d * max(st.pitch, 0.28), 0.0, flat_roof);
    let hv3 = house_vertex(vi, w, d, wall, ridge, 1.05);
    let turned = hash2(c + vec2<i32>(19, 2)) < 0.5;
    let local = vec3<f32>(select(hv3.p.xy, hv3.p.yx, turned), hv3.p.z);
    let n = vec3<f32>(select(hv3.normal.xy, hv3.normal.yx, turned), hv3.normal.z);
    var color = mix(st.wall_a, st.wall_b, tone);
    if (hv3.is_roof) {
        color = select(select(st.roof_a, st.roof_c, tone > 0.6), st.flat_roof, flat_roof) * (0.9 + 0.2 * tone);
    } else {
        color *= mix(0.74, 1.0, smoothstep(0.0, wall, hv3.p.z));
    }
    let world = vec3<f32>(foot, ground_z(foot)) + local;
    var o: MOut;
    o.clip = g3.view_proj * vec4<f32>(world, 1.0);
    o.color = lit(color, n);
    o.pos = world;
    return o;
}

// One building per plot, standing at its street: four walls, two roof slopes and two gable ends,
// 42 vertices. Three kinds: a detached house; a terrace (the houses of a block joined into one
// row, same height and roof); and, deep inside a big town, a taller block with a flat roof (ridge
// height zero). A plot in a park gets a tree instead.
@vertex
fn vs_building(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> MOut {
    let cb = cell_base(g3.blds, ii);
    let c = cb.xy;
    let s = f32(cb.z);
    let origin = vec2<f32>(c * cb.z - g3.center.xy);   // the low corner of the plot
    let centre = origin + 0.5 * s;
    if (g3.sizes.y <= 0.0 || height_raw(centre) <= 0.0) {
        return hidden();
    }
    if (urban_at(centre) < 0.5) {
        return farmstead(vi, c, origin, s);
    }
    // how deep inside the town this plot is: the mask a few plots away in eight directions
    var inside = 0.0;
    for (var k = 0; k < 8; k++) {
        let a = f32(k) * 0.7853982;
        inside += urban_at(centre + vec2<f32>(cos(a), sin(a)) * s * 4.0);
    }
    inside = inside / 8.0;
    let unit = g3.sizes.y;     // the plot size, shrinking to zero as the buildings fade out

    // the plan: q is the plot in plan coordinates (x along the street, y across)
    let swap = plan_swap(c);
    var q = c;
    if (swap) { q = c.yx; }
    let block = vec2<i32>(q.x >> 2u, q.y >> 1u);
    let h1 = hash2(c + vec2<i32>(6, 6));
    let h2 = hash2(c + vec2<i32>(4, 1));
    let h3 = hash2(c + vec2<i32>(3, 9));

    if (is_park(q)) {
        if (vi >= 36u) { return hidden(); }
        let foot2 = origin + (0.3 + 0.4 * vec2<f32>(h1, h2)) * s;
        let hot = abs(lat_lon(vec2<f32>(g3.center.xy) + centre).x) < 32.0;
        let k = crown(vi, c, unit * (0.50 + 0.25 * h3), select(BROADLEAF, PALM, hot && h1 > 0.4), 1.0, 1.0);
        let world = vec3<f32>(foot2, ground_z(foot2)) + k.p;
        var o: MOut;
        o.clip = g3.view_proj * vec4<f32>(world, 1.0);
        o.color = crown_color(k, c, vec3<f32>(0.27, 0.40, 0.19));
        o.pos = world;
        return o;
    }
    // a few empty plots (yards, squares) toward the edge of the town
    if (hash2(c + vec2<i32>(1, 8)) > mix(0.70, 0.97, inside)) {
        return hidden();
    }

    // free length along the street: the plot less any cross street (and its pavement) at either end
    let front = (q.y & 1) == 0;     // the street is on the low side
    let hv = street_half(select(q.y + 1, q.y, front));
    let lo = select(0.0, street_half(q.x) + 0.045, cross_street(q.x));
    let hi = select(0.0, street_half(q.x + 1) + 0.045, cross_street(q.x + 1));
    let avail = 1.0 - lo - hi;

    let st = house_style(culture_at(vec2<f32>(g3.center.xy) + centre));
    let core = smoothstep(0.93, 1.0, inside);
    let tower = hash2(c + vec2<i32>(9, 3)) < core * 0.32 * st.towers;
    let terrace = !tower && hash2(block + vec2<i32>(71, 5)) < 0.40;
    let bh = hash2(block + vec2<i32>(15, 44));       // shared by the houses of one block
    // sizes in plots: w along the street, d across, setback from the pavement
    var w = avail * (0.60 + 0.22 * h1);
    var d = 0.40 + 0.14 * h2;
    var setback = 0.02 + 0.07 * h3;
    var wall = 0.17 + 0.11 * h3;
    var flat_roof = hash2(c + vec2<i32>(27, 14)) < st.flat;
    if (terrace) {
        w = avail + 0.002;
        d = 0.42 + 0.10 * bh;
        setback = 0.025;
        wall = 0.21 + 0.10 * bh;
        flat_roof = hash2(block + vec2<i32>(27, 14)) < st.flat;
    }
    var ridge = d * st.pitch;
    if (flat_roof) { ridge = 0.0; wall *= 1.15; }
    if (tower) {
        w = avail * (0.66 + 0.2 * h1);
        d = 0.50 + 0.18 * h2;
        setback = 0.04;
        wall = (0.45 + 0.85 * h3 * hash2(c + vec2<i32>(12, 5))) * st.tower_h;
        ridge = 0.0;
    }
    let along = lo + avail * 0.5 + (h2 - 0.5) * (avail - w) * 0.8;
    let across_front = hv + 0.045 + setback + d * 0.5;
    let foot_plan = vec2<f32>(along, select(1.0 - across_front, across_front, front));
    let foot = origin + select(foot_plan, foot_plan.yx, swap) * s;
    // no building on a road, a railway or a river: look at the middle and the four sides of the plot
    var blocked = textureSampleLevel(urban_mask, samp, overlay_uv(foot), 0.0).g;
    for (var k = 0; k < 4; k++) {
        let a = f32(k) * 1.5707963;
        blocked = max(blocked, textureSampleLevel(urban_mask, samp, overlay_uv(foot + vec2<f32>(cos(a), sin(a)) * s * 0.32), 0.0).g);
    }
    if (blocked > 0.4) {
        return hidden();
    }
    w *= unit; d *= unit; wall *= unit; ridge *= unit;

    let hv3 = house_vertex(vi, w, d, wall, ridge, select(1.05, 1.0, terrace));
    let p = hv3.p;
    let normal = hv3.normal;
    let is_roof = hv3.is_roof;
    let local = vec3<f32>(select(p.xy, p.yx, swap), p.z);
    let n = vec3<f32>(select(normal.xy, normal.yx, swap), normal.z);

    // muted palette, in the colours of this part of the world. Each district leans to one of two
    // roof colours, with some of a third; the houses of a terrace share one roof.
    let district = vec2<i32>(c.x >> 4u, c.y >> 4u);
    var tone = hash2(c + vec2<i32>(8, 8));
    var pick = hash2(c + vec2<i32>(2, 2));
    if (terrace) { tone = bh; pick = hash2(block + vec2<i32>(2, 2)); }
    var color = mix(st.wall_a, st.wall_b, tone);
    if (pick > 0.7) { color = mix(color, st.wall_c, 0.6); }
    if (tower) { color = st.tower_wall * (0.88 + 0.2 * tone); }
    if (is_roof) {
        let lean_b = hash2(district + vec2<i32>(3, 57)) < 0.35;
        color = select(st.roof_a, st.roof_b, lean_b);
        if (pick > 0.62) { color = st.roof_c; }
        if (pick > 0.86) { color = select(st.roof_b, st.roof_a, lean_b); }
        if (flat_roof) { color = st.flat_roof; }
        color *= 0.90 + 0.2 * tone;
        if (tower) { color = vec3<f32>(0.38, 0.38, 0.40) * (0.9 + 0.2 * tone); }
    } else {
        // walls are darker near the ground, as if the street were in shadow
        color *= mix(0.74, 1.0, smoothstep(0.0, wall, p.z));
    }
    let world = vec3<f32>(foot, ground_z(foot)) + local;
    var o: MOut;
    o.clip = g3.view_proj * vec4<f32>(world, 1.0);
    o.color = lit(color, n);
    o.pos = world;
    return o;
}

// ---------------------------------------------------------------- city landmarks
//
// One per city, on its square (see `vs_plaza` in shader.wgsl), in the style of its part of the
// world. The square is four plots by four. A landmark is up to six parts; a part is a block with eight side faces and a top, whose
// bottom and top outlines are rectangles (4 sides) or octagons (8 sides) of any size. Equal
// outlines give a box or a drum, a top of zero width a roof wedge, of zero size a spire or cone.
// Sizes are in plots: x east, y north, z up.

/// Landmarks are drawn larger than their size in plots says, so they stand out among the houses.
const LANDMARK_SCALE: f32 = 1.7;

fn floor_div(a: i32, b: i32) -> i32 {
    let m = a % b;
    return (a - select(m, m + b, m < 0)) / b;
}
fn landmark_at(pos: vec2<i32>, plot: i32) -> vec2<i32> {
    return vec2<i32>(floor_div(pos.x + plot / 2, plot), floor_div(pos.y + plot / 2, plot)) * plot;
}

struct Part {
    at: vec3<f32>,       // centre of the bottom outline
    hb: vec2<f32>,       // half size of the bottom outline
    ht: vec2<f32>,       // half size of the top outline
    h: f32,              // height
    sides: u32,          // 4 or 8; 0 = no part
    color: vec3<f32>,
};

fn block(x: f32, y: f32, z: f32, hx: f32, hy: f32, h: f32, color: vec3<f32>) -> Part {
    return Part(vec3<f32>(x, y, z), vec2<f32>(hx, hy), vec2<f32>(hx, hy), h, 4u, color);
}
fn taper(x: f32, y: f32, z: f32, hb: vec2<f32>, ht: vec2<f32>, h: f32, sides: u32, color: vec3<f32>) -> Part {
    return Part(vec3<f32>(x, y, z), hb, ht, h, sides, color);
}
fn round_part(x: f32, y: f32, z: f32, rb: f32, rt: f32, h: f32, color: vec3<f32>) -> Part {
    return Part(vec3<f32>(x, y, z), vec2<f32>(rb), vec2<f32>(rt), h, 8u, color);
}
fn no_part() -> Part {
    return Part(vec3<f32>(0.0), vec2<f32>(0.0), vec2<f32>(0.0), 0.0, 0u, vec3<f32>(0.0));
}

fn landmark_part(kind: i32, i: u32, seed: u32) -> Part {
    let pick = hash2(vec2<i32>(i32(seed), 5));
    let gold = vec3<f32>(0.80, 0.65, 0.28);
    switch kind {
        case 1: {   // Russia: a white church with an onion dome, gold or blue
            let white = vec3<f32>(0.88, 0.87, 0.84);
            let dome = select(gold, vec3<f32>(0.26, 0.40, 0.62), pick > 0.55);
            switch i {
                case 0u: { return block(0.0, 0.0, 0.0, 0.52, 0.52, 0.60, white); }
                case 1u: { return round_part(0.0, 0.0, 0.60, 0.25, 0.25, 0.30, white); }
                case 2u: { return round_part(0.0, 0.0, 0.90, 0.25, 0.40, 0.16, dome); }
                case 3u: { return round_part(0.0, 0.0, 1.06, 0.40, 0.05, 0.44, dome); }
                case 4u: { return round_part(0.0, 0.0, 1.50, 0.04, 0.0, 0.26, gold); }
                default: { return block(0.70, 0.0, 0.0, 0.20, 0.28, 0.36, white); }
            }
        }
        case 2: {   // Middle East: a mosque, with a dome and a minaret
            let wall = select(vec3<f32>(0.84, 0.78, 0.64), vec3<f32>(0.90, 0.88, 0.82), pick > 0.5);
            let dome = select(vec3<f32>(0.48, 0.64, 0.62), vec3<f32>(0.80, 0.74, 0.60), pick > 0.7);
            switch i {
                case 0u: { return block(0.12, 0.0, 0.0, 0.60, 0.60, 0.42, wall); }
                case 1u: { return round_part(0.12, 0.0, 0.42, 0.44, 0.36, 0.20, dome); }
                case 2u: { return round_part(0.12, 0.0, 0.62, 0.36, 0.03, 0.26, dome); }
                case 3u: { return round_part(-0.78, 0.50, 0.0, 0.13, 0.11, 1.50, wall); }
                case 4u: { return round_part(-0.78, 0.50, 1.50, 0.15, 0.0, 0.32, dome); }
                default: { return block(-0.62, -0.25, 0.0, 0.16, 0.35, 0.30, wall); }
            }
        }
        case 4: {   // South Asia: a stone temple, a hall and a tall curved tower
            let stone = vec3<f32>(0.80, 0.71, 0.56);
            switch i {
                case 0u: { return block(0.45, 0.0, 0.0, 0.42, 0.38, 0.38, stone); }
                case 1u: { return taper(0.45, 0.0, 0.38, vec2<f32>(0.42, 0.38), vec2<f32>(0.18, 0.16), 0.16, 4u, stone * 0.92); }
                case 2u: { return block(-0.35, 0.0, 0.0, 0.36, 0.36, 0.50, stone); }
                case 3u: { return taper(-0.35, 0.0, 0.50, vec2<f32>(0.36), vec2<f32>(0.25), 0.55, 4u, stone * 0.95); }
                case 4u: { return taper(-0.35, 0.0, 1.05, vec2<f32>(0.25), vec2<f32>(0.08), 0.45, 4u, stone * 0.90); }
                default: { return round_part(-0.35, 0.0, 1.50, 0.10, 0.0, 0.20, gold); }
            }
        }
        case 5: {   // East Asia: a pagoda, three storeys each under a wide dark roof
            let body = select(vec3<f32>(0.58, 0.26, 0.20), vec3<f32>(0.82, 0.80, 0.74), pick > 0.6);
            let roof = vec3<f32>(0.23, 0.24, 0.26);
            switch i {
                case 0u: { return block(0.0, 0.0, 0.0, 0.42, 0.42, 0.32, body); }
                case 1u: { return taper(0.0, 0.0, 0.32, vec2<f32>(0.64), vec2<f32>(0.34), 0.14, 4u, roof); }
                case 2u: { return block(0.0, 0.0, 0.46, 0.32, 0.32, 0.30, body); }
                case 3u: { return taper(0.0, 0.0, 0.76, vec2<f32>(0.52), vec2<f32>(0.24), 0.14, 4u, roof); }
                case 4u: { return block(0.0, 0.0, 0.90, 0.22, 0.22, 0.28, body); }
                default: { return taper(0.0, 0.0, 1.18, vec2<f32>(0.40), vec2<f32>(0.0), 0.34, 4u, roof); }
            }
        }
        case 6: {   // South-East Asia: a gilded stupa on a white base
            let white = vec3<f32>(0.90, 0.88, 0.82);
            switch i {
                case 0u: { return block(0.0, 0.0, 0.0, 0.60, 0.60, 0.18, white); }
                case 1u: { return block(0.0, 0.0, 0.18, 0.45, 0.45, 0.14, white); }
                case 2u: { return round_part(0.0, 0.0, 0.32, 0.38, 0.27, 0.30, gold); }
                case 3u: { return round_part(0.0, 0.0, 0.62, 0.27, 0.07, 0.26, gold); }
                case 4u: { return round_part(0.0, 0.0, 0.88, 0.07, 0.0, 0.62, gold * 1.05); }
                default: { return no_part(); }
            }
        }
        case 7: {   // North America, Australia: a cluster of office towers
            let glass = select(vec3<f32>(0.42, 0.52, 0.62), vec3<f32>(0.50, 0.56, 0.60), pick > 0.5);
            let concrete = vec3<f32>(0.72, 0.71, 0.68);
            switch i {
                case 0u: { return block(0.0, 0.0, 0.0, 0.72, 0.56, 0.24, concrete); }
                case 1u: { return block(-0.26, 0.06, 0.24, 0.26, 0.26, 1.80 + 0.5 * pick, glass); }
                case 2u: { return block(-0.26, 0.06, 2.04 + 0.5 * pick, 0.17, 0.17, 0.45, glass * 0.92); }
                case 3u: { return round_part(-0.26, 0.06, 2.49 + 0.5 * pick, 0.03, 0.0, 0.45, vec3<f32>(0.30)); }
                case 4u: { return block(0.36, -0.10, 0.24, 0.22, 0.28, 1.25, glass * 1.12); }
                default: { return block(0.36, -0.10, 1.49, 0.14, 0.20, 0.14, concrete); }
            }
        }
        case 8: {   // Latin America: a colonial church with two towers at the front
            let wall = select(vec3<f32>(0.88, 0.82, 0.62), vec3<f32>(0.90, 0.88, 0.82), pick > 0.5);
            let tile = vec3<f32>(0.52, 0.33, 0.26);
            switch i {
                case 0u: { return block(0.20, 0.0, 0.0, 0.70, 0.34, 0.60, wall); }
                case 1u: { return taper(0.20, 0.0, 0.60, vec2<f32>(0.73, 0.37), vec2<f32>(0.73, 0.0), 0.26, 4u, tile); }
                case 2u: { return block(-0.62, 0.36, 0.0, 0.20, 0.20, 1.05, wall); }
                case 3u: { return block(-0.62, -0.36, 0.0, 0.20, 0.20, 1.05, wall); }
                case 4u: { return round_part(-0.62, 0.36, 1.05, 0.20, 0.03, 0.28, tile); }
                default: { return round_part(-0.62, -0.36, 1.05, 0.20, 0.03, 0.28, tile); }
            }
        }
        default: {   // Europe, and (smaller, whitewashed, tin roof) Africa: a church with a spire
            var stone = select(vec3<f32>(0.74, 0.71, 0.64), vec3<f32>(0.80, 0.74, 0.62), pick > 0.6);
            var roof = select(vec3<f32>(0.30, 0.31, 0.36), vec3<f32>(0.49, 0.33, 0.27), pick > 0.5);
            var k = 1.0;
            if (kind == 3) { stone = vec3<f32>(0.88, 0.86, 0.80); roof = vec3<f32>(0.50, 0.35, 0.27); k = 0.8; }
            var q: Part;
            switch i {
                case 0u: { q = block(0.15, 0.0, 0.0, 0.75, 0.32, 0.55, stone); }
                case 1u: { q = taper(0.15, 0.0, 0.55, vec2<f32>(0.78, 0.35), vec2<f32>(0.78, 0.0), 0.32, 4u, roof); }
                case 2u: { q = block(-0.80, 0.0, 0.0, 0.23, 0.23, 1.15, stone); }
                case 3u: { q = taper(-0.80, 0.0, 1.15, vec2<f32>(0.26), vec2<f32>(0.0), select(0.85, 0.40, kind == 3), 4u, roof * 0.9); }
                case 4u: { q = round_part(0.95, 0.0, 0.0, 0.26, 0.26, 0.42, stone); }
                default: { q = round_part(0.95, 0.0, 0.42, 0.28, 0.0, 0.22, roof); }
            }
            q.at *= k; q.hb *= k; q.ht *= k; q.h *= k;
            return q;
        }
    }
}

/// Corner `k` (0..7) of an outline of half size `half`: a rectangle (corners and edge middles) or
/// an octagon.
fn outline(k: u32, half: vec2<f32>, sides: u32) -> vec2<f32> {
    var dirs = array<vec2<f32>, 8>(vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0), vec2<f32>(-1.0, 1.0), vec2<f32>(-1.0, 0.0), vec2<f32>(-1.0, -1.0), vec2<f32>(0.0, -1.0), vec2<f32>(1.0, -1.0));
    var d = dirs[k % 8u];
    if (sides == 8u && (k % 2u) == 1u) { d = d * 0.7071; }
    return d * half;
}

@vertex
fn vs_landmark(@builtin(vertex_index) vi: u32, @location(0) pos: vec2<i32>, @location(1) radius: f32, @location(2) seed: u32) -> MOut {
    let plot = g3.blds.w;
    let unit = g3.sizes.y;
    if (unit <= 0.0 || radius < 4.0 * f32(plot)) {
        return hidden();
    }
    let at = landmark_at(pos, plot);
    let foot = vec2<f32>(at - g3.center.xy);
    if (height_raw(foot) <= 0.0) {
        return hidden();
    }
    let kind = culture(lat_lon(vec2<f32>(at)));
    let q = landmark_part(kind, vi / 72u, seed);
    if (q.sides == 0u) {
        return hidden();
    }
    let v = vi % 72u;
    var p: vec3<f32>;
    var normal = vec3<f32>(0.0, 0.0, 1.0);
    if (v < 48u) {
        // side faces: two triangles between corner k and k + 1 of the bottom and top outlines
        let k = v / 6u;
        var e6 = array<u32, 6>(0u, 1u, 1u, 0u, 1u, 0u);
        var t6 = array<f32, 6>(0.0, 0.0, 1.0, 0.0, 1.0, 1.0);
        let b0 = vec3<f32>(outline(k, q.hb, q.sides), 0.0);
        let b1 = vec3<f32>(outline(k + 1u, q.hb, q.sides), 0.0);
        let t0 = vec3<f32>(outline(k, q.ht, q.sides), q.h);
        let t1 = vec3<f32>(outline(k + 1u, q.ht, q.sides), q.h);
        let kk = k + e6[v % 6u];
        p = vec3<f32>(outline(kk, mix(q.hb, q.ht, t6[v % 6u]), q.sides), q.h * t6[v % 6u]);
        normal = cross(b1 - b0, t0 - b0) + cross(t1 - b1, t0 - b1);
        let outward = vec3<f32>(outline(k, vec2<f32>(1.0), 8u) + outline(k + 1u, vec2<f32>(1.0), 8u), 0.0);
        if (dot(normal, outward) < 0.0) { normal = -normal; }
        if (dot(normal, normal) < 1e-12) { normal = outward; }
    } else {
        // the top: a fan around its centre
        let k = (v - 48u) / 3u;
        let corner = (v - 48u) % 3u;
        p = vec3<f32>(0.0, 0.0, q.h);
        if (corner == 1u) { p = vec3<f32>(outline(k, q.ht, q.sides), q.h); }
        if (corner == 2u) { p = vec3<f32>(outline(k + 1u, q.ht, q.sides), q.h); }
    }
    let world = vec3<f32>(foot, ground_z(foot)) + (q.at + p) * unit * LANDMARK_SCALE;
    var o: MOut;
    o.clip = g3.view_proj * vec4<f32>(world, 1.0);
    // a little darker toward the ground, like the houses around
    o.color = lit(q.color, normal) * mix(0.82, 1.0, smoothstep(0.0, 0.5, q.at.z + p.z));
    o.pos = world;
    return o;
}

// ---------------------------------------------------------------- traffic
//
// Little cars and trucks on the roads and trains on the railways (placed each frame by
// `road_traffic` and `rail_traffic` in render.rs). It is 1990: boxy saloons in plain paint, lorries
// with a cab and a box or a tarpaulin, diesel and electric locomotives, brown goods wagons and
// coaches in railway liveries. A vehicle is two boxes: a body, and a cabin, load or roof on it.
// Sizes are shares of the vehicle's length: x along, y across, z up.

struct VBox {
    at: vec3<f32>,       // centre along, centre across, bottom
    half: vec2<f32>,     // half length, half width
    h: f32,
    color: vec3<f32>,
};

fn vehicle_box(ty: u32, part: u32, seed: i32) -> VBox {
    let pick = hash2(vec2<i32>(seed, 3));
    let shade = 0.9 + 0.2 * hash2(vec2<i32>(seed, 8));
    switch ty {
        case 1u: {   // truck: cab at the front, box or tarpaulin behind
            var cab = vec3<f32>(0.62, 0.20, 0.17);
            if (pick > 0.3) { cab = vec3<f32>(0.22, 0.34, 0.55); }
            if (pick > 0.55) { cab = vec3<f32>(0.82, 0.80, 0.74); }
            if (pick > 0.8) { cab = vec3<f32>(0.25, 0.42, 0.28); }
            var load = vec3<f32>(0.80, 0.79, 0.75);
            if (hash2(vec2<i32>(seed, 5)) > 0.5) { load = vec3<f32>(0.40, 0.44, 0.33); }
            if (hash2(vec2<i32>(seed, 5)) > 0.8) { load = vec3<f32>(0.62, 0.52, 0.38); }
            if (part == 0u) { return VBox(vec3<f32>(0.38, 0.0, 0.05), vec2<f32>(0.12, 0.17), 0.26, cab * shade); }
            return VBox(vec3<f32>(-0.13, 0.0, 0.07), vec2<f32>(0.37, 0.18), 0.30, load * shade);
        }
        case 2u: {   // locomotive: a long body in the railway's colour, a grey roof with a raised cab
            var livery = vec3<f32>(0.20, 0.33, 0.24);
            if (pick > 0.3) { livery = vec3<f32>(0.52, 0.18, 0.16); }
            if (pick > 0.55) { livery = vec3<f32>(0.20, 0.28, 0.46); }
            if (pick > 0.8) { livery = vec3<f32>(0.70, 0.44, 0.16); }
            if (part == 0u) { return VBox(vec3<f32>(0.0, 0.0, 0.05), vec2<f32>(0.48, 0.20), 0.36, livery * shade); }
            return VBox(vec3<f32>(0.18, 0.0, 0.41), vec2<f32>(0.26, 0.17), 0.09, vec3<f32>(0.44, 0.44, 0.45));
        }
        case 3u: {   // wagon: a coach (seed bit 2048) in a livery, or a goods wagon in brown, rust or grey
            let coach = (seed & 2048) != 0;
            var body = vec3<f32>(0.42, 0.28, 0.21);
            if (pick > 0.4) { body = vec3<f32>(0.50, 0.32, 0.24); }
            if (pick > 0.7) { body = vec3<f32>(0.46, 0.46, 0.45); }
            var roof = body * 0.8;
            if (coach) {
                body = vec3<f32>(0.22, 0.36, 0.27);
                if (pick > 0.35) { body = vec3<f32>(0.24, 0.32, 0.50); }
                if (pick > 0.65) { body = vec3<f32>(0.76, 0.70, 0.54); }
                if (pick > 0.85) { body = vec3<f32>(0.55, 0.20, 0.18); }
                roof = vec3<f32>(0.48, 0.48, 0.50);
            }
            if (part == 0u) { return VBox(vec3<f32>(0.0, 0.0, 0.05), vec2<f32>(0.47, 0.19), 0.33, body * shade); }
            return VBox(vec3<f32>(0.0, 0.0, 0.38), vec2<f32>(0.47, 0.15), 0.05, roof);
        }
        default: {   // car: a three-box saloon in the paint of the time
            var paint = vec3<f32>(0.78, 0.77, 0.73);                         // white
            if (pick > 0.22) { paint = vec3<f32>(0.58, 0.16, 0.14); }         // red
            if (pick > 0.40) { paint = vec3<f32>(0.20, 0.28, 0.46); }         // blue
            if (pick > 0.55) { paint = vec3<f32>(0.62, 0.56, 0.42); }         // beige
            if (pick > 0.68) { paint = vec3<f32>(0.54, 0.56, 0.58); }         // silver
            if (pick > 0.80) { paint = vec3<f32>(0.20, 0.34, 0.24); }         // dark green
            if (pick > 0.90) { paint = vec3<f32>(0.16, 0.16, 0.18); }         // black
            if (part == 0u) { return VBox(vec3<f32>(0.0, 0.0, 0.05), vec2<f32>(0.50, 0.21), 0.17, paint * shade); }
            return VBox(vec3<f32>(-0.06, 0.0, 0.22), vec2<f32>(0.24, 0.18), 0.13, mix(paint, vec3<f32>(0.20, 0.24, 0.28), 0.65));
        }
    }
}

@vertex
fn vs_vehicle(@builtin(vertex_index) vi: u32, @location(0) pos: vec2<i32>, @location(1) dir: vec2<f32>, @location(2) len_m: f32, @location(3) kind: u32) -> MOut {
    let foot = vec2<f32>(pos - g3.center.xy);
    let face = (vi % 36u) / 6u;
    if (len_m <= 0.0 || face == 5u || height_raw(foot) <= 0.0) {
        return hidden();   // (a box has no bottom face)
    }
    let b = vehicle_box(kind & 3u, vi / 36u, i32(kind >> 2u));
    var u6 = array<f32, 6>(0.0, 1.0, 1.0, 0.0, 1.0, 0.0);
    var v6 = array<f32, 6>(0.0, 0.0, 1.0, 0.0, 1.0, 1.0);
    let k = vi % 6u;
    let uu = u6[k] * 2.0 - 1.0;
    var p: vec3<f32>;
    var normal: vec3<f32>;
    switch face {
        case 0u: { p = vec3<f32>(uu * b.half.x, -b.half.y, v6[k] * b.h); normal = vec3<f32>(0.0, -1.0, 0.0); }
        case 1u: { p = vec3<f32>(b.half.x, uu * b.half.y, v6[k] * b.h); normal = vec3<f32>(1.0, 0.0, 0.0); }
        case 2u: { p = vec3<f32>(-uu * b.half.x, b.half.y, v6[k] * b.h); normal = vec3<f32>(0.0, 1.0, 0.0); }
        case 3u: { p = vec3<f32>(-b.half.x, -uu * b.half.y, v6[k] * b.h); normal = vec3<f32>(-1.0, 0.0, 0.0); }
        default: { p = vec3<f32>(uu * b.half.x, (v6[k] * 2.0 - 1.0) * b.half.y, b.h); normal = vec3<f32>(0.0, 0.0, 1.0); }
    }
    p = (p + b.at) * len_m;
    let side = vec2<f32>(-dir.y, dir.x);
    let world = vec3<f32>(foot + dir * p.x + side * p.y, ground_z(foot) + p.z);
    var o: MOut;
    o.clip = g3.view_proj * vec4<f32>(world, 1.0);
    o.color = lit(b.color, vec3<f32>(dir * normal.x + side * normal.y, normal.z));
    o.pos = world;
    return o;
}

@fragment
fn fs_model(i: MOut) -> @location(0) vec4<f32> {
    return vec4<f32>(hazed(i.color, i.pos), 1.0);
}
