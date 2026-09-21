//! The matcher.
//!
//! Every cell is scored in closed form from a handful of precomputed sums, so
//! a candidate (glyph, fg, bg) costs O(1) instead of a 128-pixel render.
//!
//! Two kinds of candidate:
//!
//! * **Uniform** (space, full block, and the three shades). The eye fuses a
//!   shade into one mixed colour M, so the cost is the distance from every
//!   source pixel to M, plus a *texture* cost `lambda * a(1-a) * |fg-bg|^2`.
//!   `a(1-a)|fg-bg|^2` is exactly the variance of a two-colour pattern with
//!   coverage `a`, so lambda is the fraction of that pattern the eye still
//!   sees: 1.0 = raw pixel matching (pixel art), 0.0 = perfect mixing.
//!   Low lambda lets shades through, but only between close-valued colours,
//!   which is how ramps are drawn by hand.
//!
//! * **Structural** (the four half blocks). Real geometry: pixels under the
//!   glyph mask are compared to fg, the rest to bg. fg and bg separate, so the
//!   best of each is found independently.
//!
//! An optional coherence pass then re-chooses each cell with a penalty for
//! introducing colours its similar-looking neighbours don't use, so a region
//! settles on one ramp instead of flipping pairs cell to cell.
//!
//! **Truecolor** is a different problem. Any mix of two colours is available
//! directly as a solid, without the dither texture, so shades can never win
//! and the palette search disappears. For a given glyph the best fg and bg are
//! simply the mean colours under and outside its mask, and the cost is the
//! variance left inside those two regions. A cell is a solid in its exact mean
//! colour unless a half block removes enough error to be a visible edge.

use crate::color::{dist2, dot, linear_to_srgb, oklab_to_linear, Palette, V3};
use crate::font::{self, CELL_H, CELL_PIXELS, CELL_W};
use crate::source::Source;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell {
    pub ch: u8,
    /// VGA palette indices. In truecolor mode these are the 16-colour fallback
    /// for viewers that ignore 24-bit sequences.
    pub fg: u8,
    pub bg: u8,
    /// Exact (fg, bg) sRGB colours, truecolor mode only.
    pub rgb: Option<([u8; 3], [u8; 3])>,
}

impl Cell {
    /// Bitmask of the palette colours actually visible in this cell.
    fn colour_set(self) -> u16 {
        match self.ch {
            font::SPACE => 1 << self.bg,
            font::FULL_BLOCK => 1 << self.fg,
            _ => (1 << self.fg) | (1 << self.bg),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GlyphSet {
    /// Shades and half blocks: the point of this program.
    Shaded,
    /// Solids and half blocks only: the pixel-art baseline, for comparison.
    Blocks,
}

pub struct Options {
    pub lambda: f32,
    pub ice: bool,
    pub glyphs: GlyphSet,
    /// Strength of the neighbour colour-coherence penalty (0 = single pass).
    pub coherence: f32,
    pub sweeps: u32,
    /// 24-bit colours per cell instead of matching the 16-colour palette.
    pub truecolor: bool,
}

const STRUCTURAL: [u8; 4] = [
    font::HALF_UPPER,
    font::HALF_LOWER,
    font::HALF_LEFT,
    font::HALF_RIGHT,
];
const SHADES: [u8; 3] = [font::SHADE_LIGHT, font::SHADE_MEDIUM, font::SHADE_DARK];

/// Sufficient statistics of a set of source pixels for squared-error costs.
#[derive(Clone, Copy, Default)]
struct Region {
    n: f32,
    sum: V3,
    sum_sq: f32,
}

impl Region {
    fn add(&mut self, p: V3) {
        self.n += 1.0;
        self.sum[0] += p[0];
        self.sum[1] += p[1];
        self.sum[2] += p[2];
        self.sum_sq += dot(p, p);
    }

    fn minus(self, other: Region) -> Region {
        Region {
            n: self.n - other.n,
            sum: [
                self.sum[0] - other.sum[0],
                self.sum[1] - other.sum[1],
                self.sum[2] - other.sum[2],
            ],
            sum_sq: self.sum_sq - other.sum_sq,
        }
    }

    /// Sum over the region's pixels of |pixel - colour|^2.
    fn cost(&self, colour: V3) -> f32 {
        self.sum_sq - 2.0 * dot(colour, self.sum) + self.n * dot(colour, colour)
    }

    fn mean(&self) -> V3 {
        [self.sum[0] / self.n, self.sum[1] / self.n, self.sum[2] / self.n]
    }
}

struct CellStats {
    all: Region,
    /// Pixels under each structural glyph's mask, in `STRUCTURAL` order.
    under: [Region; 4],
}

fn cell_stats(src: &Source, cx: usize, cy: usize) -> CellStats {
    let mut stats = CellStats {
        all: Region::default(),
        under: [Region::default(); 4],
    };
    for y in 0..CELL_H {
        let row = (cy * CELL_H + y) * src.width + cx * CELL_W;
        for x in 0..CELL_W {
            let p = src.lab[row + x];
            stats.all.add(p);
            for (g, &ch) in STRUCTURAL.iter().enumerate() {
                if font::pixel_is_fg(ch, x, y) {
                    stats.under[g].add(p);
                }
            }
        }
    }
    stats
}

/// Shade mixes are fixed per palette, so build them once.
struct ShadeTable {
    /// `[level][fg][bg]` fused Oklab colour.
    mix: Vec<[[V3; 16]; 16]>,
    /// `coverage * (1 - coverage)` per level.
    pattern_var: Vec<f32>,
}

impl ShadeTable {
    fn new(pal: &Palette) -> Self {
        let mut mix = Vec::new();
        let mut pattern_var = Vec::new();
        for &ch in &SHADES {
            let a = font::coverage(ch);
            let mut table = [[[0.0f32; 3]; 16]; 16];
            for (f, row) in table.iter_mut().enumerate() {
                for (b, out) in row.iter_mut().enumerate() {
                    *out = pal.mix_lab(f, b, a);
                }
            }
            mix.push(table);
            pattern_var.push(a * (1.0 - a));
        }
        ShadeTable { mix, pattern_var }
    }
}

fn solid_cell(colour: usize, ice: bool) -> Cell {
    // Dark colours are a space on that background. Bright ones need the full
    // block unless iCE colours make bright backgrounds available.
    if colour < 8 || ice {
        Cell { ch: font::SPACE, fg: 7, bg: colour as u8, rgb: None }
    } else {
        Cell { ch: font::FULL_BLOCK, fg: colour as u8, bg: 0, rgb: None }
    }
}

/// Best cell for these statistics. `penalty[k]` is an extra cost for using
/// palette colour `k` at all (zero everywhere for the plain single pass).
fn best_cell(
    stats: &CellStats,
    pal: &Palette,
    shades: &ShadeTable,
    opts: &Options,
    penalty: &[f32; 16],
) -> Cell {
    let bg_count = if opts.ice { 16 } else { 8 };
    let mut best = solid_cell(0, opts.ice);
    let mut best_cost = f32::INFINITY;

    for (k, (&lab, &extra)) in pal.lab.iter().zip(penalty).enumerate() {
        let cost = stats.all.cost(lab) + extra;
        if cost < best_cost {
            best_cost = cost;
            best = solid_cell(k, opts.ice);
        }
    }

    if opts.glyphs == GlyphSet::Shaded {
        for (level, &ch) in SHADES.iter().enumerate() {
            let texture_scale = opts.lambda * stats.all.n * shades.pattern_var[level];
            for f in 0..16 {
                for b in 0..bg_count {
                    if f == b {
                        continue;
                    }
                    let cost = stats.all.cost(shades.mix[level][f][b])
                        + texture_scale * pal.pair_d2[f][b]
                        + penalty[f]
                        + penalty[b];
                    if cost < best_cost {
                        best_cost = cost;
                        best = Cell { ch, fg: f as u8, bg: b as u8, rgb: None };
                    }
                }
            }
        }
    }

    for (g, &ch) in STRUCTURAL.iter().enumerate() {
        let under = stats.under[g];
        let over = stats.all.minus(under);
        let pick = |region: &Region, count: usize| {
            (0..count)
                .map(|k| (k, region.cost(pal.lab[k]) + penalty[k]))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap()
        };
        let (f, f_cost) = pick(&under, 16);
        let (b, b_cost) = pick(&over, bg_count);
        if f != b && f_cost + b_cost < best_cost {
            best_cost = f_cost + b_cost;
            best = Cell { ch, fg: f as u8, bg: b as u8, rgb: None };
        }
    }

    best
}

impl Region {
    /// Squared error left when the whole region is painted its own mean colour.
    fn variance_sum(&self) -> f32 {
        (self.sum_sq - dot(self.sum, self.sum) / self.n).max(0.0)
    }
}

fn oklab_to_srgb8(lab: V3) -> [u8; 3] {
    let lin = oklab_to_linear(lab);
    [linear_to_srgb(lin[0]), linear_to_srgb(lin[1]), linear_to_srgb(lin[2])]
}

fn nearest_vga(pal: &Palette, lab: V3, count: usize) -> u8 {
    (0..count)
        .min_by(|&a, &b| dist2(pal.lab[a], lab).total_cmp(&dist2(pal.lab[b], lab)))
        .unwrap() as u8
}

/// Error a half block must remove, per pixel, before it replaces a solid.
/// Two halves that differ by d in Oklab gain about d^2/4, so this is d ~ 0.03:
/// roughly where the step between them becomes visible.
const TRUECOLOR_EDGE_GAIN: f32 = 0.0002;

fn best_cell_truecolor(stats: &CellStats, pal: &Palette, opts: &Options) -> Cell {
    let bg_count = if opts.ice { 16 } else { 8 };
    let solid_cost = stats.all.variance_sum();

    let mut best: Option<(u8, Region, Region)> = None;
    let mut best_cost = solid_cost - TRUECOLOR_EDGE_GAIN * stats.all.n;
    for (g, &ch) in STRUCTURAL.iter().enumerate() {
        let under = stats.under[g];
        let over = stats.all.minus(under);
        let cost = under.variance_sum() + over.variance_sum();
        if cost < best_cost {
            best_cost = cost;
            best = Some((ch, under, over));
        }
    }

    match best {
        Some((ch, under, over)) => {
            let (f, b) = (under.mean(), over.mean());
            Cell {
                ch,
                fg: nearest_vga(pal, f, 16),
                bg: nearest_vga(pal, b, bg_count),
                rgb: Some((oklab_to_srgb8(f), oklab_to_srgb8(b))),
            }
        }
        None => {
            let mean = stats.all.mean();
            let colour = oklab_to_srgb8(mean);
            Cell {
                rgb: Some((colour, colour)),
                ..solid_cell(nearest_vga(pal, mean, 16) as usize, opts.ice)
            }
        }
    }
}

pub fn convert(src: &Source, cols: usize, rows: usize, pal: &Palette, opts: &Options) -> Vec<Cell> {
    let shades = ShadeTable::new(pal);
    let stats: Vec<CellStats> = (0..rows * cols)
        .map(|i| cell_stats(src, i % cols, i / cols))
        .collect();

    if opts.truecolor {
        return stats
            .iter()
            .map(|s| best_cell_truecolor(s, pal, opts))
            .collect();
    }

    let no_penalty = [0.0f32; 16];
    let mut cells: Vec<Cell> = stats
        .iter()
        .map(|s| best_cell(s, pal, &shades, opts, &no_penalty))
        .collect();

    if opts.coherence > 0.0 {
        refine_coherence(&mut cells, &stats, cols, rows, pal, &shades, opts);
    }
    cells
}

/// Iterated conditional modes over the grid: each cell is re-chosen with a
/// penalty for every colour that a similar-looking neighbour does not use.
fn refine_coherence(
    cells: &mut [Cell],
    stats: &[CellStats],
    cols: usize,
    rows: usize,
    pal: &Palette,
    shades: &ShadeTable,
    opts: &Options,
) {
    // Neighbours only pull on each other when their source colours are close;
    // across a real edge the weight falls to nothing.
    const SIMILARITY_SIGMA: f32 = 0.08;
    let means: Vec<V3> = stats.iter().map(|s| s.all.mean()).collect();
    let weight = |a: usize, b: usize| {
        (-dist2(means[a], means[b]) / (2.0 * SIMILARITY_SIGMA * SIMILARITY_SIGMA)).exp()
    };

    for _ in 0..opts.sweeps {
        let mut changed = 0usize;
        for cy in 0..rows {
            for cx in 0..cols {
                let i = cy * cols + cx;
                let mut neighbours = [usize::MAX; 4];
                if cx > 0 {
                    neighbours[0] = i - 1;
                }
                if cx + 1 < cols {
                    neighbours[1] = i + 1;
                }
                if cy > 0 {
                    neighbours[2] = i - cols;
                }
                if cy + 1 < rows {
                    neighbours[3] = i + cols;
                }

                let mut penalty = [0.0f32; 16];
                for &j in neighbours.iter().filter(|&&j| j != usize::MAX) {
                    let pull = opts.coherence * CELL_PIXELS as f32 * weight(i, j);
                    let set = cells[j].colour_set();
                    for (k, p) in penalty.iter_mut().enumerate() {
                        if set & (1 << k) == 0 {
                            *p += pull;
                        }
                    }
                }

                let chosen = best_cell(&stats[i], pal, shades, opts, &penalty);
                if chosen != cells[i] {
                    cells[i] = chosen;
                    changed += 1;
                }
            }
        }
        if changed == 0 {
            break;
        }
    }
}
