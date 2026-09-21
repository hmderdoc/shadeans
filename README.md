# shadeans

Image to ANSI converter that shades like an ANSI artist instead of drawing
pixel art.

Most converters treat each text cell as two stacked half-block "pixels", which
gives 16 flat colours and no shading. shadeans also uses the CP437 shade
characters `░▒▓`, which blend a foreground and background colour at 25%, 50%
and 75%. That turns the 16-colour palette into a few hundred usable tones, and
half blocks are still used where the picture has a real edge.

Output is CP437 `.ANS` with a SAUCE record, in 16 colours or, with `-t`, in
24-bit colour.

## Build

Needs a Rust toolchain (built with 1.95).

```
cargo build --release
```

The binary is `target/release/shadeans`.

## Use

```
shadeans INPUT_IMAGE [OUTPUT.ans] [options]
```

```
shadeans ~/Pictures/cat.jpg ~/Desktop/cat.ans
```

`INPUT_IMAGE` (jpg, png, gif, bmp, webp) is only ever read. If `OUTPUT.ans` is
left off, the file is written next to the input (`cat.jpg` -> `cat.ans`). The
full path of every file written is printed.

| option | default | what it does |
| --- | --- | --- |
| `-c, --cols <n>` | 80 | width in columns (`--columns` also works) |
| `--rows <n>` | from aspect ratio | height in rows |
| `-t, --truecolor` | off | 24-bit colour, see below |
| `--ice` | off | iCE colours: 16 background colours instead of 8, no blink |
| `--lambda <f>` | 0.10 | how visible dither texture is. 1 = pixel art; lower = more and bolder shading |
| `--coherence <f>` | 0.002 | pulls neighbouring cells onto shared colours. 0 = off, 0.006 = flat |
| `--sweeps <n>` | 4 | maximum coherence passes |
| `--blocks` | off | pixel-art baseline: no shade characters |
| `--contrast <f>` | 1.0 | lightness contrast of the source |
| `--saturation <f>` | 1.0 | colour strength of the source |
| `--smooth <n>` | 0 | edge-preserving smoothing passes on the source |
| `--no-levels` | | don't stretch the source to the full black-to-white range |
| `--no-sauce` | | leave off the SAUCE record |
| `--title/--author/--group <text>` | | SAUCE fields (title defaults to the image name) |
| `--force-newlines` | | CRLF after every row, even full 80-column rows |

Full-width 80-column rows are written without a newline, as ANSI viewers wrap
them on their own. Rows that end in black are trimmed and end with CRLF.

### Truecolor

`-t` writes 24-bit colour the same way [gif2ans](https://github.com/andyherbert/gif2ans)
does: every colour change is a complete 16-colour code followed by
`ESC[0;R;G;Bt` (background) and `ESC[1;R;G;Bt` (foreground). Viewers that read
those sequences (SyncTERM, PabloDraw, ansilove, Moebius) show the exact
colours; anything else shows the nearest 16-colour version of the same
characters. These are not the `38;2` codes of xterm-style terminals.

With exact colours there is nothing to shade: any blend of two colours is
available as a solid, without the dither pattern. So each cell is either a
solid in its mean colour, or a half block in the mean colours of its two halves
where that removes enough error to be a visible edge. `--lambda`, `--coherence`
and `--blocks` have no effect. Files are much larger (around 100-170 KB for an
80-column picture) because nearly every cell has its own colours.

### Coming from gif2ans

The command line is the same shape, `shadeans INPUT OUTPUT`, and `-c`,
`--columns`, `-t` and `-i` (writes a picture of the result to `OUTPUT.ans.png`)
mean the same thing. `-r`/`--restrict` is accepted and ignored, because
shadeans only ever uses the shade, half-block and full-block characters.
`-v`/`--vga50` (8x8 font) is not supported.

`--preview <file.png>` and `--src-png <file.png>` are debugging aids: a picture
of the finished ANSI drawn with the VGA font, and the prepared source the
matcher saw.

## How it works

The source is resized in linear light to exactly 8x16 pixels per cell and
converted to Oklab, where distances match what the eye sees.

Each cell is then scored in closed form. A few sums over the cell's pixels are
computed once, after which any (character, foreground, background) candidate
costs O(1) instead of a 128-pixel render. Matching an 80-column image takes a
few milliseconds.

* **Shades and solids.** The eye fuses a shade character into one mixed colour,
  so the cost is the distance from the cell's pixels to that mix (mixed in
  linear light, measured in Oklab), plus a texture cost
  `lambda * a(1-a) * |fg-bg|^2`. The term `a(1-a)|fg-bg|^2` is the variance of a
  two-colour pattern with coverage `a`, so `lambda` is the share of the dither
  pattern the eye still notices. It favours shading between close colours, the
  way ramps are drawn by hand, over loud pairs such as blue on red.
* **Half blocks.** Scored against the real font shapes (`▄` is 9 of 16 rows,
  not 8). Foreground and background separate, so the best of each is found
  independently.

A coherence pass then re-chooses each cell with a penalty for using colours its
four neighbours don't, weighted by how similar the source is across that
border. Regions settle on one ramp instead of changing colour pair from cell to
cell, while real edges are left alone.

## Font

`assets/ibmstd.f16` is the standard IBM VGA 8x16 ROM font, in the raw 4096-byte
layout used by ANSI tools. Character shapes and shade coverages are read from
it rather than assumed.
