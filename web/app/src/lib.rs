//! wasm-bindgen glue: one terminal core plus one WebGPU renderer per canvas.

use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;
use webterm_core::Terminal;
use webterm_frame::Frame;
use webterm_renderer::{Renderer, RendererConfig};

const SCROLLBACK_LINES: usize = 10_000;
const INITIAL_COLS: u16 = 80;
const INITIAL_ROWS: u16 = 24;

#[wasm_bindgen]
pub struct App {
    terminal: Terminal,
    renderer: Renderer,
    frame: Frame,
    cols: u16,
    rows: u16,
    /// Set when the renderer changed size and must repaint even if the
    /// terminal content did not change.
    needs_redraw: bool,
}

#[wasm_bindgen]
impl App {
    /// Rejects when WebGPU is unavailable.
    pub async fn create(
        canvas: HtmlCanvasElement,
        font_family: String,
        font_size_px: f32,
        device_pixel_ratio: f32,
    ) -> Result<App, JsValue> {
        console_error_panic_hook::set_once();
        let config = RendererConfig {
            font_family,
            font_size_px,
            device_pixel_ratio,
        };
        let renderer = Renderer::new(canvas, config).await?;
        Ok(App {
            terminal: Terminal::new(INITIAL_COLS, INITIAL_ROWS, SCROLLBACK_LINES),
            renderer,
            frame: Frame::new(INITIAL_COLS, INITIAL_ROWS),
            cols: INITIAL_COLS,
            rows: INITIAL_ROWS,
            needs_redraw: true,
        })
    }

    /// Fits the grid to the element's CSS size. Call on layout or DPR change.
    pub fn fit(&mut self, css_width: f32, css_height: f32, device_pixel_ratio: f32) {
        let width_px = ((css_width * device_pixel_ratio).round() as u32).max(1);
        let height_px = ((css_height * device_pixel_ratio).round() as u32).max(1);
        self.renderer
            .resize(width_px, height_px, device_pixel_ratio);

        // Whole cells that fit the backing store; CSS cell sizes can be fractional.
        let (cols, rows) = self.renderer.grid_size();
        let cols = cols.max(2);
        if (cols, rows) != (self.cols, self.rows) {
            self.terminal.resize(cols, rows);
            (self.cols, self.rows) = (cols, rows);
        }
        self.needs_redraw = true;
    }

    /// Bytes received from the serial port.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.terminal.feed(bytes);
    }

    /// Replies the terminal wants written back to the serial port.
    pub fn take_output(&mut self) -> Vec<u8> {
        self.terminal.take_output()
    }

    /// Repaints if anything changed. Cheap to call every animation frame.
    pub fn draw(&mut self) -> Result<(), JsValue> {
        if !self.needs_redraw && !self.terminal.is_dirty() {
            return Ok(());
        }
        self.terminal.render(&mut self.frame);
        self.renderer.render(&self.frame)?;
        self.needs_redraw = false;
        Ok(())
    }

    pub fn scroll_lines(&mut self, delta: i32) {
        self.terminal.scroll_lines(delta);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.terminal.scroll_to_bottom();
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    pub fn app_cursor_keys(&self) -> bool {
        self.terminal.app_cursor_keys()
    }

    pub fn bracketed_paste(&self) -> bool {
        self.terminal.bracketed_paste()
    }

    pub fn line_count(&self) -> usize {
        self.terminal.line_count()
    }

    /// Text of lines `start..end`, oldest first.
    pub fn lines_text(&self, start: usize, end: usize) -> Vec<String> {
        self.terminal.lines_text(start, end)
    }
}
