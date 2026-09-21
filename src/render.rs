//! Renders a cell grid to pixels with the VGA font, for previews.

use crate::color::VGA;
use crate::convert::Cell;
use crate::font::{self, CELL_H, CELL_W};

pub fn to_rgb_image(cells: &[Cell], cols: usize, rows: usize) -> image::RgbImage {
    let mut img = image::RgbImage::new((cols * CELL_W) as u32, (rows * CELL_H) as u32);
    for (i, cell) in cells.iter().enumerate() {
        let (cx, cy) = (i % cols, i / cols);
        let (fg, bg) = cell
            .rgb
            .unwrap_or((VGA[cell.fg as usize], VGA[cell.bg as usize]));
        for y in 0..CELL_H {
            for x in 0..CELL_W {
                let colour = if font::pixel_is_fg(cell.ch, x, y) { fg } else { bg };
                img.put_pixel(
                    (cx * CELL_W + x) as u32,
                    (cy * CELL_H + y) as u32,
                    image::Rgb(colour),
                );
            }
        }
    }
    img
}
