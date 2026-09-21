//! Loads the input image and turns it into an Oklab pixel grid at exactly
//! 8x16 pixels per output cell, with the tonal prep that makes ramps band
//! cleanly (levels, contrast, saturation, edge-preserving smoothing).

use crate::color::{linear_to_oklab, linear_to_srgb, oklab_to_linear, srgb_to_linear, V3};
use crate::font::{CELL_H, CELL_W};
use image::{imageops::FilterType, ImageBuffer, Rgb};

pub struct Prep {
    pub auto_levels: bool,
    /// Chroma the image's most colourful pixels are lifted to (0 = off).
    pub auto_chroma: f32,
    /// Blend toward a flat lightness histogram, 0..1.
    pub equalize: f32,
    /// Strength of the wide-radius lightness boost that separates a shape
    /// from its surroundings.
    pub local_contrast: f32,
    pub contrast: f32,
    pub saturation: f32,
    /// Bilateral filter passes (0 = off).
    pub smooth: u32,
}

pub struct Source {
    pub width: usize,
    pub height: usize,
    pub lab: Vec<V3>,
}

/// Rows that keep the image's aspect ratio for `cols` columns of 8x16 square-pixel cells.
pub fn rows_for_aspect(img_w: u32, img_h: u32, cols: usize) -> usize {
    let px_w = (cols * CELL_W) as f32;
    let px_h = px_w * img_h as f32 / img_w as f32;
    ((px_h / CELL_H as f32).round() as usize).max(1)
}

pub fn load(path: &str) -> Result<image::RgbaImage, String> {
    image::open(path)
        .map(|img| img.to_rgba8())
        .map_err(|e| format!("cannot read {path}: {e}"))
}

pub fn prepare(rgba: &image::RgbaImage, cols: usize, rows: usize, prep: &Prep) -> Source {
    let (w, h) = ((cols * CELL_W) as u32, (rows * CELL_H) as u32);

    // Resize in linear light, composited over black.
    let mut linear: ImageBuffer<Rgb<f32>, Vec<f32>> = ImageBuffer::new(rgba.width(), rgba.height());
    for (dst, src) in linear.pixels_mut().zip(rgba.pixels()) {
        let a = src[3] as f32 / 255.0;
        dst.0 = [
            srgb_to_linear(src[0]) * a,
            srgb_to_linear(src[1]) * a,
            srgb_to_linear(src[2]) * a,
        ];
    }
    let resized = image::imageops::resize(&linear, w, h, FilterType::CatmullRom);

    let mut lab: Vec<V3> = resized.pixels().map(|p| linear_to_oklab(p.0)).collect();

    let (width, height) = (w as usize, h as usize);
    if prep.auto_levels {
        auto_levels(&mut lab);
    }
    if prep.equalize > 0.0 {
        equalize(&mut lab, prep.equalize);
    }
    if prep.local_contrast > 0.0 {
        local_contrast(&mut lab, width, height, prep.local_contrast);
    }
    if prep.auto_chroma > 0.0 {
        auto_chroma(&mut lab, prep.auto_chroma);
    }
    for p in lab.iter_mut() {
        p[0] = 0.5 + (p[0] - 0.5) * prep.contrast;
        p[1] *= prep.saturation;
        p[2] *= prep.saturation;
    }
    for _ in 0..prep.smooth {
        lab = bilateral(&lab, width, height);
    }

    Source { width, height, lab }
}

/// Stretch lightness so the 0.5th..99.5th percentiles span black..white.
fn auto_levels(lab: &mut [V3]) {
    let mut ls: Vec<f32> = lab.iter().map(|p| p[0]).collect();
    ls.sort_by(|a, b| a.total_cmp(b));
    let lo = ls[ls.len() / 200];
    let hi = ls[ls.len() - 1 - ls.len() / 200];
    if hi - lo < 0.05 {
        return;
    }
    let scale = 1.0 / (hi - lo);
    for p in lab.iter_mut() {
        p[0] = (p[0] - lo) * scale;
    }
}

/// The VGA palette has no muted colours: everything that isn't a grey is
/// vivid. A muted green is therefore nearer to grey than to green, and whole
/// regions lose their hue. Like auto-levels does for lightness, lift chroma so
/// the image's most colourful pixels (95th percentile) reach `target`. Only
/// ever boosts, and leaves near-greyscale images alone.
///
/// Near-neutral pixels are left out of the boost: a white wall with a faint
/// cast should stay white, not turn blue.
fn auto_chroma(lab: &mut [V3], target: f32) {
    const MAX_BOOST: f32 = 1.8;
    const NEUTRAL_BELOW: f32 = 0.02;
    const FULL_BOOST_ABOVE: f32 = 0.06;
    let mut chroma: Vec<f32> = lab.iter().map(|p| p[1].hypot(p[2])).collect();
    chroma.sort_by(|a, b| a.total_cmp(b));
    let p95 = chroma[chroma.len() * 95 / 100];
    if p95 < NEUTRAL_BELOW || p95 >= target {
        return;
    }
    let boost = (target / p95).min(MAX_BOOST);
    for p in lab.iter_mut() {
        let c = p[1].hypot(p[2]);
        let t = ((c - NEUTRAL_BELOW) / (FULL_BOOST_ABOVE - NEUTRAL_BELOW)).clamp(0.0, 1.0);
        let gain = 1.0 + (boost - 1.0) * t * t * (3.0 - 2.0 * t);
        p[1] *= gain;
        p[2] *= gain;
    }
}

/// Partial, contrast-limited histogram equalisation of lightness. With only
/// four greys to land on, an image whose tones bunch together collapses into
/// one of them; this spreads the tones out so neighbouring regions reach
/// different palette steps.
///
/// The histogram is clipped before it is integrated (as in CLAHE), which caps
/// how steep the tone curve can get. Without that, a picture that is mostly
/// bright background spends the whole range on the background and drags every
/// midtone, faces included, down into the darks.
fn equalize(lab: &mut [V3], amount: f32) {
    const BINS: usize = 256;
    const CLIP: f32 = 2.0;
    let bin = |l: f32| ((l.clamp(0.0, 1.0) * (BINS - 1) as f32) as usize).min(BINS - 1);
    let mut hist = [0.0f32; BINS];
    for p in lab.iter() {
        hist[bin(p[0])] += 1.0;
    }
    let limit = CLIP * lab.len() as f32 / BINS as f32;
    let mut excess = 0.0;
    for h in hist.iter_mut() {
        if *h > limit {
            excess += *h - limit;
            *h = limit;
        }
    }
    let share = excess / BINS as f32;
    let mut total = 0.0;
    for h in hist.iter_mut() {
        total += *h + share;
        *h = total;
    }
    for p in lab.iter_mut() {
        let flat = hist[bin(p[0])] / total;
        p[0] += (flat - p[0]) * amount;
    }
}

/// Unsharp mask on lightness with a radius of a few cells: pushes a shape away
/// from whatever surrounds it, the way a hand-drawn piece darkens a figure
/// against a light ground.
fn local_contrast(lab: &mut [V3], w: usize, h: usize, amount: f32) {
    let sigma = 2.5 * CELL_W as f32;
    let radius = (sigma * 2.5) as isize;
    let kernel: Vec<f32> = (-radius..=radius)
        .map(|d| (-((d * d) as f32) / (2.0 * sigma * sigma)).exp())
        .collect();
    let norm: f32 = kernel.iter().sum();

    let blur_axis = |src: &[f32], horizontal: bool| -> Vec<f32> {
        let mut out = vec![0.0f32; src.len()];
        for y in 0..h as isize {
            for x in 0..w as isize {
                let mut acc = 0.0;
                for (k, wgt) in kernel.iter().enumerate() {
                    let d = k as isize - radius;
                    let (xx, yy) = if horizontal {
                        ((x + d).clamp(0, w as isize - 1), y)
                    } else {
                        (x, (y + d).clamp(0, h as isize - 1))
                    };
                    acc += src[yy as usize * w + xx as usize] * wgt;
                }
                out[y as usize * w + x as usize] = acc / norm;
            }
        }
        out
    };
    let light: Vec<f32> = lab.iter().map(|p| p[0]).collect();
    let blurred = blur_axis(&blur_axis(&light, true), false);
    for (p, b) in lab.iter_mut().zip(blurred) {
        p[0] = (p[0] + (p[0] - b) * amount).clamp(0.0, 1.0);
    }
}

/// Edge-preserving smoothing: flattens regions so shade ramps come out as
/// clean bands, without softening the contours between regions.
fn bilateral(lab: &[V3], w: usize, h: usize) -> Vec<V3> {
    const RADIUS: isize = 4;
    const SIGMA_SPACE: f32 = 3.0;
    const SIGMA_RANGE: f32 = 0.07;
    let mut spatial = [[0.0f32; (2 * RADIUS + 1) as usize]; (2 * RADIUS + 1) as usize];
    for dy in -RADIUS..=RADIUS {
        for dx in -RADIUS..=RADIUS {
            spatial[(dy + RADIUS) as usize][(dx + RADIUS) as usize] =
                (-((dx * dx + dy * dy) as f32) / (2.0 * SIGMA_SPACE * SIGMA_SPACE)).exp();
        }
    }
    let inv_range = 1.0 / (2.0 * SIGMA_RANGE * SIGMA_RANGE);
    let mut out = vec![[0.0f32; 3]; lab.len()];
    for y in 0..h as isize {
        for x in 0..w as isize {
            let c = lab[y as usize * w + x as usize];
            let mut acc = [0.0f32; 3];
            let mut wsum = 0.0f32;
            for dy in -RADIUS..=RADIUS {
                let yy = y + dy;
                if yy < 0 || yy >= h as isize {
                    continue;
                }
                for dx in -RADIUS..=RADIUS {
                    let xx = x + dx;
                    if xx < 0 || xx >= w as isize {
                        continue;
                    }
                    let p = lab[yy as usize * w + xx as usize];
                    let d = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
                    let range = (-(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) * inv_range).exp();
                    let wgt = spatial[(dy + RADIUS) as usize][(dx + RADIUS) as usize] * range;
                    acc[0] += p[0] * wgt;
                    acc[1] += p[1] * wgt;
                    acc[2] += p[2] * wgt;
                    wsum += wgt;
                }
            }
            out[y as usize * w + x as usize] = [acc[0] / wsum, acc[1] / wsum, acc[2] / wsum];
        }
    }
    out
}

impl Source {
    /// The prepared source as an sRGB image, for eyeballing against the output.
    pub fn to_rgb_image(&self) -> image::RgbImage {
        let mut img = image::RgbImage::new(self.width as u32, self.height as u32);
        for (dst, src) in img.pixels_mut().zip(self.lab.iter()) {
            let lin = oklab_to_linear(*src);
            dst.0 = [
                linear_to_srgb(lin[0]),
                linear_to_srgb(lin[1]),
                linear_to_srgb(lin[2]),
            ];
        }
        img
    }
}
