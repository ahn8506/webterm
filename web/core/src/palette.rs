//! Default colour palette and colour resolution to final RGB.

use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::{COUNT, Colors};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb as AnsiRgb};
use webterm_frame::{Cell, Rgb};

/// Brightness factor applied to dim (SGR 2) text.
const DIM_FACTOR: f32 = 0.66;

const ANSI: [Rgb; 16] = [
    Rgb::new(0x1d, 0x1f, 0x26),
    Rgb::new(0xe0, 0x6c, 0x75),
    Rgb::new(0x98, 0xc3, 0x79),
    Rgb::new(0xe5, 0xc0, 0x7b),
    Rgb::new(0x61, 0xaf, 0xef),
    Rgb::new(0xc6, 0x78, 0xdd),
    Rgb::new(0x56, 0xb6, 0xc2),
    Rgb::new(0xab, 0xb2, 0xbf),
    Rgb::new(0x5c, 0x63, 0x70),
    Rgb::new(0xf0, 0x8a, 0x92),
    Rgb::new(0xb1, 0xd9, 0x94),
    Rgb::new(0xf2, 0xd3, 0x96),
    Rgb::new(0x82, 0xc2, 0xf7),
    Rgb::new(0xd8, 0x96, 0xea),
    Rgb::new(0x75, 0xcd, 0xd8),
    Rgb::new(0xf2, 0xf4, 0xf8),
];

fn dim(c: Rgb) -> Rgb {
    let scale = |v: u8| (v as f32 * DIM_FACTOR) as u8;
    Rgb::new(scale(c.r), scale(c.g), scale(c.b))
}

fn from_ansi(c: AnsiRgb) -> Rgb {
    Rgb::new(c.r, c.g, c.b)
}

/// Default colours, indexed like [`Colors`] (16 ANSI, 216 cube, 24 greys,
/// then the named extras).
pub(crate) struct Palette([Rgb; COUNT]);

impl Palette {
    pub(crate) fn new() -> Self {
        let mut p = [Rgb::default(); COUNT];
        p[..16].copy_from_slice(&ANSI);

        // 6x6x6 colour cube followed by the greyscale ramp, as in xterm.
        let level = |i: usize| if i == 0 { 0 } else { (55 + 40 * i) as u8 };
        for i in 0..216 {
            p[16 + i] = Rgb::new(level(i / 36), level(i / 6 % 6), level(i % 6));
        }
        for i in 0..24 {
            let v = (8 + 10 * i) as u8;
            p[232 + i] = Rgb::new(v, v, v);
        }

        let default = Cell::default();
        p[NamedColor::Foreground as usize] = default.fg;
        p[NamedColor::Background as usize] = default.bg;
        p[NamedColor::Cursor as usize] = default.fg;
        p[NamedColor::BrightForeground as usize] = ANSI[15];
        p[NamedColor::DimForeground as usize] = dim(default.fg);
        for i in 0..8 {
            p[NamedColor::DimBlack as usize + i] = dim(ANSI[i]);
        }
        Self(p)
    }

    /// Colour at `index`, honouring overrides set by the device (OSC 4/10/11/12).
    pub(crate) fn get(&self, overrides: &Colors, index: usize) -> Rgb {
        overrides[index].map_or(self.0[index], from_ansi)
    }

    fn resolve(&self, overrides: &Colors, color: Color, dimmed: bool) -> Rgb {
        match color {
            Color::Spec(rgb) if dimmed => dim(from_ansi(rgb)),
            Color::Spec(rgb) => from_ansi(rgb),
            Color::Named(named) if dimmed => self.get(overrides, named.to_dim() as usize),
            Color::Named(named) => self.get(overrides, named as usize),
            // The first eight indexed colours have dedicated dim variants.
            Color::Indexed(i) if dimmed && i < 8 => {
                self.get(overrides, NamedColor::DimBlack as usize + i as usize)
            },
            Color::Indexed(i) if dimmed => dim(self.get(overrides, i as usize)),
            Color::Indexed(i) => self.get(overrides, i as usize),
        }
    }

    /// Final `(fg, bg)` of a cell after dim, inverse and hidden are applied.
    pub(crate) fn cell_colors(
        &self,
        overrides: &Colors,
        fg: Color,
        bg: Color,
        flags: Flags,
    ) -> (Rgb, Rgb) {
        let mut fg = self.resolve(overrides, fg, flags.contains(Flags::DIM));
        let mut bg = self.resolve(overrides, bg, false);
        if flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if flags.contains(Flags::HIDDEN) {
            fg = bg;
        }
        (fg, bg)
    }
}
