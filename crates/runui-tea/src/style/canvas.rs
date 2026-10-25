//! Layers composited on a canvas, as Lip Gloss v2 does it on ultraviolet's
//! cell buffer: each layer is a block of styled text at an x, y and z; the
//! canvas draws them from the lowest z up, each blanking the cells it
//! covers first, and renders the cells back into styled lines.

use super::layout;
use super::wrap::{RESET_HYPERLINK, set_hyperlink};
use crate::ansi::{self, RESET_STYLE, Token, sgr::Pen, sgr::read_style};

/// A cell on the canvas: what's drawn in it, its style and hyperlink, and
/// how many cells it takes (0 for the cells a wide one covers).
#[derive(Clone, Debug, Default, PartialEq)]
struct Cell {
    content: String,
    style: Pen,
    link: (String, String),
    width: i32,
}

impl Cell {
    fn empty_cell() -> Cell {
        Cell { content: " ".into(), width: 1, ..Default::default() }
    }
    fn is_zero(&self) -> bool {
        self.content.is_empty()
            && self.width == 0
            && self.style.is_zero()
            && self.link.0.is_empty()
            && self.link.1.is_empty()
    }
    fn is_empty_cell(&self) -> bool {
        self.width == 1
            && self.content == " "
            && self.style.equal(&Pen::default())
            && self.link.0.is_empty()
            && self.link.1.is_empty()
    }
    /// empty makes the cell a space, keeping its style.
    fn empty(&mut self) {
        self.content = " ".into();
        self.width = 1;
    }
}

/// A rectangle of cells: from min (included) to max (not).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Rect {
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
}

impl Rect {
    fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { min_x: x, min_y: y, max_x: x + w, max_y: y + h }
    }
    fn is_empty(&self) -> bool {
        self.min_x >= self.max_x || self.min_y >= self.max_y
    }
    fn overlaps(&self, s: &Rect) -> bool {
        !self.is_empty()
            && !s.is_empty()
            && self.min_x < s.max_x
            && s.min_x < self.max_x
            && self.min_y < s.max_y
            && s.min_y < self.max_y
    }
    fn union(&self, s: &Rect) -> Rect {
        if self.is_empty() {
            return *s;
        }
        if s.is_empty() {
            return *self;
        }
        Rect {
            min_x: self.min_x.min(s.min_x),
            min_y: self.min_y.min(s.min_y),
            max_x: self.max_x.max(s.max_x),
            max_y: self.max_y.max(s.max_y),
        }
    }
    fn contains(&self, x: i32, y: i32) -> bool {
        self.min_x <= x && x < self.max_x && self.min_y <= y && y < self.max_y
    }
}

/// Canvas is a grid of cells layers are drawn on: Lip Gloss's `Canvas`.
#[derive(Clone, Debug, Default)]
pub struct Canvas {
    lines: Vec<Vec<Cell>>,
}

impl Canvas {
    /// new is a blank canvas `width` by `height` cells.
    pub fn new(width: i32, height: i32) -> Canvas {
        let mut c = Canvas::default();
        c.resize(width, height);
        c
    }

    /// width is how many cells wide the canvas is.
    pub fn width(&self) -> i32 {
        self.lines.first().map(|l| l.len() as i32).unwrap_or(0)
    }

    /// height is how many lines the canvas has.
    pub fn height(&self) -> i32 {
        self.lines.len() as i32
    }

    /// resize makes the canvas `width` by `height`, keeping what fits.
    pub fn resize(&mut self, width: i32, height: i32) {
        let (w, h) = (width.max(0) as usize, height.max(0) as usize);
        for l in self.lines.iter_mut() {
            l.resize(w, Cell::empty_cell());
        }
        self.lines.resize(h, vec![Cell::empty_cell(); w]);
    }

    /// clear blanks every cell.
    pub fn clear(&mut self) {
        for l in self.lines.iter_mut() {
            for c in l.iter_mut() {
                *c = Cell::empty_cell();
            }
        }
    }

    fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width(), self.height())
    }

    fn cell_at(&self, x: i32, y: i32) -> Option<&Cell> {
        if y < 0 || x < 0 {
            return None;
        }
        self.lines.get(y as usize).and_then(|l| l.get(x as usize))
    }

    /// set_cell puts a cell at x, y (a blank one for none), clearing what a
    /// wide cell it overwrites covered.
    fn set_cell(&mut self, x: i32, y: i32, c: Option<&Cell>) {
        if y < 0 || y as usize >= self.lines.len() {
            return;
        }
        let l = &mut self.lines[y as usize];
        let width = l.len() as i32;
        if x < 0 || x >= width {
            return;
        }
        let xu = x as usize;
        let prev = l[xu].clone();
        if prev.width > 1 {
            let mut j = 0;
            while j < prev.width && x + j < width {
                l[(x + j) as usize] = prev.clone();
                l[(x + j) as usize].empty();
                j += 1;
            }
        } else if prev.width == 0 {
            let mut j = 1;
            while x - j >= 0 {
                let wide = l[(x - j) as usize].clone();
                if wide.width > 1 && j < wide.width {
                    for k in 0..wide.width {
                        let at = x - j + k;
                        if at < width {
                            l[at as usize] = wide.clone();
                            l[at as usize].empty();
                        }
                    }
                    break;
                }
                j += 1;
            }
        }
        let Some(c) = c else {
            l[xu] = Cell::empty_cell();
            return;
        };
        l[xu] = c.clone();
        let cw = c.width;
        if x + cw > width {
            let mut i = 0;
            while i < cw && x + i < width {
                l[(x + i) as usize] = c.clone();
                l[(x + i) as usize].empty();
                i += 1;
            }
            return;
        }
        if cw > 1 {
            let mut j = 1;
            while j < cw && x + j < width {
                l[(x + j) as usize] = Cell::default();
                j += 1;
            }
        }
    }

    /// compose draws a layer or a compositor's layers on the canvas.
    pub fn compose(&mut self, d: &dyn Drawable) -> &mut Canvas {
        d.draw_on(self);
        self
    }

    /// render is the canvas as styled lines, trailing spaces trimmed.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for (i, l) in self.lines.iter().enumerate() {
            render_line(&mut out, l);
            if i < self.lines.len() - 1 {
                out.push('\n');
            }
        }
        trim_space(&out)
    }

    /// draw_styled draws styled text into `area`: the area blanked first,
    /// then the text, cut off where the area ends.
    fn draw_styled(&mut self, text: &str, area: Rect) {
        for y in area.min_y..area.max_y {
            for x in area.min_x..area.max_x {
                self.set_cell(x, y, None);
            }
        }
        let text = text.replace("\r\n", "\n");
        let (mut x, mut y) = (area.min_x, area.min_y);
        let mut pen = Pen::default();
        let mut link = (String::new(), String::new());
        let (mut last_x, mut last_y) = (-1, -1);
        for tok in ansi::tokens(&text) {
            match tok {
                Token::Text(seq, width) if width > 0 => {
                    if area.contains(x, y) {
                        let cell = Cell { content: seq.to_string(), style: pen, link: link.clone(), width };
                        self.set_cell(x, y, Some(&cell));
                        last_x = x;
                        last_y = y;
                        x += width;
                    }
                }
                Token::Text(seq, _) => {
                    if seq.as_bytes()[0] >= 0xc0
                        && last_x >= 0
                        && let Some(prev) = self.cell_at(last_x, last_y)
                    {
                        let mut folded = prev.clone();
                        folded.content.push_str(seq);
                        self.set_cell(last_x, last_y, Some(&folded));
                    }
                }
                Token::Sgr(params) => read_style(&params, &mut pen),
                Token::Link(_, params, url) => link = (params, url),
                Token::Control("\n") => {
                    y += 1;
                    x = area.min_x;
                }
                Token::Control("\r") => x = area.min_x,
                Token::Control(_) | Token::Other(_) => {}
            }
            if y >= area.max_y {
                break;
            }
        }
    }
}

/// render_line writes a line of cells, each style as the change from the
/// one before.
fn render_line(buf: &mut String, l: &[Cell]) {
    let mut pen = Pen::default();
    let mut link = (String::new(), String::new());
    let mut pending = String::new();
    for c in l {
        if c.is_zero() {
            continue;
        }
        if c.is_empty_cell() {
            if !pen.is_zero() {
                buf.push_str(RESET_STYLE);
                pen = Pen::default();
            }
            if !link.0.is_empty() || !link.1.is_empty() {
                buf.push_str(RESET_HYPERLINK);
                link = Default::default();
            }
            pending.push(' ');
            continue;
        }
        if !pending.is_empty() {
            buf.push_str(&pending);
            pending.clear();
        }
        if c.style.is_zero() && !pen.is_zero() {
            buf.push_str(RESET_STYLE);
            pen = Pen::default();
        }
        if !c.style.equal(&pen) {
            buf.push_str(&c.style.diff(&pen));
            pen = c.style;
        }
        if c.link != link && !link.1.is_empty() {
            buf.push_str(RESET_HYPERLINK);
            link = Default::default();
        }
        if c.link != link {
            buf.push_str(&set_hyperlink(&c.link.1, &c.link.0));
            link = c.link.clone();
        }
        buf.push_str(&c.content);
    }
    if !pending.is_empty() {
        buf.push_str(&pending);
    }
    if !link.1.is_empty() {
        buf.push_str(RESET_HYPERLINK);
    }
    if !pen.is_zero() {
        buf.push_str(RESET_STYLE);
    }
}

/// trim_space drops the spaces ending each line.
fn trim_space(s: &str) -> String {
    s.split('\n')
        .map(|line| match line.strip_suffix('\r') {
            Some(l) => l.trim_end_matches(' ').to_string() + "\r",
            None => line.trim_end_matches(' ').to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Drawable is what can be drawn on a canvas: a layer or a compositor.
pub trait Drawable {
    /// draw_on draws on the whole of `canvas`.
    fn draw_on(&self, canvas: &mut Canvas);
}

/// Layer is a block of styled text at a position and depth, with layers of
/// its own placed relative to it: Lip Gloss's `Layer`.
#[derive(Clone, Debug, Default)]
pub struct Layer {
    id: String,
    content: String,
    width: i32,
    height: i32,
    x: i32,
    y: i32,
    z: i32,
    layers: Vec<Layer>,
}

impl Layer {
    /// new is a layer of `content` at 0, 0, depth 0.
    pub fn new(content: impl Into<String>) -> Layer {
        let mut l = Layer { content: content.into(), ..Default::default() };
        l.update_size();
        l
    }

    /// x places the layer `x` cells right of its parent.
    pub fn x(mut self, x: i32) -> Layer {
        self.x = x;
        self
    }

    /// y places the layer `y` lines below its parent.
    pub fn y(mut self, y: i32) -> Layer {
        self.y = y;
        self
    }

    /// z sets the layer's depth: higher is drawn later, over lower ones.
    pub fn z(mut self, z: i32) -> Layer {
        self.z = z;
        self
    }

    /// id names the layer, for finding it.
    pub fn id(mut self, id: impl Into<String>) -> Layer {
        self.id = id.into();
        self
    }

    /// add_layers puts layers in this one.
    pub fn add_layers(mut self, layers: Vec<Layer>) -> Layer {
        self.layers.extend(layers);
        self.update_size();
        self
    }

    fn update_size(&mut self) {
        let area = self.bounds_with_offset(0, 0);
        self.width = area.max_x - area.min_x;
        self.height = area.max_y - area.min_y;
    }

    /// get_content is the layer's text.
    pub fn get_content(&self) -> &str {
        &self.content
    }
    /// width is how wide the layer and its children are.
    pub fn width(&self) -> i32 {
        self.width
    }
    /// height is how tall the layer and its children are.
    pub fn height(&self) -> i32 {
        self.height
    }
    /// get_id is the layer's name.
    pub fn get_id(&self) -> &str {
        &self.id
    }
    /// get_x is the layer's x.
    pub fn get_x(&self) -> i32 {
        self.x
    }
    /// get_y is the layer's y.
    pub fn get_y(&self) -> i32 {
        self.y
    }
    /// get_z is the layer's depth.
    pub fn get_z(&self) -> i32 {
        self.z
    }
    /// max_z is the deepest of the layer and its children.
    pub fn max_z(&self) -> i32 {
        self.layers.iter().map(Layer::max_z).fold(self.z, i32::max)
    }

    fn bounds_with_offset(&self, px: i32, py: i32) -> Rect {
        let (ax, ay) = (self.x + px, self.y + py);
        let (w, h) = layout::size(&self.content);
        let mut b = Rect::new(ax, ay, w, h);
        for c in &self.layers {
            b = b.union(&c.bounds_with_offset(ax, ay));
        }
        b
    }
}

impl Drawable for Layer {
    fn draw_on(&self, canvas: &mut Canvas) {
        let area = canvas.bounds();
        canvas.draw_styled(&self.content, area);
    }
}

/// A layer flattened: its text, depth, name and where it is on the canvas.
#[derive(Clone, Debug)]
struct Flat {
    content: String,
    id: String,
    z: i32,
    bounds: Rect,
}

/// Compositor flattens layers and their children into one list, lowest
/// depth first, and draws them: Lip Gloss's `Compositor`.
#[derive(Clone, Debug)]
pub struct Compositor {
    root: Layer,
    layers: Vec<Flat>,
    bounds: Rect,
}

impl Compositor {
    /// new composes `layers`.
    pub fn new(layers: Vec<Layer>) -> Compositor {
        let mut c = Compositor { root: Layer::new("").add_layers(layers), layers: Vec::new(), bounds: Rect::default() };
        c.flatten();
        c
    }

    /// add_layers adds more layers.
    pub fn add_layers(mut self, layers: Vec<Layer>) -> Compositor {
        self.root = std::mem::take(&mut self.root).add_layers(layers);
        self.flatten();
        self
    }

    fn flatten(&mut self) {
        let mut out = Vec::new();
        flatten_into(&self.root, 0, 0, &mut out);
        out.sort_by_key(|f| f.z);
        self.bounds = out.iter().fold(Rect::default(), |b, f| b.union(&f.bounds));
        if let Some(first) = out.first() {
            self.bounds = out.iter().skip(1).fold(first.bounds, |b, f| b.union(&f.bounds));
        }
        self.layers = out;
    }

    /// bounds is the area every layer covers: its x, y, width and height.
    pub fn bounds(&self) -> (i32, i32, i32, i32) {
        let b = self.bounds;
        (b.min_x, b.min_y, b.max_x - b.min_x, b.max_y - b.min_y)
    }

    /// hit is the name of the topmost named layer at x, y, if any.
    pub fn hit(&self, x: i32, y: i32) -> Option<&str> {
        self.layers.iter().rev().find(|f| !f.id.is_empty() && f.bounds.contains(x, y)).map(|f| f.id.as_str())
    }

    /// render draws the layers on a canvas just big enough for them.
    pub fn render(&self) -> String {
        let (_, _, w, h) = self.bounds();
        let mut c = Canvas::new(w, h);
        c.compose(self).render()
    }
}

fn flatten_into(l: &Layer, px: i32, py: i32, out: &mut Vec<Flat>) {
    let (ax, ay) = (l.x + px, l.y + py);
    let (w, h) = layout::size(&l.content);
    out.push(Flat { content: l.content.clone(), id: l.id.clone(), z: l.z, bounds: Rect::new(ax, ay, w, h) });
    for c in &l.layers {
        flatten_into(c, ax, ay, out);
    }
}

impl Drawable for Compositor {
    fn draw_on(&self, canvas: &mut Canvas) {
        let area = canvas.bounds();
        for f in &self.layers {
            if f.bounds.overlaps(&area) {
                canvas.draw_styled(&f.content, f.bounds);
            }
        }
    }
}
