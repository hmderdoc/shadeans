//! The IBM VGA 8x16 ROM font. Glyph shapes and coverages are read from the
//! real bitmaps rather than assumed (e.g. the lower half block is 9 rows, not 8).

pub const CELL_W: usize = 8;
pub const CELL_H: usize = 16;
pub const CELL_PIXELS: usize = CELL_W * CELL_H;

static FONT: &[u8; 4096] = include_bytes!("../assets/ibmstd.f16");

pub const SPACE: u8 = 0x20;
pub const SHADE_LIGHT: u8 = 0xB0;
pub const SHADE_MEDIUM: u8 = 0xB1;
pub const SHADE_DARK: u8 = 0xB2;
pub const FULL_BLOCK: u8 = 0xDB;
pub const HALF_LOWER: u8 = 0xDC;
pub const HALF_LEFT: u8 = 0xDD;
pub const HALF_RIGHT: u8 = 0xDE;
pub const HALF_UPPER: u8 = 0xDF;

pub fn glyph_row(ch: u8, y: usize) -> u8 {
    FONT[ch as usize * CELL_H + y]
}

pub fn pixel_is_fg(ch: u8, x: usize, y: usize) -> bool {
    (glyph_row(ch, y) >> (7 - x)) & 1 != 0
}

/// Fraction of the cell's pixels drawn in the foreground colour.
pub fn coverage(ch: u8) -> f32 {
    let bits: u32 = (0..CELL_H).map(|y| glyph_row(ch, y).count_ones()).sum();
    bits as f32 / CELL_PIXELS as f32
}
