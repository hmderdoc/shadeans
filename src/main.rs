mod ansi;
mod color;
mod convert;
mod font;
mod render;
mod source;

use std::process::ExitCode;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const USAGE: &str = "\
shadeans - image to ANSI with CP437 shading

usage: shadeans INPUT_IMAGE [OUTPUT.ans] [options]

example:
  shadeans ~/Pictures/cat.jpg ~/Desktop/cat.ans

INPUT_IMAGE is the picture to convert (jpg, png, gif, bmp, webp). It is only
ever read. Everything else on the line is a file this program writes.

If OUTPUT.ans is left off, the .ans is written next to the input image
(~/Pictures/cat.jpg -> ~/Pictures/cat.ans). Every file written is printed
with its full path.

output
  -o, --out <file.ans>     same as giving OUTPUT.ans
      --no-sauce           omit the SAUCE record
      --title/--author/--group <text>   SAUCE fields
      --force-newlines     CRLF after every row. Full-width rows normally get
                           none, because viewers wrap at the width in the SAUCE
                           record. Use this to show art narrower than the
                           screen in a plain terminal, which knows no SAUCE.

size
  -c, --cols <n>           columns (default 80); --columns also works
      --rows <n>           rows (default: keep the image's aspect ratio)

colour
  -t, --truecolor          24-bit colour. Written the way gif2ans does it: a
                           16-colour code per cell as the fallback, then
                           ESC[0;R;G;Bt / ESC[1;R;G;Bt for viewers that read
                           them (SyncTERM, PabloDraw, ansilove, Moebius).
                           Shading is not needed with exact colours, so
                           --lambda, --coherence and --blocks have no effect.
      --ice                iCE colours: 16 background colours, no blink

look (16-colour mode)
      --lambda <f>         how visible dither texture is, 0..1 (default 0.10)
                           1 = pixel art, lower = more and bolder shading
      --blocks             pixel-art baseline: no shade glyphs
      --coherence <f>      pull neighbouring cells onto shared colours
                           (default 0.002, 0 = off, 0.006 = flat)
      --sweeps <n>         max coherence passes (default 4)

source prep
      --contrast <f>       lightness contrast (default 1.0)
      --saturation <f>     chroma multiplier (default 1.0)
      --smooth <n>         edge-preserving smoothing passes (default 0)
      --no-levels          don't auto-stretch lightness to the full range

gif2ans compatibility
  -i, --image              also write a picture of the result to OUTPUT.ans.png
  -r, --restrict           accepted and ignored: shadeans only ever uses the
                           shade, half-block and full-block characters

debugging (not needed for normal use)
      --preview <file.png> write a picture of the finished ANSI in the VGA font
      --src-png <file.png> write the prepared source the matcher saw
";

struct Args {
    input: String,
    out: Option<String>,
    png: Option<String>,
    image: bool,
    src_png: Option<String>,
    sauce: bool,
    title: Option<String>,
    author: String,
    group: String,
    force_newlines: bool,
    cols: usize,
    rows: Option<usize>,
    opts: convert::Options,
    prep: source::Prep,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        input: String::new(),
        out: None,
        png: None,
        image: false,
        src_png: None,
        sauce: true,
        title: None,
        author: String::new(),
        group: String::new(),
        force_newlines: false,
        cols: 80,
        rows: None,
        opts: convert::Options {
            lambda: 0.10,
            ice: false,
            glyphs: convert::GlyphSet::Shaded,
            coherence: 0.002,
            sweeps: 4,
            truecolor: false,
        },
        prep: source::Prep {
            auto_levels: true,
            contrast: 1.0,
            saturation: 1.0,
            smooth: 0,
        },
    };

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or(format!("{name} needs a value"));
        fn num<T: std::str::FromStr>(name: &str, text: String) -> Result<T, String> {
            text.parse().map_err(|_| format!("{name}: bad number '{text}'"))
        }
        match arg.as_str() {
            "-h" | "--help" => return Err(String::new()),
            "-o" | "--out" => args.out = Some(value("--out")?),
            "--preview" | "--png" => args.png = Some(value("--preview")?),
            "--src-png" => args.src_png = Some(value("--src-png")?),
            "--no-sauce" => args.sauce = false,
            "--title" => args.title = Some(value("--title")?),
            "--author" => args.author = value("--author")?,
            "--group" => args.group = value("--group")?,
            "--force-newlines" => args.force_newlines = true,
            "-c" | "--cols" | "--columns" => args.cols = num("--cols", value("--cols")?)?,
            "--rows" => args.rows = Some(num("--rows", value("--rows")?)?),
            "-t" | "--truecolor" => args.opts.truecolor = true,
            "-i" | "--image" => args.image = true,
            "-r" | "--restrict" => {}
            "--lambda" => args.opts.lambda = num("--lambda", value("--lambda")?)?,
            "--ice" => args.opts.ice = true,
            "--blocks" => args.opts.glyphs = convert::GlyphSet::Blocks,
            "--coherence" => args.opts.coherence = num("--coherence", value("--coherence")?)?,
            "--sweeps" => args.opts.sweeps = num("--sweeps", value("--sweeps")?)?,
            "--contrast" => args.prep.contrast = num("--contrast", value("--contrast")?)?,
            "--saturation" => args.prep.saturation = num("--saturation", value("--saturation")?)?,
            "--smooth" => args.prep.smooth = num("--smooth", value("--smooth")?)?,
            "--no-levels" => args.prep.auto_levels = false,
            other if other.starts_with('-') => return Err(format!("unknown option {other}")),
            _ if args.input.is_empty() => args.input = arg,
            _ if args.out.is_none() => args.out = Some(arg),
            _ => return Err(format!("unexpected argument {arg}")),
        }
    }
    if args.input.is_empty() {
        return Err(String::new());
    }
    if args.input.to_lowercase().ends_with(".ans") {
        return Err(format!(
            "the first argument is the image to convert, but '{}' looks like the ANSI output.\n\
             usage: shadeans INPUT_IMAGE [OUTPUT.ans]",
            args.input
        ));
    }
    // Never let an output land on top of the picture being converted.
    let same_file = |a: &str, b: &str| {
        let full = |p: &str| std::fs::canonicalize(p).unwrap_or_else(|_| p.into());
        full(a) == full(b)
    };
    for (flag, path) in [("OUTPUT.ans", &args.out), ("--preview", &args.png), ("--src-png", &args.src_png)] {
        if let Some(path) = path {
            if same_file(path, &args.input) {
                return Err(format!(
                    "{flag} is '{path}', which is the input image itself; refusing to overwrite it.\n\
                     Give {flag} a different file name."
                ));
            }
        }
    }
    if args.cols == 0 || args.cols > 1000 {
        return Err("--cols must be 1..1000".into());
    }
    Ok(args)
}

/// Today's UTC date as CCYYMMDD (civil-from-days, no date crate needed).
fn today_ccyymmdd() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}{month:02}{day:02}")
}

fn stem(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .map_or_else(|| "out".into(), |s| s.to_string_lossy().into_owned())
}

fn run(args: Args) -> Result<(), String> {
    let started = Instant::now();
    let rgba = source::load(&args.input)?;
    let cols = args.cols;
    let rows = args
        .rows
        .unwrap_or_else(|| source::rows_for_aspect(rgba.width(), rgba.height(), cols));

    let src = source::prepare(&rgba, cols, rows, &args.prep);
    let prepared = Instant::now();

    let pal = color::Palette::vga();
    let cells = convert::convert(&src, cols, rows, &pal, &args.opts);
    let matched = Instant::now();

    let mut data = ansi::encode(&cells, cols, rows, args.force_newlines);
    if args.sauce {
        let title = args.title.clone().unwrap_or_else(|| stem(&args.input));
        let sauce = ansi::Sauce {
            title: &title,
            author: &args.author,
            group: &args.group,
            date: &today_ccyymmdd(),
        };
        ansi::append_sauce(&mut data, &sauce, cols, rows, args.opts.ice);
    }
    let out_path = args.out.clone().unwrap_or_else(|| {
        std::path::Path::new(&args.input)
            .with_extension("ans")
            .to_string_lossy()
            .into_owned()
    });
    std::fs::write(&out_path, &data).map_err(|e| format!("cannot write {out_path}: {e}"))?;
    let mut written = vec![out_path.clone()];
    if args.image {
        let path = std::path::Path::new(&out_path).with_extension("ans.png");
        render::to_rgb_image(&cells, cols, rows)
            .save(&path)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        written.push(path.to_string_lossy().into_owned());
    }

    if let Some(path) = &args.png {
        render::to_rgb_image(&cells, cols, rows)
            .save(path)
            .map_err(|e| format!("cannot write {path}: {e}"))?;
        written.push(path.clone());
    }
    if let Some(path) = &args.src_png {
        src.to_rgb_image()
            .save(path)
            .map_err(|e| format!("cannot write {path}: {e}"))?;
        written.push(path.clone());
    }

    for path in &written {
        let full = std::fs::canonicalize(path).unwrap_or_else(|_| path.into());
        println!("wrote {}", full.display());
    }
    eprintln!(
        "{cols}x{rows} cells, {} bytes  (prep {:.0} ms, match {:.0} ms)",
        data.len(),
        (prepared - started).as_secs_f64() * 1000.0,
        (matched - prepared).as_secs_f64() * 1000.0,
    );
    Ok(())
}

fn main() -> ExitCode {
    match parse_args().and_then(run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) if msg.is_empty() => {
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
        Err(msg) => {
            eprintln!("shadeans: {msg}");
            ExitCode::from(1)
        }
    }
}
