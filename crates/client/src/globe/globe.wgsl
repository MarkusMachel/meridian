// The globe (map rewrite M3). Two passes:
//   far:   one triangle over the whole screen; each pixel finds where its line of sight meets
//          the Earth and looks the hexagon up in a picture of the world.
//   tiles: the hexagons near the camera as triangles, drawn over the far pass.

struct Globals {
    row_x: vec4f,    // east / (tan * aspect): gives clip x from a camera-relative position
    row_y: vec4f,    // north / tan
    toward: vec4f,   // xyz: direction from the Earth's centre to the point looked at; w: R + height
    east: vec4f,     // xyz: east; w: tan * aspect
    north: vec4f,    // xyz: north; w: tan
    misc: vec4f,     // x: Earth radius; y: how much of the far land to show (0..1); z: hexagon grid strength; w: unused
    ids: vec4u,      // x: selected cell; y, z: world picture width, height; w: cell table width
    more: vec4f,     // x: road and railway strength; y: colouring (0 palette with terrain shaded in, 1 palette, 2 terrain)
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var world_picture: texture_2d<u32>;   // cell per longitude/latitude pixel
@group(0) @binding(2) var cell_table: texture_2d<u32>;      // per cell: r province, g country, b terrain, a built-up (0..255)
@group(0) @binding(3) var palette: texture_2d<f32>;         // colour per province

const NO_CELL: u32 = 0xffffffffu;
const SEA: vec3f = vec3f(0.42, 0.55, 0.66);
const SPACE: vec3f = vec3f(0.035, 0.04, 0.06);
const PI: f32 = 3.14159265;

fn cell_info(cell: u32) -> vec4u {
    let w = g.ids.w;
    return textureLoad(cell_table, vec2u(cell % w, cell / w), 0);
}

// Terrain classes of the map: 0 plains, 1 hills, 2 mountains, 3 high mountains, 4 desert, 5 ice.
fn terrain_color(t: u32) -> vec3f {
    switch t {
        case 1u { return vec3f(0.60, 0.62, 0.42); }
        case 2u { return vec3f(0.52, 0.47, 0.38); }
        case 3u { return vec3f(0.74, 0.72, 0.70); }
        case 4u { return vec3f(0.85, 0.78, 0.58); }
        case 5u { return vec3f(0.93, 0.95, 0.97); }
        default { return vec3f(0.62, 0.70, 0.48); }
    }
}

// The colour of a land cell before any lines: by the colouring in use.
fn land_color(info: vec4u) -> vec3f {
    if g.more.y > 1.5 {
        return terrain_color(info.b);
    }
    var c = province_color(info.r);
    if g.more.y < 0.5 {
        // the country's colour with the ground showing through a little
        c = mix(c, c * (terrain_color(info.b) / vec3f(0.62, 0.70, 0.48)), 0.45);
    }
    return c;
}

fn province_color(province: u32) -> vec3f {
    return textureLoad(palette, vec2u(province % 4096u, province / 4096u), 0).rgb;
}

// ---------------------------------------------------------------- far

struct FarOut {
    @builtin(position) pos: vec4f,
    @location(0) ndc: vec2f,
};

@vertex
fn vs_far(@builtin(vertex_index) i: u32) -> FarOut {
    let p = vec2f(f32(i32(i & 1u) * 4 - 1), f32(i32(i >> 1u) * 4 - 1));
    var o: FarOut;
    o.pos = vec4f(p, 0.0, 1.0);
    o.ndc = p;
    return o;
}

fn picture_cell(lon: f32, lat: f32, dx: i32, dy: i32) -> u32 {
    let size = vec2i(g.ids.yz);
    let x = i32(floor((lon + PI) / (2.0 * PI) * f32(size.x))) + dx;
    let y = clamp(i32(floor((0.5 * PI - lat) / PI * f32(size.y))) + dy, 0, size.y - 1);
    return textureLoad(world_picture, vec2i(((x % size.x) + size.x) % size.x, y), 0).r;
}

@fragment
fn fs_far(in: FarOut) -> @location(0) vec4f {
    let x = in.ndc.x * g.east.w;
    let y = in.ndc.y * g.north.w;
    let len2 = 1.0 + x * x + y * y;
    let s2 = (x * x + y * y) / len2;
    let far = g.toward.w;
    let r = g.misc.x;
    let under = r * r - far * far * s2;
    if under < 0.0 {
        // space, with a thin glow at the edge of the Earth
        let glow = exp(under / (r * r) * 60.0) * 0.5;
        return vec4f(SPACE + vec3f(0.25, 0.4, 0.6) * glow, 1.0);
    }
    let t = far / sqrt(len2) - sqrt(under);
    let ray = normalize(-g.toward.xyz + g.east.xyz * x + g.north.xyz * y);
    let d = normalize(g.toward.xyz * far + ray * t);
    let lat = asin(clamp(d.z, -1.0, 1.0));
    let lon = atan2(d.y, d.x);
    // the edge of the Earth is a little darker
    let facing = dot(d, -ray);
    let shade = 0.80 + 0.20 * smoothstep(0.0, 0.5, facing);
    var color = SEA;
    let cell = picture_cell(lon, lat, 0, 0);
    if cell != NO_CELL {
        let info = cell_info(cell);
        var land = land_color(info);
        // country borders and coasts: a neighbouring pixel of the picture belongs to someone else
        let right = picture_cell(lon, lat, 1, 0);
        let below = picture_cell(lon, lat, 0, 1);
        var edge = false;
        if right == NO_CELL || below == NO_CELL {
            edge = true;
        } else if cell_info(right).g != info.g || cell_info(below).g != info.g {
            edge = true;
        }
        if edge {
            land = land * 0.55;
        }
        if cell == g.ids.x {
            land = mix(land, vec3f(1.0), 0.4);
        }
        color = mix(SEA, land, g.misc.y);
    }
    return vec4f(color * shade, 1.0);
}

// ---------------------------------------------------------------- tiles

struct TileIn {
    @location(0) pos: vec3f,
    @location(1) cell: u32,
    @location(2) inward: f32,
    @location(3) side: u32,
    @location(4) offset: vec3f,   // per tile: its anchor relative to the camera
    @location(5) across: f32,
};

struct TileOut {
    @builtin(position) pos: vec4f,
    @location(0) @interpolate(flat) cell: u32,
    @location(1) inward: f32,
    @location(2) @interpolate(flat) side: u32,
    @location(3) across: f32,
};

@vertex
fn vs_tile(in: TileIn) -> TileOut {
    let v = in.pos + in.offset;
    let depth = -dot(v, g.toward.xyz);
    var o: TileOut;
    o.pos = vec4f(dot(v, g.row_x.xyz), dot(v, g.row_y.xyz), 0.5 * depth, depth);
    o.cell = in.cell;
    o.inward = in.inward;
    o.side = in.side;
    o.across = in.across;
    return o;
}

// How strongly a line of the given half-width (pixels) covers a pixel `px` pixels from the side.
fn line(px: f32, half_width: f32) -> f32 {
    return 1.0 - smoothstep(half_width - 0.5, half_width + 0.5, px);
}

@fragment
fn fs_tile(in: TileOut) -> @location(0) vec4f {
    let info = cell_info(in.cell);
    var color = land_color(info);
    // built-up ground is greyer
    color = mix(color, vec3f(0.55, 0.53, 0.52), f32(info.a) / 255.0 * 0.6 * g.more.x);
    if in.cell == g.ids.x {
        color = mix(color, vec3f(1.0), 0.4);
    }
    // pixels from the line that joins the hexagon's centre to the middle of this side
    let lane = abs(in.across) / max(fwidth(in.across), 1e-6);
    // pixels from the hexagon's side (each neighbour draws its own half of a line)
    let px = in.inward / max(fwidth(in.inward), 1e-6);
    color = mix(color, color * 0.80, line(px, 0.5) * g.misc.z);                                  // the hexagon grid
    if (in.side & 1u) != 0u { color = mix(color, color * 0.55, line(px, 0.6) * 0.8); }           // province border
    if (in.side & 2u) != 0u { color = mix(color, vec3f(0.12, 0.10, 0.10), line(px, 1.2)); }      // country border
    if (in.side & 8u) != 0u { color = mix(color, vec3f(0.20, 0.28, 0.36), line(px, 1.0)); }      // coast
    if (in.side & 4u) != 0u { color = mix(color, vec3f(0.22, 0.42, 0.68), line(px, 1.6)); }      // river
    if (in.side & 16u) != 0u { color = mix(color, vec3f(0.36, 0.24, 0.16), line(lane, 0.7) * g.more.x); }                       // road
    if (in.side & 32u) != 0u {                                                                                                  // railway: a dashed dark line
        let dash = step(0.5, fract(in.inward * 7.0));
        color = mix(color, vec3f(0.08, 0.08, 0.10), line(lane, 0.9) * dash * g.more.x);
    }
    if (in.side & 4u) != 0u && (in.side & 48u) != 0u {
        // a bridge: a pale deck with dark rails across the river
        let on_deck = line(lane, 2.2) * line(px, 4.0) * g.more.x;
        color = mix(color, vec3f(0.10, 0.09, 0.08), on_deck);
        color = mix(color, vec3f(0.93, 0.90, 0.82), line(lane, 1.2) * line(px, 4.0) * g.more.x);
    }
    return vec4f(color, 1.0);
}
