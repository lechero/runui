//! Terminal colors: the kinds Lip Gloss and x/ansi use (none, the 16 basic
//! ANSI colors, the 256 indexed ones and 24-bit RGB), how each is written
//! in an SGR sequence, and how a color is brought down to 256 or 16 colors
//! for a terminal that has no more (x/ansi's `Convert256`/`Convert16`).

/// Color is a terminal color, as `lipgloss.Color` makes them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Color {
    /// No color: Lip Gloss's `NoColor`, what an unset or invalid color is.
    #[default]
    NoColor,
    /// One of the 16 basic ANSI colors (`ansi.BasicColor`), 0 to 15.
    Basic(u8),
    /// One of the 256 indexed colors (`ansi.IndexedColor`).
    Indexed(u8),
    /// A 24-bit color.
    Rgb(u8, u8, u8),
}

impl Color {
    /// rgb is the color's red, green and blue, as Go's `RGBA` gives them
    /// (no color is black).
    pub fn rgb(&self) -> (u8, u8, u8) {
        match *self {
            Color::NoColor => (0, 0, 0),
            Color::Basic(c) if c > 15 => (0, 0, 0),
            Color::Basic(c) | Color::Indexed(c) => ANSI_HEX[c as usize],
            Color::Rgb(r, g, b) => (r, g, b),
        }
    }
}

/// color parses a color the way `lipgloss.Color` does: "#RRGGBB" or "#RGB"
/// for 24-bit colors, a number below 16 for a basic ANSI color, below 256
/// for an indexed one, larger numbers as 0xRRGGBB; anything else is no color.
pub const fn color(s: &str) -> Color {
    let b = s.as_bytes();
    if !b.is_empty() && b[0] == b'#' {
        return parse_hex(b);
    }
    // strconv.Atoi: an optional sign, then decimal digits.
    let mut i = 0;
    let mut neg = false;
    if !b.is_empty() && (b[0] == b'+' || b[0] == b'-') {
        neg = b[0] == b'-';
        i = 1;
    }
    if i >= b.len() {
        return Color::NoColor;
    }
    let mut n: i64 = 0;
    while i < b.len() {
        let d = b[i];
        if d < b'0' || d > b'9' {
            return Color::NoColor;
        }
        n = match n.checked_mul(10) {
            Some(v) => match v.checked_add((d - b'0') as i64) {
                Some(v) => v,
                None => return Color::NoColor,
            },
            None => return Color::NoColor,
        };
        i += 1;
    }
    if neg {
        n = -n;
    }
    if n < 0 {
        n = -n;
    }
    if n < 16 {
        Color::Basic(n as u8)
    } else if n < 256 {
        Color::Indexed(n as u8)
    } else {
        Color::Rgb(((n >> 16) & 0xff) as u8, ((n >> 8) & 0xff) as u8, (n & 0xff) as u8)
    }
}

const fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

const fn parse_hex(b: &[u8]) -> Color {
    let mut v = [0u8; 6];
    let n = b.len();
    if n != 7 && n != 4 {
        return Color::NoColor;
    }
    let mut i = 1;
    while i < n {
        match hex_digit(b[i]) {
            Some(d) => v[i - 1] = d,
            None => return Color::NoColor,
        }
        i += 1;
    }
    if n == 7 {
        Color::Rgb(v[0] << 4 | v[1], v[2] << 4 | v[3], v[4] << 4 | v[5])
    } else {
        Color::Rgb(v[0] * 17, v[1] * 17, v[2] * 17)
    }
}

/// fg_sgr is the SGR parameter that sets `c` as the foreground.
pub(crate) fn fg_sgr(c: Option<Color>) -> String {
    match c {
        None => "39".into(),
        Some(Color::Basic(c)) if c < 8 => format!("{}", 30 + c as u32),
        Some(Color::Basic(c)) if c < 16 => format!("{}", 90 + (c as u32 - 8)),
        Some(Color::Basic(_)) => "39".into(),
        Some(Color::Indexed(c)) => format!("38;5;{c}"),
        Some(c) => {
            let (r, g, b) = c.rgb();
            format!("38;2;{r};{g};{b}")
        }
    }
}

/// bg_sgr is the SGR parameter that sets `c` as the background.
pub(crate) fn bg_sgr(c: Option<Color>) -> String {
    match c {
        None => "49".into(),
        Some(Color::Basic(c)) if c < 8 => format!("{}", 40 + c as u32),
        Some(Color::Basic(c)) if c < 16 => format!("{}", 100 + (c as u32 - 8)),
        Some(Color::Basic(_)) => "49".into(),
        Some(Color::Indexed(c)) => format!("48;5;{c}"),
        Some(c) => {
            let (r, g, b) = c.rgb();
            format!("48;2;{r};{g};{b}")
        }
    }
}

/// ul_sgr is the SGR parameter that sets `c` as the underline color.
pub(crate) fn ul_sgr(c: Option<Color>) -> String {
    match c {
        None => "59".into(),
        Some(Color::Basic(c)) | Some(Color::Indexed(c)) => format!("58;5;{c}"),
        Some(c) => {
            let (r, g, b) = c.rgb();
            format!("58;2;{r};{g};{b}")
        }
    }
}

/// The 256 colors' RGB values, as x/ansi has them.
pub(crate) static ANSI_HEX: [(u8, u8, u8); 256] = ansi_hex();

const fn ansi_hex() -> [(u8, u8, u8); 256] {
    let mut t = [(0u8, 0u8, 0u8); 256];
    let base: [(u8, u8, u8); 16] = [
        (0x00, 0x00, 0x00),
        (0x80, 0x00, 0x00),
        (0x00, 0x80, 0x00),
        (0x80, 0x80, 0x00),
        (0x00, 0x00, 0x80),
        (0x80, 0x00, 0x80),
        (0x00, 0x80, 0x80),
        (0xc0, 0xc0, 0xc0),
        (0x80, 0x80, 0x80),
        (0xff, 0x00, 0x00),
        (0x00, 0xff, 0x00),
        (0xff, 0xff, 0x00),
        (0x00, 0x00, 0xff),
        (0xff, 0x00, 0xff),
        (0x00, 0xff, 0xff),
        (0xff, 0xff, 0xff),
    ];
    let mut i = 0;
    while i < 16 {
        t[i] = base[i];
        i += 1;
    }
    let q2c: [u8; 6] = [0x00, 0x5f, 0x87, 0xaf, 0xd7, 0xff];
    let mut i = 16;
    while i < 232 {
        let n = i - 16;
        t[i] = (q2c[n / 36], q2c[(n / 6) % 6], q2c[n % 6]);
        i += 1;
    }
    while i < 256 {
        let g = (8 + 10 * (i - 232)) as u8;
        t[i] = (g, g, g);
        i += 1;
    }
    t
}

/// Which of the 16 colors each of the 256 is closest to, as x/ansi has it.
static ANSI_256_TO_16: [u8; 256] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 0, 4, 4, 4, 12, 12, 2, 6, 4, 4, 12, 12, 2, 2, 6, 4, 12, 12,
    2, 2, 2, 6, 12, 12, 10, 10, 10, 10, 14, 12, 10, 10, 10, 10, 10, 14, 1, 5, 4, 4, 12, 12, 3, 8, 4, 4, 12, 12, 2, 2,
    6, 4, 12, 12, 2, 2, 2, 6, 12, 12, 10, 10, 10, 10, 14, 12, 10, 10, 10, 10, 10, 14, 1, 1, 5, 4, 12, 12, 1, 1, 5, 4,
    12, 12, 3, 3, 8, 4, 12, 12, 2, 2, 2, 6, 12, 12, 10, 10, 10, 10, 14, 12, 10, 10, 10, 10, 10, 14, 1, 1, 1, 5, 12, 12,
    1, 1, 1, 5, 12, 12, 1, 1, 1, 5, 12, 12, 3, 3, 3, 7, 12, 12, 10, 10, 10, 10, 14, 12, 10, 10, 10, 10, 10, 14, 9, 9,
    9, 9, 13, 12, 9, 9, 9, 9, 13, 12, 9, 9, 9, 9, 13, 12, 9, 9, 9, 9, 13, 12, 11, 11, 11, 11, 7, 12, 10, 10, 10, 10,
    10, 14, 9, 9, 9, 9, 9, 13, 9, 9, 9, 9, 9, 13, 9, 9, 9, 9, 9, 13, 9, 9, 9, 9, 9, 13, 9, 9, 9, 9, 9, 13, 11, 11, 11,
    11, 11, 15, 0, 0, 0, 0, 0, 0, 8, 8, 8, 8, 8, 8, 7, 7, 7, 7, 7, 7, 15, 15, 15, 15, 15, 15,
];

fn to_6_cube(v: f64) -> usize {
    if v < 48.0 {
        return 0;
    }
    if v < 115.0 {
        return 1;
    }
    ((v - 35.0) / 40.0) as usize
}

/// convert256 is the closest of the 256 indexed colors, as x/ansi's
/// `Convert256` picks it (tmux's way, then the nearer of the cube color and
/// the grey in HSLuv).
pub(crate) fn convert256(c: Color) -> u8 {
    if let Color::Indexed(i) = c {
        return i;
    }
    let (r8, g8, b8) = c.rgb();
    // as go-colorful's MakeColor has it: 16-bit channels over 65535
    let unit = |v: u8| (v as u32 * 257) as f64 / 65535.0;
    let col = (unit(r8), unit(g8), unit(b8));
    let (r, g, b) = (col.0 * 255.0, col.1 * 255.0, col.2 * 255.0);
    let q2c = [0x00, 0x5f, 0x87, 0xaf, 0xd7, 0xff];
    let (qr, qg, qb) = (to_6_cube(r), to_6_cube(g), to_6_cube(b));
    let (cr, cg, cb) = (q2c[qr], q2c[qg], q2c[qb]);
    let ci = 36 * qr + 6 * qg + qb;
    if cr == r as i32 && cg == g as i32 && cb == b as i32 {
        return (16 + ci) as u8;
    }
    let grey_avg = (r + g + b) as i32 / 3;
    let grey_idx = if grey_avg > 238 { 23 } else { (grey_avg - 3) / 10 };
    let grey = 8 + 10 * grey_idx;
    let c2 = (cr as f64 / 255.0, cg as f64 / 255.0, cb as f64 / 255.0);
    let g2 = (grey as f64 / 255.0, grey as f64 / 255.0, grey as f64 / 255.0);
    if distance_hsluv(col, c2) <= distance_hsluv(col, g2) { (16 + ci) as u8 } else { (232 + grey_idx) as u8 }
}

/// convert16 is the closest of the 16 basic colors.
pub(crate) fn convert16(c: Color) -> u8 {
    match c {
        Color::Basic(b) => b,
        Color::Indexed(i) => ANSI_256_TO_16[i as usize],
        other => ANSI_256_TO_16[convert256(other) as usize],
    }
}

// HSLuv, as go-colorful computes it, for the distance Convert256 uses; its
// constants as go-colorful writes them.

#[allow(clippy::excessive_precision)]
const M: [[f64; 3]; 3] = [
    [3.2409699419045214, -1.5373831775700935, -0.49861076029300328],
    [-0.96924363628087983, 1.8759675015077207, 0.041555057407175613],
    [0.055630079696993609, -0.20397695888897657, 1.0569715142428786],
];
#[allow(clippy::excessive_precision)]
const KAPPA: f64 = 903.2962962962963;
#[allow(clippy::excessive_precision)]
const EPSILON: f64 = 0.0088564516790356308;
#[allow(clippy::excessive_precision)]
const HSLUV_D65: [f64; 3] = [0.95045592705167, 1.0, 1.089057750759878];

fn linearize(v: f64) -> f64 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn xyz_to_uv(x: f64, y: f64, z: f64) -> (f64, f64) {
    let denom = x + 15.0 * y + 3.0 * z;
    if denom == 0.0 { (0.0, 0.0) } else { (4.0 * x / denom, 9.0 * y / denom) }
}

#[allow(clippy::excessive_precision)]
fn luv_lch(c: (f64, f64, f64)) -> (f64, f64, f64) {
    let (r, g, b) = (linearize(c.0), linearize(c.1), linearize(c.2));
    let x = 0.41239079926595948 * r + 0.35758433938387796 * g + 0.18048078840183429 * b;
    let y = 0.21263900587151036 * r + 0.71516867876775593 * g + 0.072192315360733715 * b;
    let z = 0.019330818715591851 * r + 0.11919477979462599 * g + 0.95053215224966058 * b;
    let wref = HSLUV_D65;
    let l = if y / wref[1] <= 6.0 / 29.0 * 6.0 / 29.0 * 6.0 / 29.0 {
        y / wref[1] * (29.0 / 3.0 * 29.0 / 3.0 * 29.0 / 3.0) / 100.0
    } else {
        1.16 * (y / wref[1]).cbrt() - 0.16
    };
    let (ubis, vbis) = xyz_to_uv(x, y, z);
    let (un, vn) = xyz_to_uv(wref[0], wref[1], wref[2]);
    let u = 13.0 * l * (ubis - un);
    let v = 13.0 * l * (vbis - vn);
    let h = if (v - u).abs() > 1e-4 && u.abs() > 1e-4 {
        (57.29577951308232087721 * v.atan2(u) + 360.0) % 360.0
    } else {
        0.0
    };
    (l, (u * u + v * v).sqrt(), h)
}

fn bounds(l: f64) -> [[f64; 2]; 6] {
    let sub1 = (l + 16.0).powi(3) / 1560896.0;
    let sub2 = if sub1 > EPSILON { sub1 } else { l / KAPPA };
    let mut ret = [[0.0; 2]; 6];
    for (i, m) in M.iter().enumerate() {
        for k in 0..2 {
            let kf = k as f64;
            let top1 = (284517.0 * m[0] - 94839.0 * m[2]) * sub2;
            let top2 = (838422.0 * m[2] + 769860.0 * m[1] + 731718.0 * m[0]) * l * sub2 - 769860.0 * kf * l;
            let bottom = (632260.0 * m[2] - 126452.0 * m[1]) * sub2 + 126452.0 * kf;
            ret[i * 2 + k] = [top1 / bottom, top2 / bottom];
        }
    }
    ret
}

fn max_chroma(l: f64, h: f64) -> f64 {
    let h_rad = h / 360.0 * std::f64::consts::PI * 2.0;
    let mut min_length = f64::MAX;
    for line in bounds(l) {
        let length = line[1] / (h_rad.sin() - line[0] * h_rad.cos());
        if length > 0.0 && length < min_length {
            min_length = length;
        }
    }
    min_length
}

fn hsluv(c: (f64, f64, f64)) -> (f64, f64, f64) {
    let (l, c, h) = luv_lch(c);
    let (c, l) = (c * 100.0, l * 100.0);
    let s = if !(0.00000001..=99.9999999).contains(&l) { 0.0 } else { c / max_chroma(l, h) * 100.0 };
    (h, (s / 100.0).clamp(0.0, 1.0), (l / 100.0).clamp(0.0, 1.0))
}

fn distance_hsluv(a: (f64, f64, f64), b: (f64, f64, f64)) -> f64 {
    let (h1, s1, l1) = hsluv(a);
    let (h2, s2, l2) = hsluv(b);
    (((h1 - h2) / 100.0).powi(2) + (s1 - s2).powi(2) + (l1 - l2).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_parses_like_lipgloss() {
        assert_eq!(color("#7DD3FC"), Color::Rgb(0x7D, 0xD3, 0xFC));
        assert_eq!(color("#abc"), Color::Rgb(0xaa, 0xbb, 0xcc));
        assert_eq!(color("#zzzzzz"), Color::NoColor);
        assert_eq!(color("#12345"), Color::NoColor);
        assert_eq!(color("7"), Color::Basic(7));
        assert_eq!(color("240"), Color::Indexed(240));
        assert_eq!(color("-21"), Color::Indexed(21));
        assert_eq!(color("4095"), Color::Rgb(0, 0x0f, 0xff));
        assert_eq!(color(""), Color::NoColor);
        assert_eq!(color("red"), Color::NoColor);
    }

    #[test]
    fn sgr_parameters() {
        assert_eq!(fg_sgr(Some(color("#7DD3FC"))), "38;2;125;211;252");
        assert_eq!(fg_sgr(Some(color("7"))), "37");
        assert_eq!(fg_sgr(Some(color("9"))), "91");
        assert_eq!(bg_sgr(Some(color("240"))), "48;5;240");
        assert_eq!(ul_sgr(Some(color("1"))), "58;5;1");
    }

    #[test]
    fn converts_down() {
        // exact cube colors and greys
        assert_eq!(convert256(Color::Rgb(0, 0, 0)), 16);
        assert_eq!(convert256(Color::Rgb(0xff, 0xff, 0xff)), 231);
        assert_eq!(convert256(Color::Rgb(0x5f, 0x87, 0xaf)), 67);
        assert_eq!(convert16(Color::Indexed(196)), 9);
        assert_eq!(ANSI_HEX[232], (8, 8, 8));
        assert_eq!(ANSI_HEX[21], (0, 0, 0xff));
    }
}
