//! Snapshot of the visible terminal viewport.
//!
//! This is the only contract between the terminal core (which fills a
//! [`Frame`]) and the renderer (which draws one). The core resolves every
//! colour decision — palette lookup, inverse video, dim, selection — so the
//! renderer only ever sees final RGB values.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

/// Bit flags for [`Cell::flags`].
pub mod flags {
    pub const BOLD: u8 = 1 << 0;
    pub const ITALIC: u8 = 1 << 1;
    pub const UNDERLINE: u8 = 1 << 2;
    pub const STRIKEOUT: u8 = 1 << 3;
    /// First column of a double-width character; the glyph spans two cells.
    pub const WIDE: u8 = 1 << 4;
    /// Second column of a double-width character; draw background only.
    pub const WIDE_SPACER: u8 = 1 << 5;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    /// `' '` for an empty cell.
    pub ch: char,
    pub fg: Rgb,
    pub bg: Rgb,
    pub flags: u8,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: Rgb::new(0xd8, 0xdc, 0xe4),
            bg: Rgb::new(0x10, 0x12, 0x16),
            flags: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorShape {
    #[default]
    Block,
    Underline,
    Beam,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub col: u16,
    pub row: u16,
    pub shape: CursorShape,
    pub color: Rgb,
}

#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub cols: u16,
    pub rows: u16,
    /// Row-major, exactly `cols * rows` entries.
    pub cells: Vec<Cell>,
    /// `None` while the cursor is hidden or scrolled out of view.
    pub cursor: Option<Cursor>,
    /// Fill colour for the padding around the grid.
    pub background: Rgb,
}

impl Frame {
    pub fn new(cols: u16, rows: u16) -> Self {
        let mut frame = Self::default();
        frame.background = Cell::default().bg;
        frame.resize(cols, rows);
        frame
    }

    /// Resizes the grid and resets every cell to the default.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
        self.cells.clear();
        self.cells
            .resize(cols as usize * rows as usize, Cell::default());
    }

    pub fn cell(&self, col: u16, row: u16) -> &Cell {
        &self.cells[row as usize * self.cols as usize + col as usize]
    }

    pub fn cell_mut(&mut self, col: u16, row: u16) -> &mut Cell {
        &mut self.cells[row as usize * self.cols as usize + col as usize]
    }
}
