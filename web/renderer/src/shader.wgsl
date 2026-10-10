// All positions are in device pixels with the origin at the top-left of the
// canvas. One instance per cell; the cell position comes from the instance
// index, so the instance buffer only carries what actually varies.

struct Globals {
    viewport: vec2<f32>,
    cell: vec2<f32>,
    cols: u32,
    underline_y: f32,
    strike_y: f32,
    line_thickness: f32,
    cursor_rect: vec4<f32>,
    cursor_color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var atlas: texture_2d<f32>;

const FLAG_UNDERLINE: u32 = 1u;
const FLAG_STRIKEOUT: u32 = 2u;
const FLAG_COLOR_GLYPH: u32 = 4u;

struct Instance {
    @location(0) atlas_pos: vec2<u32>,
    @location(1) glyph_size: vec2<u32>,
    @location(2) glyph_offset: vec2<i32>,
    @location(3) fg: vec4<f32>,
    @location(4) bg: vec4<f32>,
    @location(5) flags: u32,
}

fn to_clip(px: vec2<f32>) -> vec4<f32> {
    let ndc = px / g.viewport * 2.0 - 1.0;
    return vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);
}

// Triangle-strip corner: (0,0) (1,0) (0,1) (1,1).
fn corner(vertex_index: u32) -> vec2<f32> {
    return vec2<f32>(f32(vertex_index & 1u), f32((vertex_index >> 1u) & 1u));
}

fn cell_origin(instance_index: u32) -> vec2<f32> {
    return vec2<f32>(f32(instance_index % g.cols), f32(instance_index / g.cols)) * g.cell;
}

// ---- Pass 1: cell backgrounds, underline and strikeout ---------------------

struct BgOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) fg: vec4<f32>,
    @location(2) @interpolate(flat) bg: vec4<f32>,
    @location(3) @interpolate(flat) flags: u32,
}

@vertex
fn vs_bg(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
    inst: Instance,
) -> BgOut {
    let local = corner(vertex_index) * g.cell;
    var out: BgOut;
    out.position = to_clip(cell_origin(instance_index) + local);
    out.local = local;
    out.fg = inst.fg;
    out.bg = inst.bg;
    out.flags = inst.flags;
    return out;
}

@fragment
fn fs_bg(in: BgOut) -> @location(0) vec4<f32> {
    let y = in.local.y;
    let underline = (in.flags & FLAG_UNDERLINE) != 0u
        && y >= g.underline_y && y < g.underline_y + g.line_thickness;
    let strikeout = (in.flags & FLAG_STRIKEOUT) != 0u
        && y >= g.strike_y && y < g.strike_y + g.line_thickness;
    if underline || strikeout {
        return vec4<f32>(in.fg.rgb, 1.0);
    }
    return vec4<f32>(in.bg.rgb, 1.0);
}

// ---- Pass 2: glyphs, alpha blended over the backgrounds --------------------

struct GlyphOut {
    @builtin(position) position: vec4<f32>,
    @location(0) texel: vec2<f32>,
    @location(1) @interpolate(flat) fg: vec4<f32>,
    @location(2) @interpolate(flat) flags: u32,
}

@vertex
fn vs_glyph(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
    inst: Instance,
) -> GlyphOut {
    // Cells without a glyph have a zero size and collapse to nothing.
    let extent = corner(vertex_index) * vec2<f32>(inst.glyph_size);
    var out: GlyphOut;
    out.position = to_clip(cell_origin(instance_index) + vec2<f32>(inst.glyph_offset) + extent);
    out.texel = vec2<f32>(inst.atlas_pos) + extent;
    out.fg = inst.fg;
    out.flags = inst.flags;
    return out;
}

@fragment
fn fs_glyph(in: GlyphOut) -> @location(0) vec4<f32> {
    // Quads are pixel aligned, so every fragment maps to exactly one texel.
    let texel = textureLoad(atlas, vec2<i32>(in.texel), 0);
    if (in.flags & FLAG_COLOR_GLYPH) != 0u {
        return texel;
    }
    return vec4<f32>(in.fg.rgb, texel.a);
}

// ---- Pass 3: underline / beam cursor ---------------------------------------

@vertex
fn vs_cursor(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    return to_clip(g.cursor_rect.xy + corner(vertex_index) * g.cursor_rect.zw);
}

@fragment
fn fs_cursor() -> @location(0) vec4<f32> {
    return vec4<f32>(g.cursor_color.rgb, 1.0);
}
