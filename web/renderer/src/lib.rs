//! WebGPU renderer for a [`webterm_frame::Frame`].
//!
//! One instanced quad per cell, drawn in two passes over the same instance
//! buffer (backgrounds and line decorations, then glyphs from an on-demand
//! atlas) plus a tiny third draw for the underline and beam cursors. All
//! geometry is in whole device pixels, so text stays crisp at any DPR.
//!
//! WebGPU is the only backend: there is no WebGL fallback, and
//! [`Renderer::new`] fails when `navigator.gpu` yields no adapter.

mod atlas;

use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use webterm_frame::{Cell, CursorShape, Frame, Rgb, flags};

use atlas::{AtlasFull, Glyph, GlyphAtlas, Metrics, STYLE_BOLD, STYLE_ITALIC};

pub struct RendererConfig {
    /// CSS font-family list, e.g. `"Consolas, 'Cascadia Mono', monospace"`.
    pub font_family: String,
    /// Font size in CSS pixels.
    pub font_size_px: f32,
    pub device_pixel_ratio: f32,
}

const INSTANCE_UNDERLINE: u32 = 1;
const INSTANCE_STRIKEOUT: u32 = 2;
const INSTANCE_COLOR_GLYPH: u32 = 4;

/// Per-cell instance data; must match `Instance` in `shader.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    atlas_pos: [u16; 2],
    glyph_size: [u16; 2],
    glyph_offset: [i16; 2],
    fg: [u8; 4],
    bg: [u8; 4],
    flags: u32,
}

impl Instance {
    /// Never produced for a real cell, so it always compares as changed.
    const STALE: Self = Self {
        atlas_pos: [0; 2],
        glyph_size: [0; 2],
        glyph_offset: [0; 2],
        fg: [0; 4],
        bg: [0; 4],
        flags: u32::MAX,
    };
}

const INSTANCE_SIZE: usize = size_of::<Instance>();

const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
    0 => Uint16x2,
    1 => Uint16x2,
    2 => Sint16x2,
    3 => Unorm8x4,
    4 => Unorm8x4,
    5 => Uint32,
];

/// Must match `Globals` in `shader.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Pod, Zeroable)]
struct Globals {
    viewport: [f32; 2],
    cell: [f32; 2],
    cols: u32,
    underline_y: f32,
    strike_y: f32,
    line_thickness: f32,
    cursor_rect: [f32; 4],
    cursor_color: [f32; 4],
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,
    max_surface_dimension: u32,

    bg_pipeline: wgpu::RenderPipeline,
    glyph_pipeline: wgpu::RenderPipeline,
    cursor_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,

    globals_buffer: wgpu::Buffer,
    /// What `globals_buffer` currently holds.
    globals: Option<Globals>,

    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    /// CPU copy of what `instance_buffer` holds, to upload only changed rows.
    instances: Vec<Instance>,
    instance_cols: usize,
    row_scratch: Vec<Instance>,

    atlas: GlyphAtlas,
    font_family: String,
    font_size_px: f32,
    device_pixel_ratio: f32,
}

fn sanitize_dpr(device_pixel_ratio: f32) -> f32 {
    if device_pixel_ratio.is_finite() && device_pixel_ratio > 0.0 {
        device_pixel_ratio
    } else {
        1.0
    }
}

fn rgba(color: Rgb) -> [u8; 4] {
    [color.r, color.g, color.b, 255]
}

fn create_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("webterm cells"),
        size: (capacity * INSTANCE_SIZE) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    globals: &wgpu::Buffer,
    atlas: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("webterm bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(atlas),
            },
        ],
    })
}

impl Renderer {
    pub async fn new(
        canvas: web_sys::HtmlCanvasElement,
        config: RendererConfig,
    ) -> Result<Self, String> {
        let mut instance_desc = wgpu::InstanceDescriptor::new_without_display_handle();
        instance_desc.backends = wgpu::Backends::BROWSER_WEBGPU;
        let instance = wgpu::Instance::new(instance_desc);

        let (width, height) = (canvas.width(), canvas.height());
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
            .map_err(|e| format!("cannot create a WebGPU surface for the canvas: {e}"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(|e| format!("WebGPU is not available (no adapter): {e}"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("webterm"),
                ..Default::default()
            })
            .await
            .map_err(|e| format!("cannot open the WebGPU device: {e}"))?;

        // Colours are passed through untouched and blended in display space,
        // like browser text, so pick a format without sRGB encoding.
        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(|format| !format.is_srgb())
            .or_else(|| capabilities.formats.first().copied())
            .ok_or("the canvas supports no WebGPU surface format")?;
        let max_surface_dimension = device.limits().max_texture_dimension_2d;
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: Default::default(),
            width: width.clamp(1, max_surface_dimension),
            height: height.clamp(1, max_surface_dimension),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: Vec::new(),
        };
        surface.configure(&device, &surface_config);

        let device_pixel_ratio = sanitize_dpr(config.device_pixel_ratio);
        let atlas = GlyphAtlas::new(
            &device,
            &config.font_family,
            config.font_size_px * device_pixel_ratio,
        )?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("webterm shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("webterm bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("webterm pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let instance_layout = Some(wgpu::VertexBufferLayout {
            array_stride: INSTANCE_SIZE as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &INSTANCE_ATTRIBUTES,
        });
        let pipeline = |label, vertex, fragment, per_cell: bool, blend| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex),
                    compilation_options: Default::default(),
                    buffers: if per_cell {
                        std::slice::from_ref(&instance_layout)
                    } else {
                        &[]
                    },
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let bg_pipeline = pipeline("webterm backgrounds", "vs_bg", "fs_bg", true, None);
        let glyph_pipeline = pipeline(
            "webterm glyphs",
            "vs_glyph",
            "fs_glyph",
            true,
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );
        let cursor_pipeline = pipeline("webterm cursor", "vs_cursor", "fs_cursor", false, None);

        let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("webterm globals"),
            size: size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group =
            create_bind_group(&device, &bind_group_layout, &globals_buffer, atlas.view());

        // Enough for 200x60 up front; grows on demand.
        let instance_capacity = 200 * 60;
        let instance_buffer = create_instance_buffer(&device, instance_capacity);

        Ok(Self {
            surface,
            device,
            queue,
            surface_config,
            max_surface_dimension,
            bg_pipeline,
            glyph_pipeline,
            cursor_pipeline,
            bind_group_layout,
            bind_group,
            globals_buffer,
            globals: None,
            instance_buffer,
            instance_capacity,
            instances: Vec::new(),
            instance_cols: 0,
            row_scratch: Vec::new(),
            atlas,
            font_family: config.font_family,
            font_size_px: config.font_size_px,
            device_pixel_ratio,
        })
    }

    /// Cell size in CSS pixels, so the caller can derive cols/rows from the element size.
    pub fn cell_size(&self) -> (f32, f32) {
        let (width, height) = self.cell_size_px();
        (
            width as f32 / self.device_pixel_ratio,
            height as f32 / self.device_pixel_ratio,
        )
    }

    /// Cell size in device pixels; always whole numbers.
    pub fn cell_size_px(&self) -> (u32, u32) {
        let metrics = self.atlas.metrics();
        (metrics.cell_w, metrics.cell_h)
    }

    /// Number of whole `(cols, rows)` that fit the current backing store.
    pub fn grid_size(&self) -> (u16, u16) {
        let (cell_w, cell_h) = self.cell_size_px();
        let fit = |pixels: u32, cell: u32| (pixels / cell).clamp(1, u16::MAX as u32) as u16;
        (
            fit(self.surface_config.width, cell_w),
            fit(self.surface_config.height, cell_h),
        )
    }

    /// Canvas backing-store size in device pixels, plus the current DPR.
    ///
    /// Sets `canvas.width`/`height` itself and clears the canvas, so call
    /// [`Renderer::render`] afterwards. A DPR change also changes
    /// [`Renderer::cell_size`].
    pub fn resize(&mut self, width_px: u32, height_px: u32, device_pixel_ratio: f32) {
        let device_pixel_ratio = sanitize_dpr(device_pixel_ratio);
        if device_pixel_ratio != self.device_pixel_ratio {
            self.device_pixel_ratio = device_pixel_ratio;
            self.reload_font();
        }

        let width = width_px.clamp(1, self.max_surface_dimension);
        let height = height_px.clamp(1, self.max_surface_dimension);
        if (width, height) != (self.surface_config.width, self.surface_config.height) {
            self.surface_config.width = width;
            self.surface_config.height = height;
            self.surface.configure(&self.device, &self.surface_config);
        }
    }

    /// Changes the font. Changes [`Renderer::cell_size`]; call
    /// [`Renderer::render`] afterwards.
    pub fn set_font(&mut self, font_family: &str, font_size_px: f32) {
        self.font_family = font_family.to_owned();
        self.font_size_px = font_size_px;
        self.reload_font();
    }

    /// Drops every rasterised glyph and re-measures the cell, e.g. once a
    /// web font the family list refers to has finished loading.
    pub fn reload_font(&mut self) {
        // Measuring text cannot realistically fail once the context exists;
        // if it does the previous metrics stay in place.
        let _ = self.atlas.set_font(
            &self.font_family,
            self.font_size_px * self.device_pixel_ratio,
        );
        // Cached instances point at glyphs that no longer exist.
        self.instances.fill(Instance::STALE);
    }

    pub fn render(&mut self, frame: &Frame) -> Result<(), String> {
        let cols = frame.cols as usize;
        let count = cols * frame.rows as usize;
        if frame.cells.len() != count {
            return Err(format!(
                "frame has {} cells, expected {}x{}",
                frame.cells.len(),
                frame.cols,
                frame.rows
            ));
        }

        if count > self.instance_capacity {
            self.instance_capacity = count.next_power_of_two();
            self.instance_buffer = create_instance_buffer(&self.device, self.instance_capacity);
            self.instances.clear();
        }
        if self.instances.len() != count || self.instance_cols != cols {
            self.instances.clear();
            self.instances.resize(count, Instance::STALE);
            self.instance_cols = cols;
        }

        // A full atlas invalidates glyphs already placed this frame, so start
        // over after making room. The last attempt drops what still won't fit.
        let mut dirty_rows: Option<Range<usize>> = None;
        for attempt in 0..3 {
            if self
                .build_instances(frame, &mut dirty_rows, attempt == 2)
                .is_ok()
            {
                break;
            }
            if self.atlas.grow_or_reset(&self.device) {
                self.bind_group = create_bind_group(
                    &self.device,
                    &self.bind_group_layout,
                    &self.globals_buffer,
                    self.atlas.view(),
                );
            }
        }
        if let Some(rows) = dirty_rows {
            let cells = rows.start * cols..rows.end * cols;
            self.queue.write_buffer(
                &self.instance_buffer,
                (cells.start * INSTANCE_SIZE) as u64,
                bytemuck::cast_slice(&self.instances[cells]),
            );
        }

        let globals = self.globals_for(frame);
        if self.globals != Some(globals) {
            self.queue
                .write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(&globals));
            self.globals = Some(globals);
        }

        let target = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.surface_config);
                match self.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(texture)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
                    other => return Err(format!("cannot acquire the canvas texture: {other:?}")),
                }
            }
            // Nothing is visible; the caller redraws on the next dirty frame.
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("cannot acquire the canvas texture: validation error".into());
            }
        };
        let view = target
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let background = frame.background;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("webterm frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: background.r as f64 / 255.0,
                            g: background.g as f64 / 255.0,
                            b: background.b as f64 / 255.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_bind_group(0, &self.bind_group, &[]);
            if count > 0 {
                pass.set_vertex_buffer(
                    0,
                    self.instance_buffer.slice(..(count * INSTANCE_SIZE) as u64),
                );
                pass.set_pipeline(&self.bg_pipeline);
                pass.draw(0..4, 0..count as u32);
                pass.set_pipeline(&self.glyph_pipeline);
                pass.draw(0..4, 0..count as u32);
            }
            if globals.cursor_rect[2] > 0.0 {
                pass.set_pipeline(&self.cursor_pipeline);
                pass.draw(0..4, 0..1);
            }
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(target);
        Ok(())
    }

    /// Rebuilds the per-cell instances, widening `dirty_rows` to cover every
    /// row whose instances differ from what the GPU buffer holds.
    fn build_instances(
        &mut self,
        frame: &Frame,
        dirty_rows: &mut Option<Range<usize>>,
        drop_on_full: bool,
    ) -> Result<(), AtlasFull> {
        let cols = frame.cols as usize;
        if cols == 0 {
            return Ok(());
        }

        // The block cursor is the cell itself with swapped colours, so the
        // character under it stays readable; it covers both halves of a
        // double-width character.
        let mut block_cells = 0..0;
        let mut block_color = Rgb::default();
        if let Some(cursor) = frame.cursor
            && cursor.shape == CursorShape::Block
            && cursor.col < frame.cols
            && cursor.row < frame.rows
        {
            let start = cursor.row as usize * cols + cursor.col as usize;
            block_cells = start..start + cursor_span(frame, cursor.col, cursor.row);
            block_color = cursor.color;
        }

        let Self {
            atlas,
            queue,
            instances,
            row_scratch,
            ..
        } = self;
        row_scratch.clear();
        row_scratch.resize(cols, Instance::STALE);

        for (row, cells) in frame.cells.chunks_exact(cols).enumerate() {
            let row_start = row * cols;
            for (col, (cell, instance)) in cells.iter().zip(row_scratch.iter_mut()).enumerate() {
                let (fg, bg) = if block_cells.contains(&(row_start + col)) {
                    (cell.bg, block_color)
                } else {
                    (cell.fg, cell.bg)
                };
                let glyph = match cell_glyph(atlas, queue, cell) {
                    Ok(glyph) => glyph,
                    Err(AtlasFull) if drop_on_full => Glyph::default(),
                    Err(full) => return Err(full),
                };
                let mut instance_flags = 0;
                if cell.flags & flags::UNDERLINE != 0 {
                    instance_flags |= INSTANCE_UNDERLINE;
                }
                if cell.flags & flags::STRIKEOUT != 0 {
                    instance_flags |= INSTANCE_STRIKEOUT;
                }
                if glyph.color {
                    instance_flags |= INSTANCE_COLOR_GLYPH;
                }
                *instance = Instance {
                    atlas_pos: [glyph.x, glyph.y],
                    glyph_size: [glyph.w, glyph.h],
                    glyph_offset: [glyph.offset_x, glyph.offset_y],
                    fg: rgba(fg),
                    bg: rgba(bg),
                    flags: instance_flags,
                };
            }

            let uploaded = &mut instances[row_start..row_start + cols];
            if bytemuck::cast_slice::<_, u8>(uploaded)
                != bytemuck::cast_slice::<_, u8>(row_scratch.as_slice())
            {
                uploaded.copy_from_slice(row_scratch);
                *dirty_rows = Some(match dirty_rows.take() {
                    Some(rows) => rows.start.min(row)..rows.end.max(row + 1),
                    None => row..row + 1,
                });
            }
        }
        Ok(())
    }

    fn globals_for(&self, frame: &Frame) -> Globals {
        let Metrics {
            cell_w,
            cell_h,
            underline_y,
            strike_y,
            line_thickness,
            cursor_thickness,
            ..
        } = *self.atlas.metrics();

        let mut cursor_rect = [0.0; 4];
        let mut cursor_color = [0.0; 4];
        if let Some(cursor) = frame.cursor
            && cursor.shape != CursorShape::Block
            && cursor.col < frame.cols
            && cursor.row < frame.rows
        {
            let x = cursor.col as u32 * cell_w;
            let y = cursor.row as u32 * cell_h;
            let rect = match cursor.shape {
                CursorShape::Underline => {
                    let thickness = cursor_thickness.min(cell_h);
                    let width = cell_w * cursor_span(frame, cursor.col, cursor.row) as u32;
                    [x, y + cell_h - thickness, width, thickness]
                }
                _ => [x, y, cursor_thickness, cell_h],
            };
            cursor_rect = rect.map(|v| v as f32);
            cursor_color = [
                cursor.color.r as f32 / 255.0,
                cursor.color.g as f32 / 255.0,
                cursor.color.b as f32 / 255.0,
                1.0,
            ];
        }

        Globals {
            viewport: [
                self.surface_config.width as f32,
                self.surface_config.height as f32,
            ],
            cell: [cell_w as f32, cell_h as f32],
            cols: (frame.cols as u32).max(1),
            underline_y: underline_y as f32,
            strike_y: strike_y as f32,
            line_thickness: line_thickness as f32,
            cursor_rect,
            cursor_color,
        }
    }
}

/// Number of cells a cursor at this position covers: two on a double-width
/// character that has its spacer, otherwise one.
fn cursor_span(frame: &Frame, col: u16, row: u16) -> usize {
    let wide = frame.cell(col, row).flags & flags::WIDE != 0;
    if wide && col + 1 < frame.cols { 2 } else { 1 }
}

fn cell_glyph(atlas: &mut GlyphAtlas, queue: &wgpu::Queue, cell: &Cell) -> Result<Glyph, AtlasFull> {
    // Blank and control characters, and the second half of a double-width
    // character, draw background only.
    if cell.ch <= ' ' || cell.flags & flags::WIDE_SPACER != 0 {
        return Ok(Glyph::default());
    }
    let mut style = 0;
    if cell.flags & flags::BOLD != 0 {
        style |= STYLE_BOLD;
    }
    if cell.flags & flags::ITALIC != 0 {
        style |= STYLE_ITALIC;
    }
    atlas.glyph(queue, cell.ch, style, cell.flags & flags::WIDE != 0)
}
