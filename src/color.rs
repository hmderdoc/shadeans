//! Colour math: sRGB <-> linear light <-> Oklab, and the VGA text palette.
//!
//! Two spaces matter here. Shade glyphs mix fg and bg *physically*, so mixes
//! are computed in linear light. Errors are judged *perceptually*, so they are
//! measured in Oklab.

use std::sync::OnceLock;

pub type V3 = [f32; 3];

/// VGA text-mode palette in DOS attribute order (0 = black, 1 = blue, ... 15 = white).
pub const VGA: [[u8; 3]; 16] = [
    [0, 0, 0],
    [0, 0, 170],
    [0, 170, 0],
    [0, 170, 170],
    [170, 0, 0],
    [170, 0, 170],
    [170, 85, 0],
    [170, 170, 170],
    [85, 85, 85],
    [85, 85, 255],
    [85, 255, 85],
    [85, 255, 255],
    [255, 85, 85],
    [255, 85, 255],
    [255, 255, 85],
    [255, 255, 255],
];

pub fn srgb_to_linear(v: u8) -> f32 {
    static LUT: OnceLock<[f32; 256]> = OnceLock::new();
    LUT.get_or_init(|| {
        let mut lut = [0.0f32; 256];
        for (i, out) in lut.iter_mut().enumerate() {
            let c = i as f32 / 255.0;
            *out = if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            };
        }
        lut
    })[v as usize]
}

pub fn linear_to_srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0 + 0.5) as u8
}

pub fn linear_to_oklab(c: V3) -> V3 {
    let [r, g, b] = [c[0].max(0.0), c[1].max(0.0), c[2].max(0.0)];
    let l = (0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

pub fn oklab_to_linear(c: V3) -> V3 {
    let l = c[0] + 0.396_337_78 * c[1] + 0.215_803_76 * c[2];
    let m = c[0] - 0.105_561_346 * c[1] - 0.063_854_17 * c[2];
    let s = c[0] - 0.089_484_18 * c[1] - 1.291_485_5 * c[2];
    let (l, m, s) = (l * l * l, m * m * m, s * s * s);
    [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086_4 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ]
}

pub fn dist2(a: V3, b: V3) -> f32 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
}

pub fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The 16 colours plus everything derived from them that the matcher needs.
pub struct Palette {
    pub lin: [V3; 16],
    pub lab: [V3; 16],
    /// Squared Oklab distance between two palette colours: how loud a dither
    /// pattern made of that pair is.
    pub pair_d2: [[f32; 16]; 16],
}

impl Palette {
    pub fn vga() -> Self {
        let mut lin = [[0.0f32; 3]; 16];
        let mut lab = [[0.0f32; 3]; 16];
        for i in 0..16 {
            lin[i] = [
                srgb_to_linear(VGA[i][0]),
                srgb_to_linear(VGA[i][1]),
                srgb_to_linear(VGA[i][2]),
            ];
            lab[i] = linear_to_oklab(lin[i]);
        }
        let mut pair_d2 = [[0.0f32; 16]; 16];
        for f in 0..16 {
            for b in 0..16 {
                pair_d2[f][b] = dist2(lab[f], lab[b]);
            }
        }
        Palette { lin, lab, pair_d2 }
    }

    /// Oklab colour the eye sees when `coverage` of the cell is fg and the rest
    /// is bg. Mixed in linear light.
    pub fn mix_lab(&self, fg: usize, bg: usize, coverage: f32) -> V3 {
        let (f, b) = (self.lin[fg], self.lin[bg]);
        linear_to_oklab([
            f[0] * coverage + b[0] * (1.0 - coverage),
            f[1] * coverage + b[1] * (1.0 - coverage),
            f[2] * coverage + b[2] * (1.0 - coverage),
        ])
    }
}
