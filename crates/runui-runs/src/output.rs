//! What a run printed, as the pane shows it.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::Result;
use runui_core::golang;

/// TAIL_BYTES is how much of a long output is read when it's first shown:
/// its end.
const TAIL_BYTES: u64 = 512 << 10;

/// READ_CHUNK is the most read at a time, so a command printing fast
/// doesn't hold the interface up.
const READ_CHUNK: u64 = 1 << 20;

/// Output is what a run printed, as the pane shows it: its last lines,
/// read again as the run prints more.
#[derive(Debug)]
pub struct Output {
    path: String,
    /// lines kept
    max: usize,
    offset: i64,
    lines: Vec<String>,
    /// the last line, until its newline comes
    part: Vec<u8>,
}

impl Output {
    /// new reads a run's output, keeping its last max lines.
    pub fn new(path: &str, max: usize) -> Output {
        Output { path: path.to_string(), max, offset: -1, lines: Vec::new(), part: Vec::new() }
    }

    /// read reads what was printed since it last read, and says whether
    /// there was anything.
    pub fn read(&mut self) -> Result<bool> {
        let mut f = File::open(&self.path).map_err(|e| golang::path_error("open", Path::new(&self.path), e))?;
        let size = f.metadata().map_err(|e| golang::path_error("stat", Path::new(&self.path), e))?.len() as i64;
        let mut skip_first = false;
        if self.offset < 0 && size > TAIL_BYTES as i64 {
            (self.offset, skip_first) = (size - TAIL_BYTES as i64, true);
        } else if self.offset < 0 || size < self.offset {
            // the first read, or a file started again
            (self.offset, self.lines, self.part) = (0, Vec::new(), Vec::new());
        }
        if size == self.offset {
            return Ok(false);
        }
        let want = (size - self.offset).min(READ_CHUNK as i64) as usize;
        let mut buf = vec![0u8; want];
        f.seek(SeekFrom::Start(self.offset as u64))
            .map_err(|e| golang::path_error("read", Path::new(&self.path), e))?;
        let mut n = 0;
        while n < want {
            match f.read(&mut buf[n..]) {
                Ok(0) => break,
                Ok(m) => n += m,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(golang::path_error("read", Path::new(&self.path), e)),
            }
        }
        self.offset += n as i64;
        let mut text = &buf[..n];
        if skip_first && let Some(i) = text.iter().position(|&c| c == b'\n') {
            text = &text[i + 1..];
        }
        self.add(text);
        Ok(true)
    }

    /// add splits text into lines, the last one kept apart until its
    /// newline.
    fn add(&mut self, text: &[u8]) {
        let mut all = std::mem::take(&mut self.part);
        all.extend_from_slice(text);
        let mut lines: Vec<&[u8]> = all.split(|&c| c == b'\n').collect();
        let part = lines.pop().unwrap_or_default().to_vec();
        for l in lines {
            self.lines.push(clean(&String::from_utf8_lossy(l)));
        }
        self.part = part;
        if self.lines.len() > self.max {
            let over = self.lines.len() - self.max;
            self.lines.drain(..over);
        }
    }

    /// lines are the lines read, the one still being printed last.
    pub fn lines(&self) -> Vec<String> {
        let mut out = self.lines.clone();
        if !self.part.is_empty() {
            out.push(clean(&String::from_utf8_lossy(&self.part)));
        }
        out
    }

    /// range is lines from..to of lines, without making the rest: the ones
    /// a pane shows.
    pub fn range(&self, from: usize, to: usize) -> Vec<String> {
        let to = to.min(self.len());
        (from.min(to)..to)
            .map(|i| match self.lines.get(i) {
                Some(l) => l.clone(),
                None => clean(&String::from_utf8_lossy(&self.part)),
            })
            .collect()
    }

    /// len is how many lines lines has, without making them.
    pub fn len(&self) -> usize {
        self.lines.len() + usize::from(!self.part.is_empty())
    }

    /// is_empty says nothing's been printed.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// clean is a line of output as a pane can draw it: what a carriage return
/// overwrote dropped (a progress bar's earlier states), its colours kept
/// and every other escape sequence dropped (moving the cursor, clearing the
/// screen), and tabs as spaces.
pub fn clean(line: &str) -> String {
    let mut line = line.strip_suffix('\r').unwrap_or(line);
    if let Some(i) = line.rfind('\r') {
        line = &line[i + 1..];
    }
    let line = line.as_bytes();
    let mut b: Vec<u8> = Vec::with_capacity(line.len());
    let mut col = 0;
    let mut i = 0;
    while i < line.len() {
        let c = line[i];
        if c == 0x1b && i + 1 < line.len() && line[i + 1] == b'[' {
            let mut j = i + 2;
            while j < line.len() && (line[j] < 0x40 || line[j] > 0x7e) {
                j += 1;
            }
            if j < line.len() && line[j] == b'm' {
                b.extend_from_slice(&line[i..=j]); // a colour
            }
            i = j;
        } else if c == 0x1b && i + 1 < line.len() && line[i + 1] == b']' {
            // an OSC, a hyperlink or a title, up to BEL or ESC \
            let mut j = i + 2;
            while j < line.len() && line[j] != 0x07 && (line[j] != 0x1b || j + 1 >= line.len() || line[j + 1] != b'\\')
            {
                j += 1;
            }
            if j < line.len() && line[j] == 0x1b {
                j += 1;
            }
            i = j;
        } else if c == 0x1b {
            // any other escape: its intermediate bytes (ESC ( B), then its final one
            while i + 1 < line.len() && (0x20..=0x2f).contains(&line[i + 1]) {
                i += 1;
            }
            i += 1;
        } else if c == b'\t' {
            let n = 8 - col % 8;
            b.extend(std::iter::repeat_n(b' ', n));
            col += n;
        } else if c < 0x20 || c == 0x7f {
        } else {
            b.push(c);
            if !(0x80..0xc0).contains(&c) {
                col += 1; // a character's first byte
            }
        }
        i += 1;
    }
    // dropping escapes can cut no character in two: the bytes kept are
    // whole characters, or what was in the line already
    String::from_utf8(b).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}
