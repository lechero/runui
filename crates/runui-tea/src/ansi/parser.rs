//! The state machine x/ansi reads escape sequences with (its `parser`
//! package): a transition table from a state and a byte to the next state
//! and what to do with the byte, and a parser built on it that hands back
//! the sequences it finds, for the parts here that read styles and links
//! out of text.

/// The parser's states.
pub(crate) const GROUND: u8 = 0;
pub(crate) const CSI_ENTRY: u8 = 1;
pub(crate) const CSI_INTERMEDIATE: u8 = 2;
pub(crate) const CSI_PARAM: u8 = 3;
pub(crate) const DCS_ENTRY: u8 = 4;
pub(crate) const DCS_INTERMEDIATE: u8 = 5;
pub(crate) const DCS_PARAM: u8 = 6;
pub(crate) const DCS_STRING: u8 = 7;
pub(crate) const ESCAPE: u8 = 8;
pub(crate) const ESCAPE_INTERMEDIATE: u8 = 9;
pub(crate) const OSC_STRING: u8 = 10;
pub(crate) const SOS_STRING: u8 = 11;
pub(crate) const PM_STRING: u8 = 12;
pub(crate) const APC_STRING: u8 = 13;
pub(crate) const UTF8: u8 = 14;

/// What the parser does with a byte.
pub(crate) const NONE: u8 = 0;
pub(crate) const CLEAR: u8 = 1;
pub(crate) const COLLECT: u8 = 2;
pub(crate) const PREFIX: u8 = 3;
pub(crate) const DISPATCH: u8 = 4;
pub(crate) const EXECUTE: u8 = 5;
pub(crate) const START: u8 = 6;
pub(crate) const PUT: u8 = 7;
pub(crate) const PARAM: u8 = 8;
pub(crate) const PRINT: u8 = 9;
pub(crate) const IGNORE: u8 = NONE;

const ACTION_SHIFT: u8 = 4;
const STATE_MASK: u8 = 15;
const INDEX_STATE_SHIFT: usize = 8;
const TABLE_SIZE: usize = 4096;

/// The transition table, as x/ansi generates it.
static TABLE: [u8; TABLE_SIZE] = generate();

const fn add_one(t: &mut [u8; TABLE_SIZE], code: u8, state: u8, action: u8, next: u8) {
    let idx = (state as usize) << INDEX_STATE_SHIFT | code as usize;
    t[idx] = action << ACTION_SHIFT | next;
}

const fn add_range(t: &mut [u8; TABLE_SIZE], start: u8, end: u8, state: u8, action: u8, next: u8) {
    let mut i = start as usize;
    while i <= end as usize {
        add_one(t, i as u8, state, action, next);
        i += 1;
    }
}

const fn generate() -> [u8; TABLE_SIZE] {
    let mut t = [NONE << ACTION_SHIFT | GROUND; TABLE_SIZE];

    // Anywhere.
    let mut state = GROUND;
    while state <= UTF8 {
        add_one(&mut t, 0x18, state, EXECUTE, GROUND);
        add_one(&mut t, 0x1a, state, EXECUTE, GROUND);
        add_one(&mut t, 0x99, state, EXECUTE, GROUND);
        add_one(&mut t, 0x9a, state, EXECUTE, GROUND);
        add_range(&mut t, 0x80, 0x8F, state, EXECUTE, GROUND);
        add_range(&mut t, 0x90, 0x97, state, EXECUTE, GROUND);
        add_one(&mut t, 0x9C, state, EXECUTE, GROUND);
        // Anywhere -> Escape
        add_one(&mut t, 0x1B, state, CLEAR, ESCAPE);
        // Anywhere -> SOS, PM, APC
        add_one(&mut t, 0x98, state, START, SOS_STRING);
        add_one(&mut t, 0x9E, state, START, PM_STRING);
        add_one(&mut t, 0x9F, state, START, APC_STRING);
        // Anywhere -> CSI, DCS, OSC
        add_one(&mut t, 0x9B, state, CLEAR, CSI_ENTRY);
        add_one(&mut t, 0x90, state, CLEAR, DCS_ENTRY);
        add_one(&mut t, 0x9D, state, START, OSC_STRING);
        // Anywhere -> UTF-8
        add_range(&mut t, 0xC2, 0xDF, state, COLLECT, UTF8);
        add_range(&mut t, 0xE0, 0xEF, state, COLLECT, UTF8);
        add_range(&mut t, 0xF0, 0xF4, state, COLLECT, UTF8);
        state += 1;
    }

    // Ground
    add_range(&mut t, 0x00, 0x17, GROUND, EXECUTE, GROUND);
    add_one(&mut t, 0x19, GROUND, EXECUTE, GROUND);
    add_range(&mut t, 0x1C, 0x1F, GROUND, EXECUTE, GROUND);
    add_range(&mut t, 0x20, 0x7E, GROUND, PRINT, GROUND);
    add_one(&mut t, 0x7F, GROUND, EXECUTE, GROUND);

    // EscapeIntermediate
    add_range(&mut t, 0x00, 0x17, ESCAPE_INTERMEDIATE, EXECUTE, ESCAPE_INTERMEDIATE);
    add_one(&mut t, 0x19, ESCAPE_INTERMEDIATE, EXECUTE, ESCAPE_INTERMEDIATE);
    add_range(&mut t, 0x1C, 0x1F, ESCAPE_INTERMEDIATE, EXECUTE, ESCAPE_INTERMEDIATE);
    add_range(&mut t, 0x20, 0x2F, ESCAPE_INTERMEDIATE, COLLECT, ESCAPE_INTERMEDIATE);
    add_one(&mut t, 0x7F, ESCAPE_INTERMEDIATE, IGNORE, ESCAPE_INTERMEDIATE);
    add_range(&mut t, 0x30, 0x7E, ESCAPE_INTERMEDIATE, DISPATCH, GROUND);

    // Escape
    add_range(&mut t, 0x00, 0x17, ESCAPE, EXECUTE, ESCAPE);
    add_one(&mut t, 0x19, ESCAPE, EXECUTE, ESCAPE);
    add_range(&mut t, 0x1C, 0x1F, ESCAPE, EXECUTE, ESCAPE);
    add_one(&mut t, 0x7F, ESCAPE, IGNORE, ESCAPE);
    add_range(&mut t, 0x30, 0x4F, ESCAPE, DISPATCH, GROUND);
    add_range(&mut t, 0x51, 0x57, ESCAPE, DISPATCH, GROUND);
    add_one(&mut t, 0x59, ESCAPE, DISPATCH, GROUND);
    add_one(&mut t, 0x5A, ESCAPE, DISPATCH, GROUND);
    add_one(&mut t, 0x5C, ESCAPE, DISPATCH, GROUND);
    add_range(&mut t, 0x60, 0x7E, ESCAPE, DISPATCH, GROUND);
    // Escape -> Intermediate
    add_range(&mut t, 0x20, 0x2F, ESCAPE, COLLECT, ESCAPE_INTERMEDIATE);
    // Escape -> SOS, PM, APC
    add_one(&mut t, b'X', ESCAPE, START, SOS_STRING);
    add_one(&mut t, b'^', ESCAPE, START, PM_STRING);
    add_one(&mut t, b'_', ESCAPE, START, APC_STRING);
    // Escape -> DCS, CSI, OSC
    add_one(&mut t, b'P', ESCAPE, CLEAR, DCS_ENTRY);
    add_one(&mut t, b'[', ESCAPE, CLEAR, CSI_ENTRY);
    add_one(&mut t, b']', ESCAPE, START, OSC_STRING);

    // SOS, PM, APC strings
    let mut state = SOS_STRING;
    while state <= APC_STRING {
        add_range(&mut t, 0x00, 0x17, state, PUT, state);
        add_one(&mut t, 0x19, state, PUT, state);
        add_range(&mut t, 0x1C, 0x1F, state, PUT, state);
        add_range(&mut t, 0x20, 0x7F, state, PUT, state);
        // ESC, ST, CAN, and SUB terminate the sequence
        add_one(&mut t, 0x1B, state, DISPATCH, ESCAPE);
        add_one(&mut t, 0x9C, state, DISPATCH, GROUND);
        add_one(&mut t, 0x18, state, IGNORE, GROUND);
        add_one(&mut t, 0x1A, state, IGNORE, GROUND);
        state += 1;
    }

    // DcsEntry
    add_range(&mut t, 0x00, 0x07, DCS_ENTRY, IGNORE, DCS_ENTRY);
    add_range(&mut t, 0x0E, 0x17, DCS_ENTRY, IGNORE, DCS_ENTRY);
    add_one(&mut t, 0x19, DCS_ENTRY, IGNORE, DCS_ENTRY);
    add_range(&mut t, 0x1C, 0x1F, DCS_ENTRY, IGNORE, DCS_ENTRY);
    add_one(&mut t, 0x7F, DCS_ENTRY, IGNORE, DCS_ENTRY);
    add_range(&mut t, 0x20, 0x2F, DCS_ENTRY, COLLECT, DCS_INTERMEDIATE);
    add_range(&mut t, 0x30, 0x3B, DCS_ENTRY, PARAM, DCS_PARAM);
    add_range(&mut t, 0x3C, 0x3F, DCS_ENTRY, PREFIX, DCS_PARAM);
    add_range(&mut t, 0x08, 0x0D, DCS_ENTRY, PUT, DCS_STRING);
    add_one(&mut t, 0x1B, DCS_ENTRY, PUT, DCS_STRING);
    add_range(&mut t, 0x40, 0x7E, DCS_ENTRY, START, DCS_STRING);

    // DcsIntermediate
    add_range(&mut t, 0x00, 0x17, DCS_INTERMEDIATE, IGNORE, DCS_INTERMEDIATE);
    add_one(&mut t, 0x19, DCS_INTERMEDIATE, IGNORE, DCS_INTERMEDIATE);
    add_range(&mut t, 0x1C, 0x1F, DCS_INTERMEDIATE, IGNORE, DCS_INTERMEDIATE);
    add_range(&mut t, 0x20, 0x2F, DCS_INTERMEDIATE, COLLECT, DCS_INTERMEDIATE);
    add_one(&mut t, 0x7F, DCS_INTERMEDIATE, IGNORE, DCS_INTERMEDIATE);
    add_range(&mut t, 0x30, 0x3F, DCS_INTERMEDIATE, START, DCS_STRING);
    add_range(&mut t, 0x40, 0x7E, DCS_INTERMEDIATE, START, DCS_STRING);

    // DcsParam
    add_range(&mut t, 0x00, 0x17, DCS_PARAM, IGNORE, DCS_PARAM);
    add_one(&mut t, 0x19, DCS_PARAM, IGNORE, DCS_PARAM);
    add_range(&mut t, 0x1C, 0x1F, DCS_PARAM, IGNORE, DCS_PARAM);
    add_range(&mut t, 0x30, 0x3B, DCS_PARAM, PARAM, DCS_PARAM);
    add_one(&mut t, 0x7F, DCS_PARAM, IGNORE, DCS_PARAM);
    add_range(&mut t, 0x3C, 0x3F, DCS_PARAM, IGNORE, DCS_PARAM);
    add_range(&mut t, 0x20, 0x2F, DCS_PARAM, COLLECT, DCS_INTERMEDIATE);
    add_range(&mut t, 0x40, 0x7E, DCS_PARAM, START, DCS_STRING);

    // DcsString
    add_range(&mut t, 0x00, 0x17, DCS_STRING, PUT, DCS_STRING);
    add_one(&mut t, 0x19, DCS_STRING, PUT, DCS_STRING);
    add_range(&mut t, 0x1C, 0x1F, DCS_STRING, PUT, DCS_STRING);
    add_range(&mut t, 0x20, 0x7E, DCS_STRING, PUT, DCS_STRING);
    add_one(&mut t, 0x7F, DCS_STRING, PUT, DCS_STRING);
    add_range(&mut t, 0x80, 0xFF, DCS_STRING, PUT, DCS_STRING);
    // ST, CAN, SUB, and ESC terminate the sequence
    add_one(&mut t, 0x1B, DCS_STRING, DISPATCH, ESCAPE);
    add_one(&mut t, 0x9C, DCS_STRING, DISPATCH, GROUND);
    add_one(&mut t, 0x18, DCS_STRING, IGNORE, GROUND);
    add_one(&mut t, 0x1A, DCS_STRING, IGNORE, GROUND);

    // CsiParam
    add_range(&mut t, 0x00, 0x17, CSI_PARAM, EXECUTE, CSI_PARAM);
    add_one(&mut t, 0x19, CSI_PARAM, EXECUTE, CSI_PARAM);
    add_range(&mut t, 0x1C, 0x1F, CSI_PARAM, EXECUTE, CSI_PARAM);
    add_range(&mut t, 0x30, 0x3B, CSI_PARAM, PARAM, CSI_PARAM);
    add_one(&mut t, 0x7F, CSI_PARAM, IGNORE, CSI_PARAM);
    add_range(&mut t, 0x3C, 0x3F, CSI_PARAM, IGNORE, CSI_PARAM);
    // CsiParam -> Ground
    add_range(&mut t, 0x40, 0x7E, CSI_PARAM, DISPATCH, GROUND);
    // CsiParam -> CsiIntermediate
    add_range(&mut t, 0x20, 0x2F, CSI_PARAM, COLLECT, CSI_INTERMEDIATE);

    // CsiIntermediate
    add_range(&mut t, 0x00, 0x17, CSI_INTERMEDIATE, EXECUTE, CSI_INTERMEDIATE);
    add_one(&mut t, 0x19, CSI_INTERMEDIATE, EXECUTE, CSI_INTERMEDIATE);
    add_range(&mut t, 0x1C, 0x1F, CSI_INTERMEDIATE, EXECUTE, CSI_INTERMEDIATE);
    add_range(&mut t, 0x20, 0x2F, CSI_INTERMEDIATE, COLLECT, CSI_INTERMEDIATE);
    add_one(&mut t, 0x7F, CSI_INTERMEDIATE, IGNORE, CSI_INTERMEDIATE);
    // CsiIntermediate -> Ground
    add_range(&mut t, 0x40, 0x7E, CSI_INTERMEDIATE, DISPATCH, GROUND);
    // CsiIntermediate -> CsiIgnore
    add_range(&mut t, 0x30, 0x3F, CSI_INTERMEDIATE, IGNORE, GROUND);

    // CsiEntry
    add_range(&mut t, 0x00, 0x17, CSI_ENTRY, EXECUTE, CSI_ENTRY);
    add_one(&mut t, 0x19, CSI_ENTRY, EXECUTE, CSI_ENTRY);
    add_range(&mut t, 0x1C, 0x1F, CSI_ENTRY, EXECUTE, CSI_ENTRY);
    add_one(&mut t, 0x7F, CSI_ENTRY, IGNORE, CSI_ENTRY);
    // CsiEntry -> Ground
    add_range(&mut t, 0x40, 0x7E, CSI_ENTRY, DISPATCH, GROUND);
    // CsiEntry -> CsiIntermediate
    add_range(&mut t, 0x20, 0x2F, CSI_ENTRY, COLLECT, CSI_INTERMEDIATE);
    // CsiEntry -> CsiParam
    add_range(&mut t, 0x30, 0x3B, CSI_ENTRY, PARAM, CSI_PARAM);
    add_range(&mut t, 0x3C, 0x3F, CSI_ENTRY, PREFIX, CSI_PARAM);

    // OscString
    add_range(&mut t, 0x00, 0x06, OSC_STRING, IGNORE, OSC_STRING);
    add_range(&mut t, 0x08, 0x17, OSC_STRING, IGNORE, OSC_STRING);
    add_one(&mut t, 0x19, OSC_STRING, IGNORE, OSC_STRING);
    add_range(&mut t, 0x1C, 0x1F, OSC_STRING, IGNORE, OSC_STRING);
    add_range(&mut t, 0x20, 0xFF, OSC_STRING, PUT, OSC_STRING);
    // ST, CAN, SUB, ESC, and BEL terminate the sequence
    add_one(&mut t, 0x1B, OSC_STRING, DISPATCH, ESCAPE);
    add_one(&mut t, 0x07, OSC_STRING, DISPATCH, GROUND);
    add_one(&mut t, 0x9C, OSC_STRING, DISPATCH, GROUND);
    add_one(&mut t, 0x18, OSC_STRING, IGNORE, GROUND);
    add_one(&mut t, 0x1A, OSC_STRING, IGNORE, GROUND);

    t
}

/// transition is where the parser goes from `state` on byte `code`, and
/// what it does with the byte.
#[inline]
pub(crate) fn transition(state: u8, code: u8) -> (u8, u8) {
    let v = TABLE[(state as usize) << INDEX_STATE_SHIFT | code as usize];
    (v & STATE_MASK, v >> ACTION_SHIFT)
}

/// utf8_byte_len is how many bytes the UTF-8 sequence starting with `b`
/// takes, or -1 when `b` doesn't start one.
pub(crate) fn utf8_byte_len(b: u8) -> i32 {
    match b {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => -1,
    }
}

/// Param is one parameter of a control sequence: its value, or none when
/// it was left out, and whether sub-parameters (after a colon) follow.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Param {
    value: Option<i32>,
    has_more: bool,
}

impl Param {
    /// param is the value, or `def` when it was left out.
    pub(crate) fn param(&self, def: i32) -> i32 {
        self.value.unwrap_or(def)
    }

    /// has_more says whether sub-parameters follow.
    pub(crate) fn has_more(&self) -> bool {
        self.has_more
    }
}

/// param is the i-th parameter's value (or `def`), whether it has more, and
/// whether it's there at all, as x/ansi's `Params.Param`.
pub(crate) fn param_at(params: &[Param], i: usize, def: i32) -> (i32, bool, bool) {
    match params.get(i) {
        Some(p) => (p.param(def), p.has_more, true),
        None => (def, false, false),
    }
}

/// Seq is a sequence the parser finished.
#[derive(Debug)]
pub(crate) enum Seq {
    /// A control sequence: its final byte, prefix (`<`…`?`) and
    /// intermediate byte (0 when there's none) and its parameters.
    Csi { final_byte: u8, prefix: u8, intermediate: u8, params: Vec<Param> },
    /// An operating system command: its number (-1 when it has none) and
    /// what follows the escape, the number included.
    Osc { cmd: i32, data: Vec<u8> },
}

/// Parser reads text a byte at a time and hands back the control sequences
/// and operating system commands it finishes, the way x/ansi's `Parser`
/// calls its handlers.
#[derive(Default)]
pub(crate) struct Parser {
    state: u8,
    utf8_left: i32,
    params: Vec<Param>,
    cur: Param,
    prefix: u8,
    intermediate: u8,
    data: Vec<u8>,
    osc_cmd: i32,
}

impl Parser {
    /// clear forgets the sequence collected so far.
    fn clear(&mut self) {
        self.params.clear();
        self.cur = Param::default();
        self.prefix = 0;
        self.intermediate = 0;
    }

    /// advance takes one more byte, and is the sequence it finishes, if any.
    pub(crate) fn advance(&mut self, b: u8) -> Option<Seq> {
        if self.state == UTF8 {
            self.utf8_left -= 1;
            if self.utf8_left <= 0 {
                self.state = GROUND;
            }
            return None;
        }
        let (state, action) = transition(self.state, b);
        if self.state != state && self.state == ESCAPE {
            self.clear();
        }
        let mut out = None;
        if !(b == 0x1b && self.state == ESCAPE) {
            out = self.perform(action, state, b);
        }
        self.state = state;
        out
    }

    fn perform(&mut self, action: u8, state: u8, b: u8) -> Option<Seq> {
        match action {
            CLEAR => self.clear(),
            PREFIX => self.prefix = b,
            COLLECT => {
                if state == UTF8 {
                    self.utf8_left = utf8_byte_len(b) - 1;
                } else {
                    self.intermediate = b;
                }
            }
            PARAM => {
                if b.is_ascii_digit() {
                    let v = self.cur.value.unwrap_or(0);
                    self.cur.value = Some(v.saturating_mul(10).saturating_add((b - b'0') as i32));
                }
                if b == b':' {
                    self.cur.has_more = true;
                }
                if b == b';' || b == b':' {
                    self.params.push(self.cur);
                    self.cur = Param::default();
                }
            }
            START => {
                self.data.clear();
                self.osc_cmd = -1;
            }
            PUT => {
                if self.state == OSC_STRING && b == b';' && self.osc_cmd == -1 {
                    self.parse_osc_cmd();
                }
                self.data.push(b);
            }
            DISPATCH => {
                if !self.params.is_empty() || self.cur.value.is_some() || self.cur.has_more {
                    self.params.push(self.cur);
                    self.cur = Param::default();
                }
                match self.state {
                    CSI_ENTRY | CSI_PARAM | CSI_INTERMEDIATE => {
                        return Some(Seq::Csi {
                            final_byte: b,
                            prefix: self.prefix,
                            intermediate: self.intermediate,
                            params: std::mem::take(&mut self.params),
                        });
                    }
                    OSC_STRING => {
                        if self.osc_cmd == -1 {
                            self.parse_osc_cmd();
                        }
                        return Some(Seq::Osc { cmd: self.osc_cmd, data: std::mem::take(&mut self.data) });
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        None
    }

    fn parse_osc_cmd(&mut self) {
        for &d in &self.data {
            if !d.is_ascii_digit() {
                break;
            }
            if self.osc_cmd == -1 {
                self.osc_cmd = 0;
            }
            self.osc_cmd = self.osc_cmd.saturating_mul(10).saturating_add((d - b'0') as i32);
        }
    }
}
