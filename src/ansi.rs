//! .ANS output: CP437 bytes with SGR colour codes, plus a SAUCE record.

use crate::convert::Cell;
use crate::font;

/// DOS attribute colour (0-7) -> ANSI SGR colour digit.
const DOS_TO_SGR: [u8; 8] = [0, 4, 2, 6, 1, 5, 3, 7];

pub struct Sauce<'a> {
    pub title: &'a str,
    pub author: &'a str,
    pub group: &'a str,
    /// CCYYMMDD
    pub date: &'a str,
}

struct Attr {
    fg: u8,
    bg: u8,
}

const RESET: Attr = Attr { fg: 7, bg: 0 };

/// Emit the shortest SGR sequence that moves from `cur` to `want`. Bold (bright
/// fg) and blink (bright bg under iCE) can only be switched off by a reset.
fn write_sgr(out: &mut Vec<u8>, cur: &mut Attr, want: &Attr) {
    if cur.fg == want.fg && cur.bg == want.bg {
        return;
    }
    let (cur_bold, want_bold) = (cur.fg >= 8, want.fg >= 8);
    let (cur_blink, want_blink) = (cur.bg >= 8, want.bg >= 8);
    let mut params: Vec<String> = Vec::new();

    let mut from = Attr { fg: cur.fg, bg: cur.bg };
    if (cur_bold && !want_bold) || (cur_blink && !want_blink) {
        params.push("0".into());
        from = RESET;
    }
    if want_bold && from.fg < 8 {
        params.push("1".into());
    }
    if want_blink && from.bg < 8 {
        params.push("5".into());
    }
    if from.fg & 7 != want.fg & 7 {
        params.push(format!("3{}", DOS_TO_SGR[(want.fg & 7) as usize]));
    }
    if from.bg & 7 != want.bg & 7 {
        params.push(format!("4{}", DOS_TO_SGR[(want.bg & 7) as usize]));
    }

    out.extend_from_slice(b"\x1b[");
    out.extend_from_slice(params.join(";").as_bytes());
    out.push(b'm');
    cur.fg = want.fg;
    cur.bg = want.bg;
}

fn is_blank(cell: &Cell) -> bool {
    match cell.ch {
        font::SPACE => cell.bg == 0,
        font::FULL_BLOCK => cell.fg == 0,
        _ => cell.fg == 0 && cell.bg == 0,
    }
}

pub fn encode(cells: &[Cell], cols: usize, rows: usize, force_newlines: bool) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"\x1b[0m");
    let mut cur = RESET;

    for cy in 0..rows {
        let row = &cells[cy * cols..(cy + 1) * cols];
        let used = row.iter().rposition(|c| !is_blank(c)).map_or(0, |i| i + 1);

        for cell in &row[..used] {
            // A space shows no foreground and a full block no background, so
            // leave that half of the attribute alone and save the bytes.
            let want = match cell.ch {
                font::SPACE => Attr { fg: cur.fg, bg: cell.bg },
                font::FULL_BLOCK => Attr { fg: cell.fg, bg: cur.bg },
                _ => Attr { fg: cell.fg, bg: cell.bg },
            };
            write_sgr(&mut out, &mut cur, &want);
            out.push(cell.ch);
        }

        // A full 80-column row wraps by itself in ANSI viewers; a newline there
        // would double-space the art.
        if used < cols || cols != 80 || force_newlines {
            let black_bg = Attr { fg: cur.fg, bg: 0 };
            write_sgr(&mut out, &mut cur, &black_bg);
            out.extend_from_slice(b"\r\n");
        }
    }
    out.extend_from_slice(b"\x1b[0m");
    out
}

fn put_padded(record: &mut Vec<u8>, text: &str, len: usize) {
    let mut bytes: Vec<u8> = text.bytes().filter(|b| b.is_ascii() && *b >= 0x20).collect();
    bytes.resize(len, b' ');
    record.extend_from_slice(&bytes[..len]);
}

/// Append EOF marker + 128-byte SAUCE 00 record describing `data` as an ANSi file.
pub fn append_sauce(data: &mut Vec<u8>, sauce: &Sauce, cols: usize, rows: usize, ice: bool) {
    let file_size = data.len() as u32;
    let mut rec = Vec::with_capacity(129);
    rec.push(0x1A);
    rec.extend_from_slice(b"SAUCE00");
    put_padded(&mut rec, sauce.title, 35);
    put_padded(&mut rec, sauce.author, 20);
    put_padded(&mut rec, sauce.group, 20);
    put_padded(&mut rec, sauce.date, 8);
    rec.extend_from_slice(&file_size.to_le_bytes());
    rec.push(1); // DataType: Character
    rec.push(1); // FileType: ANSi
    rec.extend_from_slice(&(cols as u16).to_le_bytes());
    rec.extend_from_slice(&(rows as u16).to_le_bytes());
    rec.extend_from_slice(&[0; 4]); // TInfo3, TInfo4
    rec.push(0); // no comment block
    // TFlags: bit0 iCE colours, bits1-2 = 01 (8px font), bits3-4 = 10 (square pixels)
    rec.push(u8::from(ice) | (1 << 1) | (2 << 3));
    let mut font_name = b"IBM VGA".to_vec();
    font_name.resize(22, 0);
    rec.extend_from_slice(&font_name);
    debug_assert_eq!(rec.len(), 129);
    data.extend_from_slice(&rec);
}
