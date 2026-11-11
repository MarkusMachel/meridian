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
    more: vec4f,     // x: road and railway strength; y: colouring (0 palette with terrain shaded in, 1 palette, 2 terrain); z: 1 when the real lines are drawn; w: how strongly built-up ground shows
    eye_whole: vec4i,  // the camera's place in whole metres from the Earth's centre
    eye_part: vec4f,   // xyz: the rest of the camera's place (under a metre); w: metres per pixel
    screen: vec4f,     // xy: the picture's size in pixels
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
            edge = g.more.z < 0.5;   // the real coastline is drawn instead when there is one
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
    color = mix(color, vec3f(0.60, 0.57, 0.55), f32(info.a) / 255.0 * 0.7 * g.more.w);
    if in.cell == g.ids.x {
        color = mix(color, vec3f(1.0), 0.4);
    }
    // pixels from the line that joins the hexagon's centre to the middle of this side
    let lane = abs(in.across) / max(fwidth(in.across), 1e-6);
    // pixels from the hexagon's side (each neighbour draws its own half of a line)
    let px = in.inward / max(fwidth(in.inward), 1e-6);
    color = mix(color, color * 0.80, line(px, 0.5) * g.misc.z);                                  // the hexagon grid
    if (in.side & 1u) != 0u { color = mix(color, color * 0.55, line(px, 0.6) * 0.8); }           // province border
    // With the real lines drawn, the hexagon-side versions step back: borders and coasts go
    // faint, and the river sides (where a unit really has to cross) show only close up.
    let own = 1.0 - g.more.z;
    if (in.side & 2u) != 0u { color = mix(color, vec3f(0.12, 0.10, 0.10), line(px, 1.2) * max(own, 0.25)); }      // country border
    if (in.side & 8u) != 0u { color = mix(color, vec3f(0.20, 0.28, 0.36), line(px, 1.0) * own); }                 // coast
    if (in.side & 4u) != 0u { color = mix(color, vec3f(0.22, 0.42, 0.68), line(px, 1.6) * max(own, g.misc.z)); }  // river
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

// ---------------------------------------------------------------- real lines

// Coasts, borders, rivers, roads and railways as they run on the ground: each short piece is a
// thin rectangle a set number of pixels wide. The ends come as whole metres from the Earth's
// centre and the camera's place is taken off in whole numbers first, so nothing is lost to
// 32-bit floats however close the camera is.

struct LineIn {
    @builtin(vertex_index) corner: u32,
    @location(0) a: vec3i,
    @location(1) b: vec3i,
    @location(2) style: u32,
};

struct LineOut {
    @builtin(position) pos: vec4f,
    @location(0) @interpolate(flat) style: u32,
    @location(1) across: f32,          // pixels from the middle of the line
    @location(2) @interpolate(flat) half_width: f32,
};

fn line_half_width(style: u32) -> f32 {
    var w = 0.5;
    switch style {
        case 0u { w = 0.6; }    // coast
        case 1u { w = 0.9; }    // border
        case 2u { w = 1.1; }    // great river
        case 3u { w = 0.6; }    // river
        case 4u { w = 1.0; }    // major highway
        case 5u { w = 0.6; }    // highway or road
        case 6u { w = 0.4; }    // minor road
        default { w = 0.35; }   // railway
    }
    // a little bolder close up
    return w * clamp(sqrt(500.0 / g.eye_part.w), 1.0, 2.2);
}

fn line_color(style: u32) -> vec3f {
    switch style {
        case 0u { return vec3f(0.18, 0.26, 0.36); }
        case 1u { return vec3f(0.12, 0.10, 0.10); }
        case 2u { return vec3f(0.20, 0.40, 0.68); }
        case 3u { return vec3f(0.24, 0.44, 0.70); }
        case 4u { return vec3f(0.58, 0.18, 0.12); }
        case 5u { return vec3f(0.42, 0.27, 0.16); }
        case 6u { return vec3f(0.42, 0.33, 0.24); }
        default { return vec3f(0.16, 0.15, 0.17); }
    }
}

fn to_clip(v: vec3f) -> vec4f {
    let depth = -dot(v, g.toward.xyz);
    return vec4f(dot(v, g.row_x.xyz), dot(v, g.row_y.xyz), 0.5 * depth, depth);
}

@vertex
fn vs_line(in: LineIn) -> LineOut {
    var o: LineOut;
    o.style = in.style;
    o.half_width = line_half_width(in.style);
    // over the horizon: both ends must be on the near side
    let limit = g.misc.x / g.toward.w;
    let da = normalize(vec3f(in.a));
    let db = normalize(vec3f(in.b));
    if dot(da, g.toward.xyz) < limit || dot(db, g.toward.xyz) < limit {
        o.pos = vec4f(2.0, 2.0, 2.0, 1.0);
        o.across = 0.0;
        return o;
    }
    let ca = to_clip(vec3f(in.a - g.eye_whole.xyz) - g.eye_part.xyz);
    let cb = to_clip(vec3f(in.b - g.eye_whole.xyz) - g.eye_part.xyz);
    let half_screen = g.screen.xy * 0.5;
    let sa = ca.xy / ca.w * half_screen;
    let sb = cb.xy / cb.w * half_screen;
    let along = sb - sa;
    let len = max(length(along), 1e-4);
    let normal = vec2f(-along.y, along.x) / len;
    // corners: 0 1 2, 2 1 3 of the rectangle (end, side)
    let ends = array<f32, 6>(0.0, 1.0, 0.0, 0.0, 1.0, 1.0);
    let sides = array<f32, 6>(-1.0, -1.0, 1.0, 1.0, -1.0, 1.0);
    let reach = o.half_width + 1.0;   // one more pixel for the soft edge
    let clip = mix(ca, cb, ends[in.corner]);
    let shift = normal * sides[in.corner] * reach / half_screen;
    o.pos = vec4f(clip.xy + shift * clip.w, clip.z, clip.w);
    o.across = sides[in.corner] * reach;
    return o;
}

@fragment
fn fs_line(in: LineOut) -> @location(0) vec4f {
    let cover = 1.0 - smoothstep(in.half_width - 0.5, in.half_width + 0.5, abs(in.across));
    return vec4f(line_color(in.style), cover);
}
