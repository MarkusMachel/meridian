// The globe (map rewrite M3). Three passes:
//   far:   one triangle over the whole screen; each pixel finds where its line of sight meets
//          the Earth and looks the hexagon up in a picture of the world.
//   tiles: the hexagons near the camera as triangles, drawn over the far pass.
//   lines: coasts, borders, rivers, roads and railways as they run on the ground.
//
// Both ground passes colour a pixel with the same function, `ground`, from its longitude and
// latitude: a painted land colour, lit from its height, with woods, fields and towns drawn as
// soft textures where a regional pack knows them, and the country's colour washed over it,
// strongest along the borders. The look is a stylised landscape, not a survey map.

struct Globals {
    row_x: vec4f,    // east / (tan * aspect): gives clip x from a camera-relative position
    row_y: vec4f,    // north / tan
    toward: vec4f,   // xyz: direction from the Earth's centre to the point looked at; w: R + height
    east: vec4f,     // xyz: east; w: tan * aspect
    north: vec4f,    // xyz: north; w: tan
    misc: vec4f,     // x: Earth radius; y: unused; z: hexagon grid strength; w: unused
    ids: vec4u,      // x: selected cell; y, z: world picture width, height; w: cell table width
    more: vec4f,     // x: movement network strength; y: colouring (0 country wash, 1 palette, 2 terrain); z: 1 when the real lines are drawn; w: unused
    eye_whole: vec4i,  // the camera's place in whole metres from the Earth's centre
    eye_part: vec4f,   // xyz: the rest of the camera's place (under a metre); w: metres per pixel
    screen: vec4f,     // xy: the picture's size in pixels; z: 1 when there is a ground file (heights and painted colour)
    pack_a: vec4f,     // a regional pack's ground cover: west edge, north edge, pixel width, pixel height (degrees)
    pack_b: vec4u,     // its picture's width and height in pixels (0 = no pack), tile size, tiles across
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var world_picture: texture_2d<u32>;   // cell per longitude/latitude pixel
@group(0) @binding(2) var cell_table: texture_2d<u32>;      // per cell: r province, g country, b terrain, a built-up (low 8 bits) and hexagons to the nearest border (next 8)
@group(0) @binding(3) var palette: texture_2d<f32>;         // colour per province
@group(0) @binding(4) var heights: texture_2d<u32>;         // metres above the sea, the whole world
@group(0) @binding(5) var cover_tiles: texture_2d_array<u32>;  // a pack's ground cover, tile by tile
@group(0) @binding(6) var cover_table: texture_2d<u32>;     // which stored tile each tile of the picture is
@group(0) @binding(7) var land_paint: texture_2d<f32>;      // the painted colour of the world's land
@group(0) @binding(8) var smooth_sampler: sampler;

const NO_CELL: u32 = 0xffffffffu;
const NO_COVER: u32 = 255u;
const SPACE: vec3f = vec3f(0.035, 0.04, 0.06);
const DEEP_SEA: vec3f = vec3f(0.30, 0.44, 0.58);
const SHALLOW_SEA: vec3f = vec3f(0.47, 0.63, 0.74);
const PI: f32 = 3.14159265;

fn cell_info(cell: u32) -> vec4u {
    let w = g.ids.w;
    return textureLoad(cell_table, vec2u(cell % w, cell / w), 0);
}

fn province_color(province: u32) -> vec3f {
    return textureLoad(palette, vec2u(province % 4096u, province / 4096u), 0).rgb;
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

// ---------------------------------------------------------------- noise

// Repeatable "random" numbers from a position: the same place always gets the same value, so
// the textures made from them stay put on the ground.
fn hash21(p: vec2f) -> f32 {
    var q = fract(vec3f(p.x, p.y, p.x) * 0.1031);
    q = q + dot(q, q.yzx + 33.33);
    return fract((q.x + q.y) * q.z);
}

fn hash22(p: vec2f) -> vec2f {
    var q = fract(vec3f(p.x, p.y, p.x) * vec3f(0.1031, 0.1030, 0.0973));
    q = q + dot(q, q.yzx + 33.33);
    return fract((q.xx + q.yz) * q.zy);
}

// Smooth noise, 0..1.
fn noise(p: vec2f) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash21(i), hash21(i + vec2f(1.0, 0.0)), u.x), mix(hash21(i + vec2f(0.0, 1.0)), hash21(i + vec2f(1.0, 1.0)), u.x), u.y);
}

// Noise with finer noise laid over it three times: cloud-like, 0..1.
fn clouds(p: vec2f) -> f32 {
    return 0.5 * noise(p) + 0.25 * noise(p * 2.03 + 7.1) + 0.125 * noise(p * 4.01 + 3.7) + 0.125 * noise(p * 8.07 + 1.3);
}

// A pattern of irregular patches (fields, roofs): x is how far the point is from the edge of
// its patch (0 on the edge, up to about 0.5 inside), y a random number for the patch.
fn patches(p: vec2f) -> vec2f {
    let i = floor(p);
    let f = fract(p);
    var nearest = 9.0;
    var second = 9.0;
    var id = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let c = vec2f(f32(x), f32(y));
            let d = length(c + hash22(i + c) - f);
            if d < nearest {
                second = nearest;
                nearest = d;
                id = hash21(i + c + 17.0);
            } else if d < second {
                second = d;
            }
        }
    }
    return vec2f((second - nearest) * 0.5, id);
}

// ---------------------------------------------------------------- the ground

// Height in metres at a longitude and latitude (radians), smooth between the picture's pixels.
fn height_at(lon: f32, lat: f32) -> f32 {
    let size = vec2i(textureDimensions(heights));
    let p = vec2f((lon + PI) / (2.0 * PI) * f32(size.x) - 0.5, (0.5 * PI - lat) / PI * f32(size.y) - 0.5);
    let i = vec2i(floor(p));
    let f0 = fract(p);
    let f = f0 * f0 * (3.0 - 2.0 * f0);
    let x0 = ((i.x % size.x) + size.x) % size.x;
    let x1 = (x0 + 1) % size.x;
    let y0 = clamp(i.y, 0, size.y - 1);
    let y1 = clamp(i.y + 1, 0, size.y - 1);
    let top = mix(f32(textureLoad(heights, vec2i(x0, y0), 0).r), f32(textureLoad(heights, vec2i(x1, y0), 0).r), f.x);
    let bottom = mix(f32(textureLoad(heights, vec2i(x0, y1), 0).r), f32(textureLoad(heights, vec2i(x1, y1), 0).r), f.x);
    return mix(top, bottom, f.y);
}

// One pixel of a regional pack's ground cover: 0 sea, 1 inland water, 2 woods, 3 built-up,
// 4 industry, 6 other land; NO_COVER outside the pack's picture or its coverage.
fn cover_pixel(x: i32, y: i32) -> u32 {
    if x < 0 || y < 0 || x >= i32(g.pack_b.x) || y >= i32(g.pack_b.y) {
        return NO_COVER;
    }
    let px = vec2u(u32(x), u32(y));
    let layer = textureLoad(cover_table, px / g.pack_b.z, 0).r;
    if layer == NO_CELL {
        return 0u;
    }
    let kind = textureLoad(cover_tiles, px % g.pack_b.z, layer, 0).r;
    if kind == 7u {
        return NO_COVER;   // land the pack has no detail for (a neighbouring country)
    }
    return kind;
}

// What a regional pack has around a point, blended from the four pixels about it so that
// edges come out as smooth curves instead of squares. Each field is a share from 0 to 1.
struct Cover {
    known: f32,     // how much of the surroundings the pack covers at all
    sea: f32,
    water: f32,
    woods: f32,
    built: f32,
    industry: f32,
};

fn cover_at(lon: f32, lat: f32) -> Cover {
    var c: Cover;
    if g.pack_b.x == 0u {
        return c;
    }
    let p = vec2f((degrees(lon) - g.pack_a.x) / g.pack_a.z - 0.5, (g.pack_a.y - degrees(lat)) / g.pack_a.w - 0.5);
    if p.x < -1.0 || p.y < -1.0 || p.x > f32(g.pack_b.x) || p.y > f32(g.pack_b.y) {
        return c;
    }
    let i = vec2i(floor(p));
    let f = fract(p);
    for (var k = 0; k < 4; k++) {
        let dx = k & 1;
        let dy = k >> 1;
        let w = mix(1.0 - f.x, f.x, f32(dx)) * mix(1.0 - f.y, f.y, f32(dy));
        switch cover_pixel(i.x + dx, i.y + dy) {
            case 255u {}
            case 0u { c.known += w; c.sea += w; }
            case 1u { c.known += w; c.water += w; }
            case 2u { c.known += w; c.woods += w; }
            case 3u { c.known += w; c.built += w; }
            case 4u { c.known += w; c.industry += w; }
            default { c.known += w; }
        }
    }
    return c;
}

// The finished colour of a pixel of the Earth.
//   political: the country's (or map mode's) colour; info: the hexagon's table entry;
//   from_border: hexagons to the nearest country border;
//   hex_land: 1 where a land hexagon is under the pixel, 0 where none is.
fn ground(political: vec3f, info: vec4u, from_border: f32, hex_land: f32, lon: f32, lat: f32) -> vec3f {
    let m_per_px = g.eye_part.w;
    let radius = g.misc.x;
    // a flat sheet of metres around the point, for the patterns
    let q = vec2f(lon * cos(lat), lat) * radius;
    let cover = cover_at(lon, lat);
    let detailed = step(0.5, cover.known);                              // a pack knows this place
    let close = 1.0 - smoothstep(120.0, 500.0, m_per_px);              // near enough for patterns

    // ---- land or water ----
    var land = hex_land;
    var inland_water = 0.0;
    if detailed > 0.5 {
        land = 1.0 - smoothstep(0.42, 0.58, cover.sea / cover.known);
        inland_water = smoothstep(0.42, 0.58, cover.water / cover.known);
    }

    // ---- the shape of the land: slope from the heights, plus fine made-up roughness ----
    let has_ground = g.screen.z;
    let texel = PI / f32(textureDimensions(heights).y);
    let h = height_at(lon, lat);
    let east_m = 2.0 * texel * radius * max(cos(lat), 0.05);
    var slope = vec2f(
        (height_at(lon + texel, lat) - height_at(lon - texel, lat)) / east_m,
        (height_at(lon, lat + texel) - height_at(lon, lat - texel)) / (2.0 * texel * radius),
    ) * has_ground;
    let steep = length(slope);
    // roughness: finer than the height picture can show, stronger where the land is rugged
    let rough = clamp(steep * 9.0, 0.12, 1.0) * (1.0 - smoothstep(300.0, 1500.0, m_per_px));
    let d = max(m_per_px, 30.0);
    let bumps = vec2f(
        clouds((q + vec2f(d, 0.0)) / 2600.0) - clouds((q - vec2f(d, 0.0)) / 2600.0),
        clouds((q + vec2f(0.0, d)) / 2600.0) - clouds((q - vec2f(0.0, d)) / 2600.0),
    ) * (260.0 / (2.0 * d)) * rough;
    slope = slope + bumps;
    // light from the north-west; heights are exaggerated more from far away, where a mountain
    // range is only a few pixels wide
    let tall = clamp(m_per_px / 120.0, 4.0, 22.0);
    let normal = normalize(vec3f(-slope * tall, 1.0));
    let sun = normalize(vec3f(-0.55, 0.60, 0.58));
    let lit = clamp(0.30 + 0.70 * dot(normal, sun) / sun.z, 0.35, 1.45);

    // ---- what covers the land ----
    var c = terrain_color(info.b);
    if has_ground > 0.5 {
        let uv = vec2f((lon + PI) / (2.0 * PI), (0.5 * PI - lat) / PI);
        c = textureSampleLevel(land_paint, smooth_sampler, uv, 0.0).rgb;
        c = mix(c, vec3f(dot(c, vec3f(0.3, 0.6, 0.1))), -0.15) * 1.04;      // a touch more colour
    }
    // bare rock on steep slopes, snow on the heights
    c = mix(c, vec3f(0.56, 0.53, 0.49), smoothstep(0.22, 0.50, steep) * 0.55);
    c = mix(c, vec3f(0.95, 0.96, 0.98), smoothstep(2500.0, 3100.0, h + 400.0 * clouds(q / 5000.0)) * has_ground);
    if detailed > 0.5 {
        // fields: a patchwork on open ground
        let field = patches(q / 420.0);
        let open = 1.0 - smoothstep(0.08, 0.25, steep);
        let crop = mix(vec3f(0.93, 1.02, 0.86), vec3f(1.07, 1.03, 0.82), field.y);
        c = mix(c, c * crop * (0.94 + 0.10 * smoothstep(0.0, 0.05, field.x)), 0.7 * open * close);
        // woods: dark green, mottled like treetops, a little darker at the edge
        let woods = smoothstep(0.35, 0.60, cover.woods / cover.known);
        let canopy = 0.80 + 0.35 * noise(q / 38.0) * close + 0.15 * noise(q / 310.0);
        c = mix(c, vec3f(0.23, 0.37, 0.20) * canopy, woods * 0.92);
        // towns: blocks of roofs with streets between them
        let town = smoothstep(0.35, 0.60, cover.built / cover.known);
        let works = smoothstep(0.35, 0.60, cover.industry / cover.known);
        let block = patches(q / 170.0);
        let roofs = mix(vec3f(0.78, 0.72, 0.66), vec3f(0.90, 0.86, 0.79), block.y);
        let streets = smoothstep(0.03, 0.09, block.x);
        let near = 1.0 - smoothstep(25.0, 70.0, m_per_px);             // the blocks show only really close
        let town_color = mix(vec3f(0.80, 0.74, 0.68), mix(vec3f(0.52, 0.49, 0.47), roofs, streets), near);
        c = mix(c, town_color, town * 0.95);
        let shed = patches(q / 320.0);
        let works_color = mix(vec3f(0.66, 0.65, 0.68), mix(vec3f(0.50, 0.49, 0.50), mix(vec3f(0.66, 0.66, 0.70), vec3f(0.76, 0.75, 0.76), shed.y), smoothstep(0.03, 0.08, shed.x)), near);
        c = mix(c, works_color, works * 0.95);
    } else {
        // no pack here: built-up ground by the hexagon, as a soft grey
        c = mix(c, vec3f(0.74, 0.70, 0.66), f32(info.a & 255u) / 255.0 * 0.6 * (1.0 - smoothstep(1200.0, 2500.0, m_per_px)));
    }
    c = c * lit;

    // ---- the country's colour: a wash, strongest along the borders ----
    if g.more.y > 1.5 {
        // terrain mode: the ground alone
    } else if g.more.y > 0.5 {
        c = mix(c, political, 0.85);
    } else {
        let far = smoothstep(700.0, 2800.0, m_per_px);                  // from far away: a political map
        let edge = 0.15 + 0.40 * exp(-from_border * 0.6);
        c = mix(c, political * (0.80 + 0.20 * lit), mix(edge, 0.78, far));
    }

    // ---- water ----
    let ripple = 0.97 + 0.06 * clouds(q / 900.0) * close;
    var water = DEEP_SEA;
    if detailed > 0.5 {
        // lighter toward the shore, with a pale line where it meets the land
        let shore = smoothstep(0.15, 0.58, 1.0 - cover.sea / cover.known);
        water = mix(DEEP_SEA, SHALLOW_SEA, shore);
    }
    c = mix(c, SHALLOW_SEA * ripple, inland_water);
    return mix(water * ripple, c, land);
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
    let shade = 0.80 + 0.20 * smoothstep(0.0, 0.5, dot(d, -ray));
    let cell = picture_cell(lon, lat, 0, 0);
    var info = vec4u(0u, 0u, 0u, 15u << 8u);
    var political = vec3f(0.80, 0.78, 0.70);       // land that belongs to no hexagon: plain
    var hex_land = 0.0;
    if cell != NO_CELL {
        info = cell_info(cell);
        political = province_color(info.r);
        hex_land = 1.0;
        if g.more.z < 0.5 {
            // no real lines: mark coasts and country borders from the picture itself
            let right = picture_cell(lon, lat, 1, 0);
            let below = picture_cell(lon, lat, 0, 1);
            var edge = right == NO_CELL || below == NO_CELL;
            if !edge {
                edge = cell_info(right).g != info.g || cell_info(below).g != info.g;
            }
            if edge {
                political = political * 0.55;
            }
        }
        if cell == g.ids.x {
            political = mix(political, vec3f(1.0), 0.4);
        }
    }
    return vec4f(ground(political, info, f32((info.a >> 8u) & 255u), hex_land, lon, lat) * shade, 1.0);
}

// ---------------------------------------------------------------- tiles

struct TileIn {
    @location(0) pos: vec3f,
    @location(1) cell: u32,
    @location(2) inward: f32,
    @location(3) side: u32,
    @location(4) offset: vec3f,   // per tile: its anchor relative to the camera
    @location(5) across: f32,
    @location(6) from_border: f32,
};

struct TileOut {
    @builtin(position) pos: vec4f,
    @location(0) @interpolate(flat) cell: u32,
    @location(1) inward: f32,
    @location(2) @interpolate(flat) side: u32,
    @location(3) across: f32,
    @location(4) from_centre: vec3f,   // the point's place from the Earth's centre (rough: good to a metre or two)
    @location(5) from_border: f32,
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
    o.from_border = in.from_border;
    o.from_centre = v + vec3f(g.eye_whole.xyz) + g.eye_part.xyz;
    return o;
}

// How strongly a line of the given half-width (pixels) covers a pixel `px` pixels from the side.
fn line(px: f32, half_width: f32) -> f32 {
    return 1.0 - smoothstep(half_width - 0.5, half_width + 0.5, px);
}

@fragment
fn fs_tile(in: TileOut) -> @location(0) vec4f {
    let info = cell_info(in.cell);
    let d = normalize(in.from_centre);
    let lon = atan2(d.y, d.x);
    let lat = asin(clamp(d.z, -1.0, 1.0));
    var political = province_color(info.r);
    if in.cell == g.ids.x {
        political = mix(political, vec3f(1.0), 0.4);
    }
    var color = ground(political, info, in.from_border, 1.0, lon, lat);
    // The lines on hexagon sides. Where a pack says the pixel is really sea, there are none.
    let cover = cover_at(lon, lat);
    var on_land = 1.0;
    if cover.known > 0.5 {
        on_land = 1.0 - smoothstep(0.42, 0.58, (cover.sea + cover.water) / cover.known);
    }
    if in.cell == g.ids.x {
        color = mix(color, vec3f(1.0), 0.25 * on_land);       // the selected hexagon stands out whatever the zoom
    }
    // pixels from the line that joins the hexagon's centre to the middle of this side
    let lane = abs(in.across) / max(fwidth(in.across), 1e-6);
    // pixels from the hexagon's side (each neighbour draws its own half of a line)
    let px = in.inward / max(fwidth(in.inward), 1e-6);
    var marked = color;
    marked = mix(marked, marked * 0.80, line(px, 0.5) * g.misc.z);                                 // the hexagon grid
    let province_lines = 0.35 * (1.0 - smoothstep(250.0, 600.0, g.eye_part.w));
    if (in.side & 1u) != 0u { marked = mix(marked, marked * 0.62, line(px, 0.6) * province_lines); }  // province border, close up only
    // With the real lines drawn, the hexagon-side versions step back: borders and coasts go
    // faint, and the river sides (where a unit really has to cross) show only with the
    // movement network switched on.
    let own = 1.0 - g.more.z;
    if (in.side & 2u) != 0u { marked = mix(marked, vec3f(0.12, 0.10, 0.10), line(px, 1.2) * max(own, 0.15)); }     // country border
    if (in.side & 8u) != 0u { marked = mix(marked, vec3f(0.20, 0.28, 0.36), line(px, 1.0) * own); }                // coast
    if (in.side & 4u) != 0u { marked = mix(marked, vec3f(0.22, 0.42, 0.68), line(px, 1.6) * max(own, g.more.x)); } // river
    // The supply network: each link drawn from the hexagon's centre to the side it crosses,
    // wider and stronger in colour the higher its level (its capacity class).
    let road_level = f32((in.side >> 8u) & 7u);
    let rail_level = f32((in.side >> 12u) & 7u);
    if (in.side & 16u) != 0u {
        let road_color = mix(vec3f(0.62, 0.52, 0.40), vec3f(0.86, 0.30, 0.10), road_level / 5.0);
        marked = mix(marked, vec3f(0.15, 0.10, 0.06), line(lane, 1.3 + 0.6 * road_level) * g.more.x);
        marked = mix(marked, road_color, line(lane, 0.5 + 0.6 * road_level) * g.more.x);
    }
    if (in.side & 32u) != 0u {
        // a railway: dark, with pale ties across it
        let ties = step(0.5, fract(in.inward * 9.0));
        marked = mix(marked, vec3f(0.06, 0.06, 0.08), line(lane, 0.6 + 0.45 * rail_level) * g.more.x);
        marked = mix(marked, vec3f(0.92, 0.92, 0.90), line(lane, 0.1 + 0.30 * rail_level) * ties * g.more.x * step(1.5, rail_level));
    }
    if (in.side & 4u) != 0u && (in.side & 48u) != 0u {
        // a bridge: a pale deck with dark rails across the river
        let on_deck = line(lane, 2.2) * line(px, 4.0) * g.more.x;
        marked = mix(marked, vec3f(0.10, 0.09, 0.08), on_deck);
        marked = mix(marked, vec3f(0.93, 0.90, 0.82), line(lane, 1.2) * line(px, 4.0) * g.more.x);
    }
    return vec4f(mix(color, marked, on_land), 1.0);
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

// Styles: 0 coast, 1 border, 2 great river, 3 river, 4 motorway, 5 main road, 6 minor road,
// 7 railway, 8 local road.
fn line_half_width(style: u32) -> f32 {
    var w = 0.5;
    switch style {
        case 0u { w = 0.6; }
        case 1u { w = 1.0; }
        case 2u { w = 1.2; }
        case 3u { w = 0.8; }
        case 4u { w = 1.5; }
        case 5u { w = 1.1; }
        case 6u { w = 0.5; }
        case 7u { w = 0.7; }
        default { w = 0.6; }
    }
    // bolder close up, so a motorway is a band and not a hair; finer from far away
    return w * clamp(sqrt(260.0 / g.eye_part.w), 0.6, 2.6);
}

// The colour of a line's middle and of its rim. Roads have a dark rim like on a road map.
fn line_fill(style: u32) -> vec3f {
    switch style {
        case 0u { return vec3f(0.20, 0.30, 0.40); }
        case 1u { return vec3f(0.13, 0.10, 0.10); }
        case 2u { return vec3f(0.36, 0.56, 0.76); }
        case 3u { return vec3f(0.38, 0.58, 0.78); }
        case 4u { return vec3f(0.95, 0.66, 0.28); }
        case 5u { return vec3f(0.97, 0.88, 0.56); }
        case 6u { return vec3f(0.90, 0.86, 0.76); }
        case 7u { return vec3f(0.20, 0.19, 0.21); }
        default { return vec3f(0.92, 0.89, 0.82); }
    }
}

fn line_rim(style: u32) -> vec3f {
    switch style {
        case 2u, 3u { return vec3f(0.24, 0.40, 0.60); }
        case 4u { return vec3f(0.45, 0.22, 0.10); }
        case 5u, 6u, 8u { return vec3f(0.42, 0.34, 0.24); }
        default { return line_fill(style); }
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
    // style 255: a coarse world line that a regional pack replaces
    if in.style == 255u || dot(da, g.toward.xyz) < limit || dot(db, g.toward.xyz) < limit {
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
    // corners: 0 1 2, 2 1 3 of the rectangle (end, side); the ends reach half a width past
    // the piece so that the joints between pieces are covered
    let ends = array<f32, 6>(0.0, 1.0, 0.0, 0.0, 1.0, 1.0);
    let sides = array<f32, 6>(-1.0, -1.0, 1.0, 1.0, -1.0, 1.0);
    let reach = o.half_width + 1.0;   // one more pixel for the soft edge
    let end = ends[in.corner];
    let clip = mix(ca, cb, end);
    let shift = (normal * sides[in.corner] * reach + along / len * (end * 2.0 - 1.0) * o.half_width * 0.5) / half_screen;
    o.pos = vec4f(clip.xy + shift * clip.w, clip.z, clip.w);
    o.across = sides[in.corner] * reach;
    return o;
}

@fragment
fn fs_line(in: LineOut) -> @location(0) vec4f {
    let from_middle = abs(in.across);
    let cover = 1.0 - smoothstep(in.half_width - 0.5, in.half_width + 0.5, from_middle);
    // the rim takes the outer pixel of a line wide enough to have one
    let rim = smoothstep(in.half_width - 1.3, in.half_width - 0.5, from_middle) * step(1.4, in.half_width);
    return vec4f(mix(line_fill(in.style), line_rim(in.style), rim), cover);
}
