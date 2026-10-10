//! Glyph atlas: glyphs are rasterised on demand with a Canvas2D context, so
//! any font the browser can resolve (including CJK fallbacks) works without
//! shipping font data, and uploaded into one RGBA texture.

use std::collections::HashMap;

use wasm_bindgen::{JsCast, JsValue};
use web_sys::{OffscreenCanvas, OffscreenCanvasRenderingContext2d};

pub(crate) const STYLE_BOLD: u8 = 1;
pub(crate) const STYLE_ITALIC: u8 = 2;
const STYLE_COUNT: usize = 4;

const INITIAL_SIZE: u32 = 1024;
const MAX_SIZE: u32 = 4096;

/// Cell geometry in device pixels. Everything is a whole number of pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Metrics {
    pub cell_w: u32,
    pub cell_h: u32,
    /// Distance from the top of the cell to the text baseline.
    pub baseline: u32,
    /// Top of the underline band, relative to the top of the cell.
    pub underline_y: u32,
    /// Top of the strikeout band, relative to the top of the cell.
    pub strike_y: u32,
    pub line_thickness: u32,
    /// Thickness of the underline and beam cursors.
    pub cursor_thickness: u32,
}

/// Where a glyph lives in the atlas and how to place it in its cell.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Glyph {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    /// Offset of the bitmap from the top-left corner of the cell.
    pub offset_x: i16,
    pub offset_y: i16,
    /// The bitmap carries its own colours (emoji) instead of coverage only.
    pub color: bool,
}

/// The atlas has no room left for another glyph.
pub(crate) struct AtlasFull;

pub(crate) struct GlyphAtlas {
    canvas: OffscreenCanvas,
    ctx: OffscreenCanvasRenderingContext2d,
    canvas_w: u32,
    canvas_h: u32,
    pad_x: u32,
    pad_y: u32,
    fonts: [String; STYLE_COUNT],
    /// Style currently set on `ctx`, to avoid re-parsing the font string.
    active_style: Option<u8>,
    metrics: Metrics,

    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: u32,
    max_size: u32,
    shelf_x: u32,
    shelf_y: u32,
    shelf_h: u32,

    /// Narrow ASCII glyphs, indexed by `style * 128 + ch`.
    ascii: Vec<Option<Glyph>>,
    others: HashMap<u32, Glyph>,
    upload: Vec<u8>,
}

fn js_err(context: &str, err: JsValue) -> String {
    format!("{context}: {err:?}")
}

fn create_texture(device: &wgpu::Device, size: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("webterm glyph atlas"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

impl GlyphAtlas {
    /// `font_px` is the font size in device pixels.
    pub fn new(device: &wgpu::Device, font_family: &str, font_px: f32) -> Result<Self, String> {
        let canvas =
            OffscreenCanvas::new(1, 1).map_err(|e| js_err("OffscreenCanvas unavailable", e))?;
        let options = js_sys::Object::new();
        // Glyphs are read back with getImageData; keep the canvas on the CPU.
        js_sys::Reflect::set(&options, &"willReadFrequently".into(), &JsValue::TRUE)
            .map_err(|e| js_err("context options", e))?;
        let ctx = canvas
            .get_context_with_context_options("2d", &options)
            .map_err(|e| js_err("2d context", e))?
            .ok_or("2d context unavailable")?
            .dyn_into::<OffscreenCanvasRenderingContext2d>()
            .map_err(|_| "unexpected 2d context type".to_string())?;

        let max_size = device.limits().max_texture_dimension_2d.min(MAX_SIZE);
        let size = INITIAL_SIZE.min(max_size);
        let (texture, view) = create_texture(device, size);

        let mut atlas = Self {
            canvas,
            ctx,
            canvas_w: 1,
            canvas_h: 1,
            pad_x: 0,
            pad_y: 0,
            fonts: Default::default(),
            active_style: None,
            metrics: Metrics::default(),
            texture,
            view,
            size,
            max_size,
            shelf_x: 0,
            shelf_y: 0,
            shelf_h: 0,
            ascii: vec![None; STYLE_COUNT * 128],
            others: HashMap::new(),
            upload: Vec::new(),
        };
        atlas.set_font(font_family, font_px)?;
        Ok(atlas)
    }

    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Switches font, re-measures the cell and forgets every cached glyph.
    pub fn set_font(&mut self, font_family: &str, font_px: f32) -> Result<(), String> {
        let font_px = if font_px.is_finite() { font_px.max(1.0) } else { 1.0 };
        for (style, font) in self.fonts.iter_mut().enumerate() {
            let style = style as u8;
            let italic = if style & STYLE_ITALIC != 0 { "italic " } else { "" };
            let bold = if style & STYLE_BOLD != 0 { "bold " } else { "" };
            *font = format!("{italic}{bold}{font_px}px {font_family}");
        }

        self.ctx.set_font(&self.fonts[0]);
        let measured = self
            .ctx
            .measure_text("M")
            .map_err(|e| js_err("measureText", e))?;
        let advance = measured.width();
        let mut ascent = measured.font_bounding_box_ascent();
        let mut descent = measured.font_bounding_box_descent();
        if !(ascent.is_finite() && descent.is_finite() && ascent + descent > 0.0) {
            ascent = font_px as f64 * 0.9;
            descent = font_px as f64 * 0.25;
        }

        let cell_w = (advance.round() as u32).max(1);
        let cell_h = ((ascent + descent).round() as u32).max(1);
        // Share the rounding error of the line height between top and bottom.
        let slack = cell_h as f64 - (ascent + descent);
        let baseline = ((ascent + slack / 2.0).round() as u32).clamp(1, cell_h);
        let line_thickness = ((font_px / 14.0).round() as u32).max(1);
        let underline_y =
            (baseline + line_thickness).min(cell_h.saturating_sub(line_thickness));
        let strike_y = baseline
            .saturating_sub((font_px * 0.28).round() as u32)
            .saturating_sub(line_thickness / 2);
        self.metrics = Metrics {
            cell_w,
            cell_h,
            baseline,
            underline_y,
            strike_y,
            line_thickness,
            cursor_thickness: ((font_px / 8.0).round() as u32).clamp(1, cell_w),
        };

        // Room for a double-width glyph plus overhang on every side (italics,
        // accents, fallback fonts with taller boxes).
        self.pad_x = cell_w;
        self.pad_y = cell_h.div_ceil(2);
        self.canvas_w = cell_w * 2 + self.pad_x * 2;
        self.canvas_h = cell_h + self.pad_y * 2;
        self.canvas.set_width(self.canvas_w);
        self.canvas.set_height(self.canvas_h);
        // Resizing a canvas resets its context state.
        self.ctx.set_fill_style_str("#ffffff");
        self.ctx.set_text_baseline("alphabetic");
        self.active_style = None;

        self.clear();
        Ok(())
    }

    fn clear(&mut self) {
        self.ascii.fill(None);
        self.others.clear();
        self.shelf_x = 0;
        self.shelf_y = 0;
        self.shelf_h = 0;
    }

    /// Makes room after [`AtlasFull`]: doubles the texture while the device
    /// allows it, otherwise starts over in place. Either way every glyph
    /// handed out so far is invalid. Returns whether the texture (and so the
    /// view) was replaced.
    pub fn grow_or_reset(&mut self, device: &wgpu::Device) -> bool {
        self.clear();
        if self.size >= self.max_size {
            return false;
        }
        self.size = (self.size * 2).min(self.max_size);
        (self.texture, self.view) = create_texture(device, self.size);
        true
    }

    pub fn glyph(
        &mut self,
        queue: &wgpu::Queue,
        ch: char,
        style: u8,
        wide: bool,
    ) -> Result<Glyph, AtlasFull> {
        let style = style & (STYLE_COUNT as u8 - 1);
        if !wide && ch.is_ascii() {
            let slot = style as usize * 128 + ch as usize;
            if let Some(glyph) = self.ascii[slot] {
                return Ok(glyph);
            }
            let glyph = self.rasterize(queue, ch, style, wide)?;
            self.ascii[slot] = Some(glyph);
            return Ok(glyph);
        }

        // A `char` needs 21 bits.
        let key = ch as u32 | (style as u32) << 21 | (wide as u32) << 23;
        if let Some(glyph) = self.others.get(&key) {
            return Ok(*glyph);
        }
        let glyph = self.rasterize(queue, ch, style, wide)?;
        self.others.insert(key, glyph);
        Ok(glyph)
    }

    fn rasterize(
        &mut self,
        queue: &wgpu::Queue,
        ch: char,
        style: u8,
        wide: bool,
    ) -> Result<Glyph, AtlasFull> {
        if let Some(block) = block_element(ch) {
            return self.rasterize_block(queue, block, wide);
        }
        if self.active_style != Some(style) {
            self.ctx.set_font(&self.fonts[style as usize]);
            self.active_style = Some(style);
        }
        let mut utf8 = [0u8; 4];
        let text: &str = ch.encode_utf8(&mut utf8);

        let (canvas_w, canvas_h) = (self.canvas_w as usize, self.canvas_h as usize);
        let span = (self.metrics.cell_w * if wide { 2 } else { 1 }) as f64;
        let advance = self
            .ctx
            .measure_text(text)
            .map(|m| m.width())
            .unwrap_or(span);
        let x = self.pad_x as f64;
        let y = (self.pad_y + self.metrics.baseline) as f64;
        self.ctx
            .clear_rect(0.0, 0.0, canvas_w as f64, canvas_h as f64);
        // Fallback fonts rarely match the cell grid: squeeze glyphs that are
        // too wide and centre the ones that are too narrow. The primary
        // font's own glyphs land on whole pixels untouched.
        let drawn = if advance > span + 0.5 {
            self.ctx.fill_text_with_max_width(text, x, y, span)
        } else {
            self.ctx
                .fill_text(text, x + ((span - advance) / 2.0).round(), y)
        };
        let Some(image) = drawn.ok().and_then(|()| {
            self.ctx
                .get_image_data(0.0, 0.0, canvas_w as f64, canvas_h as f64)
                .ok()
        }) else {
            return Ok(Glyph::default());
        };
        let pixels = image.data().0;
        if pixels.len() != canvas_w * canvas_h * 4 {
            return Ok(Glyph::default());
        }

        // Tight bounding box of the inked pixels.
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (canvas_w, canvas_h, 0, 0);
        let mut color = false;
        for (row, line) in pixels.chunks_exact(canvas_w * 4).enumerate() {
            for (col, px) in line.chunks_exact(4).enumerate() {
                if px[3] == 0 {
                    continue;
                }
                min_x = min_x.min(col);
                max_x = max_x.max(col);
                min_y = min_y.min(row);
                max_y = max_y.max(row);
                // Text is drawn in white, so anything else is a colour glyph.
                color |= px[3] >= 128 && px[0].min(px[1]).min(px[2]) < 200;
            }
        }
        if max_x < min_x {
            return Ok(Glyph::default());
        }
        let (w, h) = ((max_x - min_x + 1) as u32, (max_y - min_y + 1) as u32);

        self.upload.clear();
        for row in min_y..=max_y {
            let start = (row * canvas_w + min_x) * 4;
            self.upload
                .extend_from_slice(&pixels[start..start + w as usize * 4]);
        }
        let (x, y) = self.store(queue, w, h)?;
        Ok(Glyph {
            x,
            y,
            w: w as u16,
            h: h as u16,
            offset_x: (min_x as i32 - self.pad_x as i32) as i16,
            offset_y: (min_y as i32 - self.pad_y as i32) as i16,
            color,
        })
    }

    /// Block elements are drawn from their definition instead of a font, so
    /// they always fill the cell exactly and tile without seams.
    fn rasterize_block(
        &mut self,
        queue: &wgpu::Queue,
        (alpha, rects): (u8, [[u8; 4]; 2]),
        wide: bool,
    ) -> Result<Glyph, AtlasFull> {
        let w = self.metrics.cell_w * if wide { 2 } else { 1 };
        let h = self.metrics.cell_h;
        // Edges in eighths of the cell, snapped to whole pixels.
        let edge = |eighths: u8, extent: u32| (eighths as u32 * extent + 4) / 8;
        self.upload.clear();
        for y in 0..h {
            for x in 0..w {
                let inside = rects.iter().any(|&[x0, y0, x1, y1]| {
                    (edge(x0, w)..edge(x1, w)).contains(&x) && (edge(y0, h)..edge(y1, h)).contains(&y)
                });
                self.upload
                    .extend_from_slice(&[255, 255, 255, if inside { alpha } else { 0 }]);
            }
        }
        let (x, y) = self.store(queue, w, h)?;
        Ok(Glyph {
            x,
            y,
            w: w as u16,
            h: h as u16,
            ..Glyph::default()
        })
    }

    /// Reserves a `w`x`h` slot and uploads `self.upload` into it.
    fn store(&mut self, queue: &wgpu::Queue, w: u32, h: u32) -> Result<(u16, u16), AtlasFull> {
        if self.shelf_x + w > self.size {
            self.shelf_y += self.shelf_h;
            self.shelf_x = 0;
            self.shelf_h = 0;
        }
        if w > self.size || self.shelf_y + h > self.size {
            return Err(AtlasFull);
        }
        let (atlas_x, atlas_y) = (self.shelf_x, self.shelf_y);
        self.shelf_x += w;
        self.shelf_h = self.shelf_h.max(h);

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: atlas_x,
                    y: atlas_y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &self.upload,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        Ok((atlas_x as u16, atlas_y as u16))
    }
}

/// Coverage and up to two rectangles (`[x0, y0, x1, y1]` in eighths of the
/// cell) for the Unicode block elements U+2580..=U+259F.
fn block_element(ch: char) -> Option<(u8, [[u8; 4]; 2])> {
    const NONE: [u8; 4] = [0; 4];
    const FULL: [u8; 4] = [0, 0, 8, 8];
    const TOP: [u8; 4] = [0, 0, 8, 4];
    const BOTTOM: [u8; 4] = [0, 4, 8, 8];
    const LEFT: [u8; 4] = [0, 0, 4, 8];
    const UL: [u8; 4] = [0, 0, 4, 4];
    const UR: [u8; 4] = [4, 0, 8, 4];
    const LL: [u8; 4] = [0, 4, 4, 8];
    const LR: [u8; 4] = [4, 4, 8, 8];

    let code = ch as u32;
    let solid = |a, b| Some((255, [a, b]));
    match code {
        0x2580 => solid(TOP, NONE),
        // Lower one eighth up to the full block.
        0x2581..=0x2588 => solid([0, (0x2588 - code) as u8, 8, 8], NONE),
        // Left seven eighths down to one eighth.
        0x2589..=0x258f => solid([0, 0, (0x2590 - code) as u8, 8], NONE),
        0x2590 => solid([4, 0, 8, 8], NONE),
        0x2591 => Some((64, [FULL, NONE])),
        0x2592 => Some((128, [FULL, NONE])),
        0x2593 => Some((191, [FULL, NONE])),
        0x2594 => solid([0, 0, 8, 1], NONE),
        0x2595 => solid([7, 0, 8, 8], NONE),
        0x2596 => solid(LL, NONE),
        0x2597 => solid(LR, NONE),
        0x2598 => solid(UL, NONE),
        0x2599 => solid(LEFT, LR),
        0x259a => solid(UL, LR),
        0x259b => solid(TOP, LL),
        0x259c => solid(TOP, LR),
        0x259d => solid(UR, NONE),
        0x259e => solid(UR, LL),
        0x259f => solid(BOTTOM, UR),
        _ => None,
    }
}
