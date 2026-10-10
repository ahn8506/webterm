//! Terminal core for WebTerm: VT parsing, grid and scrollback on top of
//! `alacritty_terminal`, with no PTY and no web dependencies.
//!
//! The caller pushes bytes from the serial port into [`Terminal::feed`], sends
//! whatever [`Terminal::take_output`] returns back to the device, and draws
//! the [`Frame`] filled by [`Terminal::render`].

mod palette;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{self, NamedColor, Processor, Rgb as AnsiRgb, Timeout};
use webterm_frame::{Cell, Cursor, CursorShape, Frame, flags};

use crate::palette::Palette;

/// Smallest grid the emulator accepts; a double-width character needs two columns.
const MIN_COLS: u16 = 2;
const MIN_ROWS: u16 = 1;

/// Collects the events `Term` emits while parsing so they can be handled once
/// the parser has returned.
#[derive(Clone, Default)]
struct EventQueue(Rc<RefCell<Vec<Event>>>);

impl EventListener for EventQueue {
    fn send_event(&self, event: Event) {
        self.0.borrow_mut().push(event);
    }
}

/// Disables synchronized updates (`CSI ? 2026 h`): the stock handler needs
/// `Instant::now()`, which panics on wasm32-unknown-unknown, and a device that
/// never ends its update must not freeze the screen.
#[derive(Default)]
struct NoSync;

impl Timeout for NoSync {
    fn set_timeout(&mut self, _duration: Duration) {}

    fn clear_timeout(&mut self) {}

    fn pending_timeout(&self) -> bool {
        false
    }
}

struct Size {
    cols: usize,
    rows: usize,
}

impl Size {
    fn new(cols: u16, rows: u16) -> Self {
        Self { cols: cols.max(MIN_COLS) as usize, rows: rows.max(MIN_ROWS) as usize }
    }
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

pub struct Terminal {
    term: Term<EventQueue>,
    parser: Processor<NoSync>,
    events: EventQueue,
    palette: Palette,
    output: Vec<u8>,
    title: Option<String>,
    bell: bool,
    dirty: bool,
}

impl Terminal {
    /// Creates a terminal; sizes below 2x1 are clamped.
    pub fn new(cols: u16, rows: u16, scrollback_lines: usize) -> Self {
        let events = EventQueue::default();
        let config = Config { scrolling_history: scrollback_lines, ..Config::default() };
        Self {
            term: Term::new(config, &Size::new(cols, rows), events.clone()),
            parser: Processor::new(),
            events,
            palette: Palette::new(),
            output: Vec::new(),
            title: None,
            bell: false,
            dirty: true,
        }
    }

    /// Feed bytes received from the serial port.
    pub fn feed(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.parser.advance(&mut self.term, bytes);
        self.dirty = true;
        self.handle_events();
    }

    /// Resizes the grid, reflowing wrapped lines; sizes below 2x1 are clamped.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let size = Size::new(cols, rows);
        if size.cols == self.term.columns() && size.rows == self.term.screen_lines() {
            return;
        }
        self.term.resize(size);
        self.dirty = true;
        self.handle_events();
    }

    /// Bytes the terminal wants to send back to the device (replies to
    /// DSR/DA queries and similar). Drains the internal buffer.
    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.output)
    }

    /// True if anything visible may have changed since the last `render`.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Fill `frame` with the current viewport (resizing it if needed) and clear the dirty flag.
    pub fn render(&mut self, frame: &mut Frame) {
        let cols = self.term.columns();
        let rows = self.term.screen_lines();
        if frame.cols as usize != cols || frame.rows as usize != rows {
            frame.resize(cols as u16, rows as u16);
        }

        let content = self.term.renderable_content();
        let colors = content.colors;
        let offset = content.display_offset as i32;

        for indexed in content.display_iter {
            let row = (indexed.point.line.0 + offset) as usize;
            let cell = indexed.cell;
            let (fg, bg) = self.palette.cell_colors(colors, cell.fg, cell.bg, cell.flags);

            let mut out = 0;
            for (flag, bit) in [
                (Flags::BOLD, flags::BOLD),
                (Flags::ITALIC, flags::ITALIC),
                (Flags::ALL_UNDERLINES, flags::UNDERLINE),
                (Flags::STRIKEOUT, flags::STRIKEOUT),
                (Flags::WIDE_CHAR, flags::WIDE),
                (Flags::WIDE_CHAR_SPACER, flags::WIDE_SPACER),
            ] {
                if cell.flags.intersects(flag) {
                    out |= bit;
                }
            }

            frame.cells[row * cols + indexed.point.column.0] =
                Cell { ch: printable(cell.c), fg, bg, flags: out };
        }

        frame.background = self.palette.get(colors, NamedColor::Background as usize);

        let shape = match content.cursor.shape {
            ansi::CursorShape::Block | ansi::CursorShape::HollowBlock => Some(CursorShape::Block),
            ansi::CursorShape::Underline => Some(CursorShape::Underline),
            ansi::CursorShape::Beam => Some(CursorShape::Beam),
            ansi::CursorShape::Hidden => None,
        };
        let row = content.cursor.point.line.0 + offset;
        frame.cursor = shape.filter(|_| (0..rows as i32).contains(&row)).map(|shape| Cursor {
            col: content.cursor.point.column.0 as u16,
            row: row as u16,
            shape,
            color: self.palette.get(colors, NamedColor::Cursor as usize),
        });

        self.term.reset_damage();
        self.dirty = false;
    }

    /// Scroll the viewport through scrollback; positive = towards older lines.
    pub fn scroll_lines(&mut self, delta: i32) {
        self.scroll(Scroll::Delta(delta));
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll(Scroll::Bottom);
    }

    /// Lines the viewport is scrolled back by; 0 means it shows the live screen.
    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// Total lines available (scrollback + screen).
    pub fn line_count(&self) -> usize {
        self.term.total_lines()
    }

    /// Text of lines `start..end` (clamped to `line_count`), oldest first, with
    /// trailing whitespace trimmed. Line 0 is the oldest scrollback line; each
    /// entry is one grid row, see [`Terminal::line_wrapped`].
    pub fn lines_text(&self, start: usize, end: usize) -> Vec<String> {
        let end = end.min(self.line_count());
        (start..end).map(|index| self.line_text(index)).collect()
    }

    /// True if line `index` (as in [`Terminal::lines_text`]) soft-wrapped into
    /// the next one, i.e. both belong to the same logical line.
    pub fn line_wrapped(&self, index: usize) -> bool {
        index < self.line_count() && {
            let row = &self.term.grid()[self.line(index)];
            row[Column(self.term.columns() - 1)].flags.contains(Flags::WRAPLINE)
        }
    }

    /// Modes the input encoder needs.
    pub fn app_cursor_keys(&self) -> bool {
        self.term.mode().contains(TermMode::APP_CURSOR)
    }

    pub fn bracketed_paste(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }

    /// Application keypad mode (DECKPAM).
    pub fn app_keypad(&self) -> bool {
        self.term.mode().contains(TermMode::APP_KEYPAD)
    }

    /// True while the alternate screen (vi, less, ...) is active; it has no scrollback.
    pub fn alt_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    pub fn cols(&self) -> u16 {
        self.term.columns() as u16
    }

    pub fn rows(&self) -> u16 {
        self.term.screen_lines() as u16
    }

    /// Window title set by the device (OSC 0/2), if any.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// True if the device rang the bell since the last call.
    pub fn take_bell(&mut self) -> bool {
        std::mem::take(&mut self.bell)
    }

    fn scroll(&mut self, scroll: Scroll) {
        let before = self.display_offset();
        self.term.scroll_display(scroll);
        self.dirty |= self.display_offset() != before;
        self.handle_events();
    }

    fn line(&self, index: usize) -> Line {
        Line(index as i32 - self.term.history_size() as i32)
    }

    fn line_text(&self, index: usize) -> String {
        let row = &self.term.grid()[self.line(index)];
        let mut text = String::new();
        for cell in &row[..] {
            if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            text.push(printable(cell.c));
            text.extend(cell.zerowidth().into_iter().flatten());
        }
        text.truncate(text.trim_end().len());
        text
    }

    fn handle_events(&mut self) {
        let events = std::mem::take(&mut *self.events.0.borrow_mut());
        for event in events {
            match event {
                Event::PtyWrite(text) => self.output.extend_from_slice(text.as_bytes()),
                Event::ColorRequest(index, format) => {
                    let c = self.palette.get(self.term.colors(), index);
                    let reply = format(AnsiRgb { r: c.r, g: c.g, b: c.b });
                    self.output.extend_from_slice(reply.as_bytes());
                },
                Event::TextAreaSizeRequest(format) => {
                    // Pixel sizes are unknown to the core; report cells only.
                    let reply = format(WindowSize {
                        num_lines: self.rows(),
                        num_cols: self.cols(),
                        cell_width: 0,
                        cell_height: 0,
                    });
                    self.output.extend_from_slice(reply.as_bytes());
                },
                Event::Title(title) => self.title = Some(title),
                Event::ResetTitle => self.title = None,
                Event::Bell => self.bell = true,
                _ => {},
            }
        }
    }
}

/// The grid stores `'\t'` in the first cell of a tab stop; it occupies one blank cell.
fn printable(c: char) -> char {
    if c == '\t' { ' ' } else { c }
}

#[cfg(test)]
mod tests;
