//! Standalone check of `webterm-renderer`: draws a fixed test pattern, with
//! no terminal core involved.
//!
//! Query parameters:
//! - `cursor=block|underline|beam|none` (default `block`); any key press or
//!   click cycles through them
//! - `wide=1` puts the cursor on a double-width Korean character
//! - `font=<css font-family>` and `size=<css px>`
//! - `bench=1` times `Renderer::render` on a 200x60 grid instead

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use web_sys::{HtmlCanvasElement, HtmlElement, UrlSearchParams, Window};
use webterm_frame::{Cell, Cursor, CursorShape, Frame, Rgb, flags};
use webterm_renderer::{Renderer, RendererConfig};

const DEFAULT_FONT: &str = "Consolas, 'Cascadia Mono', 'D2Coding', monospace";
const DEFAULT_SIZE: f32 = 14.0;

/// The 16 ANSI colours (xterm defaults).
const ANSI: [Rgb; 16] = [
    Rgb::new(0x00, 0x00, 0x00),
    Rgb::new(0xcd, 0x00, 0x00),
    Rgb::new(0x00, 0xcd, 0x00),
    Rgb::new(0xcd, 0xcd, 0x00),
    Rgb::new(0x00, 0x00, 0xee),
    Rgb::new(0xcd, 0x00, 0xcd),
    Rgb::new(0x00, 0xcd, 0xcd),
    Rgb::new(0xe5, 0xe5, 0xe5),
    Rgb::new(0x7f, 0x7f, 0x7f),
    Rgb::new(0xff, 0x00, 0x00),
    Rgb::new(0x00, 0xff, 0x00),
    Rgb::new(0xff, 0xff, 0x00),
    Rgb::new(0x5c, 0x5c, 0xff),
    Rgb::new(0xff, 0x00, 0xff),
    Rgb::new(0x00, 0xff, 0xff),
    Rgb::new(0xff, 0xff, 0xff),
];

struct Demo {
    window: Window,
    canvas: HtmlCanvasElement,
    status: HtmlElement,
    renderer: Renderer,
    cursor: Option<CursorShape>,
    cursor_on_wide: bool,
}

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    wasm_bindgen_futures::spawn_local(async {
        if let Err(message) = run().await {
            report(&format!("error: {message}"), false);
        }
    });
}

/// Shows a status line under the canvas; `data-state` lets automation tell
/// when the first frame has been submitted.
fn report(message: &str, ok: bool) {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    if let Some(status) = document.get_element_by_id("status") {
        status.set_text_content(Some(message));
        let _ = status.set_attribute("data-state", if ok { "ok" } else { "error" });
    }
}

fn js(err: JsValue) -> String {
    format!("{err:?}")
}

async fn run() -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let canvas = document
        .get_element_by_id("term")
        .ok_or("no #term canvas")?
        .dyn_into::<HtmlCanvasElement>()
        .map_err(|_| "#term is not a canvas")?;
    let status = document
        .get_element_by_id("status")
        .ok_or("no #status element")?
        .dyn_into::<HtmlElement>()
        .map_err(|_| "#status is not an HTML element")?;

    let search = window.location().search().map_err(js)?;
    let params = UrlSearchParams::new_with_str(&search).map_err(js)?;
    let cursor = match params.get("cursor").as_deref() {
        Some("underline") => Some(CursorShape::Underline),
        Some("beam") => Some(CursorShape::Beam),
        Some("none") => None,
        _ => Some(CursorShape::Block),
    };

    let renderer = Renderer::new(
        canvas.clone(),
        RendererConfig {
            font_family: params.get("font").unwrap_or_else(|| DEFAULT_FONT.into()),
            font_size_px: params
                .get("size")
                .and_then(|size| size.parse().ok())
                .unwrap_or(DEFAULT_SIZE),
            device_pixel_ratio: window.device_pixel_ratio() as f32,
        },
    )
    .await?;

    let mut demo = Demo {
        window: window.clone(),
        canvas,
        status,
        renderer,
        cursor,
        cursor_on_wide: params.get("wide").is_some(),
    };

    if params.get("bench").is_some() {
        return demo.bench().await;
    }

    demo.redraw()?;
    let demo = Rc::new(RefCell::new(demo));

    let on_resize = {
        let demo = demo.clone();
        Closure::<dyn FnMut()>::new(move || demo.borrow_mut().redraw_or_report())
    };
    window
        .add_event_listener_with_callback("resize", on_resize.as_ref().unchecked_ref())
        .map_err(js)?;
    on_resize.forget();

    let on_input = Closure::<dyn FnMut()>::new(move || {
        let mut demo = demo.borrow_mut();
        demo.cursor = match demo.cursor {
            Some(CursorShape::Block) => Some(CursorShape::Underline),
            Some(CursorShape::Underline) => Some(CursorShape::Beam),
            Some(CursorShape::Beam) => None,
            None => Some(CursorShape::Block),
        };
        demo.redraw_or_report();
    });
    for event in ["keydown", "click"] {
        window
            .add_event_listener_with_callback(event, on_input.as_ref().unchecked_ref())
            .map_err(js)?;
    }
    on_input.forget();
    Ok(())
}

impl Demo {
    fn now(&self) -> f64 {
        self.window.performance().map_or(0.0, |p| p.now())
    }

    fn redraw_or_report(&mut self) {
        if let Err(message) = self.redraw() {
            report(&format!("error: {message}"), false);
        }
    }

    /// Fits the backing store to the element and draws the test pattern.
    fn redraw(&mut self) -> Result<(), String> {
        let dpr = self.window.device_pixel_ratio();
        let width = (self.canvas.client_width() as f64 * dpr).round() as u32;
        let height = (self.canvas.client_height() as f64 * dpr).round() as u32;
        self.renderer.resize(width, height, dpr as f32);

        let (cols, rows) = self.renderer.grid_size();
        let frame = test_pattern(cols, rows, self.cursor, self.cursor_on_wide);
        let started = self.now();
        self.renderer.render(&frame)?;
        let elapsed = self.now() - started;

        let (cell_w, cell_h) = self.renderer.cell_size_px();
        let (css_w, css_h) = self.renderer.cell_size();
        self.status.set_text_content(Some(&format!(
            "ok | canvas {width}x{height} device px | dpr {dpr} | cell {cell_w}x{cell_h} device px \
             ({css_w:.3}x{css_h:.3} css) | grid {cols}x{rows} | cursor {:?} | render() {elapsed:.2} ms",
            self.cursor
        )));
        self.status.set_attribute("data-state", "ok").map_err(js)
    }

    /// Times the CPU side of `render` on a full 200x60 grid, one render per
    /// animation frame: every cell changing each frame, then an unchanged frame.
    async fn bench(&mut self) -> Result<(), String> {
        const COLS: u16 = 200;
        const ROWS: u16 = 60;
        const RUNS: usize = 120;

        let dpr = self.window.device_pixel_ratio();
        let (cell_w, cell_h) = self.renderer.cell_size_px();
        let (width, height) = (COLS as u32 * cell_w, ROWS as u32 * cell_h);
        // Show the backing store 1:1 instead of scaled to the window.
        let style = self.canvas.style();
        style.set_property("flex", "none").map_err(js)?;
        style
            .set_property("width", &format!("{}px", width as f64 / dpr))
            .map_err(js)?;
        style
            .set_property("height", &format!("{}px", height as f64 / dpr))
            .map_err(js)?;
        self.renderer.resize(width, height, dpr as f32);

        let mut frame = Frame::new(COLS, ROWS);
        let mut seed = 0x2545_f491_u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let mut scramble = |frame: &mut Frame| {
            for cell in &mut frame.cells {
                let bits = next();
                cell.ch = char::from(b'!' + (bits % 94) as u8);
                cell.fg = ANSI[(bits >> 8) as usize % 16];
                cell.bg = ANSI[(bits >> 12) as usize % 8];
                cell.flags = (bits >> 16) as u8 & (flags::BOLD | flags::ITALIC | flags::UNDERLINE);
            }
        };

        // First frame pays for rasterising every glyph in four styles.
        scramble(&mut frame);
        let started = self.now();
        self.renderer.render(&frame)?;
        let first = self.now() - started;

        let (mut total, mut worst) = (0.0f64, 0.0f64);
        for _ in 0..RUNS {
            next_animation_frame(&self.window).await?;
            scramble(&mut frame);
            let started = self.now();
            self.renderer.render(&frame)?;
            let elapsed = self.now() - started;
            total += elapsed;
            worst = worst.max(elapsed);
        }
        let changing = total / RUNS as f64;

        let mut total = 0.0f64;
        for _ in 0..RUNS {
            next_animation_frame(&self.window).await?;
            let started = self.now();
            self.renderer.render(&frame)?;
            total += self.now() - started;
        }
        let unchanged = total / RUNS as f64;

        self.status.set_text_content(Some(&format!(
            "ok | bench {COLS}x{ROWS} on {width}x{height} device px | first frame {first:.2} ms | \
             all cells changing: avg {changing:.3} ms, worst {worst:.3} ms | \
             unchanged frame: avg {unchanged:.3} ms | CPU time inside render() only, {RUNS} runs"
        )));
        self.status.set_attribute("data-state", "ok").map_err(js)
    }
}

async fn next_animation_frame(window: &Window) -> Result<(), String> {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let _ = window.request_animation_frame(&resolve);
    });
    wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map(drop)
        .map_err(js)
}

/// Characters the terminal core would report as double width.
fn is_wide(ch: char) -> bool {
    matches!(ch as u32,
        0x1100..=0x115f | 0x2e80..=0xa4cf | 0xac00..=0xd7a3 | 0xf900..=0xfaff
        | 0xff00..=0xff60 | 0x1f300..=0x1faff)
}

struct Pen {
    fg: Rgb,
    bg: Rgb,
    flags: u8,
}

/// Writes `text` starting at `(col, row)`, laying out double-width characters
/// as WIDE + WIDE_SPACER like the core does. Returns the next free column.
fn put(frame: &mut Frame, mut col: u16, row: u16, text: &str, pen: &Pen) -> u16 {
    if row >= frame.rows {
        return col;
    }
    for ch in text.chars() {
        let wide = is_wide(ch);
        if col + wide as u16 >= frame.cols {
            break;
        }
        *frame.cell_mut(col, row) = Cell {
            ch,
            fg: pen.fg,
            bg: pen.bg,
            flags: pen.flags | if wide { flags::WIDE } else { 0 },
        };
        col += 1;
        if wide {
            *frame.cell_mut(col, row) = Cell {
                ch: ' ',
                fg: pen.fg,
                bg: pen.bg,
                flags: pen.flags | flags::WIDE_SPACER,
            };
            col += 1;
        }
    }
    col
}

fn test_pattern(cols: u16, rows: u16, cursor: Option<CursorShape>, cursor_on_wide: bool) -> Frame {
    let mut frame = Frame::new(cols, rows);
    let default = Cell::default();
    let plain = |flags| Pen {
        fg: default.fg,
        bg: default.bg,
        flags,
    };
    let dim = Pen {
        fg: ANSI[8],
        bg: default.bg,
        flags: 0,
    };
    let sample = "The quick brown fox jumps over the lazy dog 0123456789";

    put(&mut frame, 0, 0, "WebTerm renderer test pattern", &plain(flags::BOLD));

    put(&mut frame, 0, 2, "ascii      ", &dim);
    let ascii: String = (b'!'..=b'~').map(char::from).collect();
    put(&mut frame, 11, 2, &ascii, &plain(0));

    let styles = [
        ("normal     ", 0),
        ("bold       ", flags::BOLD),
        ("italic     ", flags::ITALIC),
        ("bold+ital  ", flags::BOLD | flags::ITALIC),
        ("underline  ", flags::UNDERLINE),
        ("strikeout  ", flags::STRIKEOUT),
        ("all four   ", flags::BOLD | flags::ITALIC | flags::UNDERLINE | flags::STRIKEOUT),
    ];
    for (i, (label, style)) in styles.into_iter().enumerate() {
        let row = 4 + i as u16;
        put(&mut frame, 0, row, label, &dim);
        put(&mut frame, 11, row, sample, &plain(style));
    }

    // The 16 ANSI colours as foreground, then as background.
    put(&mut frame, 0, 12, "ansi fg    ", &dim);
    put(&mut frame, 0, 13, "ansi bg    ", &dim);
    for (i, color) in ANSI.into_iter().enumerate() {
        let col = 11 + i as u16 * 5;
        let label = format!("{i:>2}Ab ");
        let fg = Pen {
            fg: color,
            bg: default.bg,
            flags: 0,
        };
        put(&mut frame, col, 12, &label, &fg);
        let dark = (color.r as u32 * 3 + color.g as u32 * 6 + color.b as u32) < 1280;
        let bg = Pen {
            fg: if dark { ANSI[15] } else { ANSI[0] },
            bg: color,
            flags: 0,
        };
        put(&mut frame, col, 13, &label, &bg);
    }

    // Korean: every syllable is WIDE followed by a WIDE_SPACER.
    put(&mut frame, 0, 15, "korean     ", &dim);
    let korean_end = put(&mut frame, 11, 15, "한글 테스트: 가나다라마바사 안녕하세요, WebTerm 입니다.", &plain(0));
    put(&mut frame, 0, 16, "ko styled  ", &dim);
    let col = put(&mut frame, 11, 16, "굵게 ", &plain(flags::BOLD));
    let col = put(&mut frame, col, 16, "기울임 ", &plain(flags::ITALIC));
    let col = put(&mut frame, col, 16, "밑줄 ", &plain(flags::UNDERLINE));
    let col = put(&mut frame, col, 16, "취소선 ", &plain(flags::STRIKEOUT));
    let reverse = Pen {
        fg: ANSI[0],
        bg: ANSI[11],
        flags: 0,
    };
    put(&mut frame, col, 16, "반전 색상", &reverse);
    // Two narrow cells per wide one: the rulers must line up with the text.
    put(&mut frame, 0, 17, "ruler      ", &dim);
    let ruler: String = (11..korean_end).map(|c| if c % 2 == 1 { '|' } else { '.' }).collect();
    put(&mut frame, 11, 17, &ruler, &dim);

    put(&mut frame, 0, 19, "box/blocks ", &dim);
    put(&mut frame, 11, 19, "┌───┬───┐ ╔═══╗ ▀▁▂▃▄▅▆▇█ ▉▊▋▌▍▎▏▐▕ ░▒▓ ←↑→↓ 😀🚀 é ñ ü Ω π", &plain(0));
    put(&mut frame, 11, 20, "│ a │ b │ ║ c ║ ▖▗▘▙▚▛▜▝▞▟ ██████▓▓▓▒▒▒░░░", &plain(0));
    put(&mut frame, 11, 21, "└───┴───┘ ╚═══╝ ██████████", &plain(0));

    // The one real cursor; its shape comes from the URL / key presses.
    put(&mut frame, 0, 23, "cursor     ", &dim);
    let name = match cursor {
        Some(CursorShape::Block) => "block",
        Some(CursorShape::Underline) => "underline",
        Some(CursorShape::Beam) => "beam",
        None => "none",
    };
    let col = put(&mut frame, 11, 23, &format!("{name:<10}> "), &plain(0));
    put(&mut frame, col, 23, "abc 한글 xyz   (press a key to cycle)", &plain(0));
    let cursor_col = if cursor_on_wide { col + 4 } else { col + 1 };
    if rows > 23 && cursor_col < cols {
        frame.cursor = cursor.map(|shape| Cursor {
            col: cursor_col,
            row: 23,
            shape,
            color: Rgb::new(0xff, 0xb0, 0x30),
        });
    }

    // Grid extents: a column ruler on the last row and a marker in the last
    // column of every row.
    if rows > 25 {
        for col in 0..cols {
            let digit = char::from(b'0' + (col % 10) as u8);
            *frame.cell_mut(col, rows - 1) = Cell {
                ch: digit,
                fg: if col % 10 == 0 { ANSI[11] } else { ANSI[8] },
                bg: if (col / 10) % 2 == 0 { default.bg } else { Rgb::new(0x20, 0x24, 0x2c) },
                flags: 0,
            };
        }
        for row in 0..rows - 1 {
            *frame.cell_mut(cols - 1, row) = Cell {
                ch: '#',
                fg: ANSI[8],
                bg: default.bg,
                flags: 0,
            };
        }
    }
    frame
}
