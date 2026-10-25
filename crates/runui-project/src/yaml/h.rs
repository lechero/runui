//! yamlh.go and yamlprivateh.go: the scanner's, parser's and emitter's
//! types, and the character classes they read bytes by. Some of libyaml's
//! definitions go unused, as they do in yaml.v3; they're kept as it has
//! them.
#![allow(dead_code)]

use std::collections::HashMap;

// The size of the input raw buffer.
pub(super) const INPUT_RAW_BUFFER_SIZE: usize = 512;

// The size of the input buffer.
// It should be possible to decode the whole raw buffer.
pub(super) const INPUT_BUFFER_SIZE: usize = INPUT_RAW_BUFFER_SIZE * 3;

/// The version directive data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct VersionDirective {
    pub major: i8, // The major version number.
    pub minor: i8, // The minor version number.
}

/// The tag directive data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct TagDirective {
    pub handle: Vec<u8>, // The tag handle.
    pub prefix: Vec<u8>, // The tag prefix.
}

/// The stream encoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Encoding {
    // Let the parser choose the encoding.
    #[default]
    Any,
    Utf8,    // The default UTF-8 encoding.
    Utf16le, // The UTF-16-LE encoding with BOM.
    Utf16be, // The UTF-16-BE encoding with BOM.
}

/// Many bad things could happen with the parser and emitter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ErrorType {
    // No error is produced.
    #[default]
    None,
    Reader,  // Cannot read or decode the input stream.
    Scanner, // Cannot scan the input stream.
    Parser,  // Cannot parse the input stream.
    Writer,  // Cannot write to the output stream.
    Emitter, // Cannot emit a YAML stream.
}

/// The pointer position.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Mark {
    pub index: i64,  // The position index.
    pub line: i64,   // The position line.
    pub column: i64, // The position column.
}

// Scalar styles. The event's style holds one of these, a sequence style or
// a mapping style, as Go's yaml_style_t does.
pub(super) const ANY_SCALAR_STYLE: i8 = 0;
pub(super) const PLAIN_SCALAR_STYLE: i8 = 1 << 1; // The plain scalar style.
pub(super) const SINGLE_QUOTED_SCALAR_STYLE: i8 = 1 << 2; // The single-quoted scalar style.
pub(super) const DOUBLE_QUOTED_SCALAR_STYLE: i8 = 1 << 3; // The double-quoted scalar style.
pub(super) const LITERAL_SCALAR_STYLE: i8 = 1 << 4; // The literal scalar style.
pub(super) const FOLDED_SCALAR_STYLE: i8 = 1 << 5; // The folded scalar style.

// Sequence styles.
pub(super) const BLOCK_SEQUENCE_STYLE: i8 = 1; // The block sequence style.
pub(super) const FLOW_SEQUENCE_STYLE: i8 = 2; // The flow sequence style.

// Mapping styles.
pub(super) const BLOCK_MAPPING_STYLE: i8 = 1; // The block mapping style.
pub(super) const FLOW_MAPPING_STYLE: i8 = 2; // The flow mapping style.

/// Token types.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum TokenType {
    // An empty token.
    #[default]
    NoToken,
    StreamStart, // A STREAM-START token.
    StreamEnd,   // A STREAM-END token.

    VersionDirective, // A VERSION-DIRECTIVE token.
    TagDirective,     // A TAG-DIRECTIVE token.
    DocumentStart,    // A DOCUMENT-START token.
    DocumentEnd,      // A DOCUMENT-END token.

    BlockSequenceStart, // A BLOCK-SEQUENCE-START token.
    BlockMappingStart,  // A BLOCK-SEQUENCE-END token.
    BlockEnd,           // A BLOCK-END token.

    FlowSequenceStart, // A FLOW-SEQUENCE-START token.
    FlowSequenceEnd,   // A FLOW-SEQUENCE-END token.
    FlowMappingStart,  // A FLOW-MAPPING-START token.
    FlowMappingEnd,    // A FLOW-MAPPING-END token.

    BlockEntry, // A BLOCK-ENTRY token.
    FlowEntry,  // A FLOW-ENTRY token.
    Key,        // A KEY token.
    Value,      // A VALUE token.

    Alias,  // An ALIAS token.
    Anchor, // An ANCHOR token.
    Tag,    // A TAG token.
    Scalar, // A SCALAR token.
}

/// The token structure.
#[derive(Clone, Debug, Default)]
pub(super) struct Token {
    // The token type.
    pub typ: TokenType,

    // The start/end of the token.
    pub start_mark: Mark,
    pub end_mark: Mark,

    // The stream encoding (for STREAM-START).
    pub encoding: Encoding,

    // The alias/anchor/scalar value or tag/tag directive handle
    // (for ALIAS, ANCHOR, SCALAR, TAG, TAG-DIRECTIVE).
    pub value: Vec<u8>,

    // The tag suffix (for TAG).
    pub suffix: Vec<u8>,

    // The tag directive prefix (for TAG-DIRECTIVE).
    pub prefix: Vec<u8>,

    // The scalar style (for SCALAR).
    pub style: i8,

    // The version directive major/minor (for VERSION-DIRECTIVE).
    pub major: i8,
    pub minor: i8,
}

/// Event types.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum EventType {
    // An empty event.
    #[default]
    NoEvent,
    StreamStart,   // A STREAM-START event.
    StreamEnd,     // A STREAM-END event.
    DocumentStart, // A DOCUMENT-START event.
    DocumentEnd,   // A DOCUMENT-END event.
    Alias,         // An ALIAS event.
    Scalar,        // A SCALAR event.
    SequenceStart, // A SEQUENCE-START event.
    SequenceEnd,   // A SEQUENCE-END event.
    MappingStart,  // A MAPPING-START event.
    MappingEnd,    // A MAPPING-END event.
    TailComment,
}

impl EventType {
    /// The event's name, as Go's eventStrings say it.
    pub fn name(self) -> &'static str {
        match self {
            EventType::NoEvent => "none",
            EventType::StreamStart => "stream start",
            EventType::StreamEnd => "stream end",
            EventType::DocumentStart => "document start",
            EventType::DocumentEnd => "document end",
            EventType::Alias => "alias",
            EventType::Scalar => "scalar",
            EventType::SequenceStart => "sequence start",
            EventType::SequenceEnd => "sequence end",
            EventType::MappingStart => "mapping start",
            EventType::MappingEnd => "mapping end",
            EventType::TailComment => "tail comment",
        }
    }
}

/// The event structure.
#[derive(Clone, Debug, Default)]
pub(super) struct Event {
    // The event type.
    pub typ: EventType,

    // The start and end of the event.
    pub start_mark: Mark,
    pub end_mark: Mark,

    // The document encoding (for STREAM-START).
    pub encoding: Encoding,

    // The version directive (for DOCUMENT-START).
    pub version_directive: Option<VersionDirective>,

    // The list of tag directives (for DOCUMENT-START).
    pub tag_directives: Vec<TagDirective>,

    // The comments
    pub head_comment: Vec<u8>,
    pub line_comment: Vec<u8>,
    pub foot_comment: Vec<u8>,
    pub tail_comment: Vec<u8>,

    // The anchor (for SCALAR, SEQUENCE-START, MAPPING-START, ALIAS); None
    // where Go's is nil.
    pub anchor: Option<Vec<u8>>,

    // The tag (for SCALAR, SEQUENCE-START, MAPPING-START).
    pub tag: Vec<u8>,

    // The scalar value (for SCALAR).
    pub value: Vec<u8>,

    // Is the document start/end indicator implicit, or the tag optional?
    // (for DOCUMENT-START, DOCUMENT-END, SEQUENCE-START, MAPPING-START, SCALAR).
    pub implicit: bool,

    // Is the tag optional for any non-plain style? (for SCALAR).
    pub quoted_implicit: bool,

    // The style (for SCALAR, SEQUENCE-START, MAPPING-START).
    pub style: i8,
}

/// This structure holds information about a potential simple key.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct SimpleKey {
    pub possible: bool,      // Is a simple key possible?
    pub required: bool,      // Is a simple key required?
    pub token_number: usize, // The number of the token.
    pub mark: Mark,          // The position mark.
}

/// The states of the parser.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ParserState {
    #[default]
    StreamStart,

    ImplicitDocumentStart,         // Expect the beginning of an implicit document.
    DocumentStart,                 // Expect DOCUMENT-START.
    DocumentContent,               // Expect the content of a document.
    DocumentEnd,                   // Expect DOCUMENT-END.
    BlockNode,                     // Expect a block node.
    BlockNodeOrIndentlessSequence, // Expect a block node or indentless sequence.
    FlowNode,                      // Expect a flow node.
    BlockSequenceFirstEntry,       // Expect the first entry of a block sequence.
    BlockSequenceEntry,            // Expect an entry of a block sequence.
    IndentlessSequenceEntry,       // Expect an entry of an indentless sequence.
    BlockMappingFirstKey,          // Expect the first key of a block mapping.
    BlockMappingKey,               // Expect a block mapping key.
    BlockMappingValue,             // Expect a block mapping value.
    FlowSequenceFirstEntry,        // Expect the first entry of a flow sequence.
    FlowSequenceEntry,             // Expect an entry of a flow sequence.
    FlowSequenceEntryMappingKey,   // Expect a key of an ordered mapping.
    FlowSequenceEntryMappingValue, // Expect a value of an ordered mapping.
    FlowSequenceEntryMappingEnd,   // Expect the and of an ordered mapping entry.
    FlowMappingFirstKey,           // Expect the first key of a flow mapping.
    FlowMappingKey,                // Expect a key of a flow mapping.
    FlowMappingValue,              // Expect a value of a flow mapping.
    FlowMappingEmptyValue,         // Expect an empty value of a flow mapping.
    End,                           // Expect nothing.
}

/// A comment the scanner found, waiting for the token it goes with.
#[derive(Clone, Debug, Default)]
pub(super) struct Comment {
    pub scan_mark: Mark,  // Position where scanning for comments started
    pub token_mark: Mark, // Position after which tokens will be associated with this comment
    pub start_mark: Mark, // Position of '#' comment mark
    pub end_mark: Mark,   // Position where comment terminated

    pub head: Vec<u8>,
    pub line: Vec<u8>,
    pub foot: Vec<u8>,
}

/// The parser structure: the reader's, the scanner's and the parser's state.
#[derive(Default)]
pub(super) struct Parser {
    // Error handling
    pub error: ErrorType, // Error type.
    pub problem: String,  // Error description.

    // The byte about which the problem occurred.
    pub problem_offset: i64,
    pub problem_value: i64,
    pub problem_mark: Mark,

    // The error context.
    pub context: String,
    pub context_mark: Mark,

    // Reader stuff
    pub input: Vec<u8>, // String input data.
    pub input_pos: usize,

    pub eof: bool, // EOF flag

    pub buffer: Vec<u8>,   // The working buffer: Go's slice, its length the slice's.
    pub buffer_pos: usize, // The current position of the buffer.

    pub unread: usize, // The number of unread characters in the buffer.

    pub newlines: usize, // The number of line breaks since last non-break/non-blank character

    pub raw_buffer: Vec<u8>,   // The raw buffer, at most INPUT_RAW_BUFFER_SIZE.
    pub raw_buffer_pos: usize, // The current position of the buffer.

    pub encoding: Encoding, // The input encoding.

    pub offset: i64, // The offset of the current position (in bytes).
    pub mark: Mark,  // The mark of the current position.

    // Comments
    pub head_comment: Vec<u8>, // The current head comments
    pub line_comment: Vec<u8>, // The current line comments
    pub foot_comment: Vec<u8>, // The current foot comments
    pub tail_comment: Vec<u8>, // Foot comment that happens at the end of a block.
    pub stem_comment: Vec<u8>, // Comment in item preceding a nested structure (list inside list item, etc)

    pub comments: Vec<Comment>, // The folded comments for all parsed tokens
    pub comments_head: usize,

    // Scanner stuff
    pub stream_start_produced: bool, // Have we started to scan the input stream?
    pub stream_end_produced: bool,   // Have we reached the end of the input stream?

    pub flow_level: i64, // The number of unclosed '[' and '{' indicators.

    pub tokens: Vec<Token>,    // The tokens queue.
    pub tokens_cap: usize,     // The queue's capacity, as Go's append grows it.
    pub tokens_head: usize,    // The head of the tokens queue.
    pub tokens_parsed: usize,  // The number of tokens fetched from the queue.
    pub token_available: bool, // Does the tokens queue contain a token ready for dequeueing.

    pub indent: i64,       // The current indentation level.
    pub indents: Vec<i64>, // The indentation levels stack.

    pub simple_key_allowed: bool,    // May a simple key occur at the current position?
    pub simple_keys: Vec<SimpleKey>, // The stack of simple keys.
    pub simple_keys_by_tok: HashMap<usize, usize>, // possible simple_key indexes indexed by token_number

    // Parser stuff
    pub state: ParserState,                // The current parser state.
    pub states: Vec<ParserState>,          // The parser states stack.
    pub marks: Vec<Mark>,                  // The stack of marks.
    pub tag_directives: Vec<TagDirective>, // The list of TAG directives.
}

/// The emitter states.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum EmitterState {
    // Expect STREAM-START.
    #[default]
    StreamStart,

    FirstDocumentStart,      // Expect the first DOCUMENT-START or STREAM-END.
    DocumentStart,           // Expect DOCUMENT-START or STREAM-END.
    DocumentContent,         // Expect the content of a document.
    DocumentEnd,             // Expect DOCUMENT-END.
    FlowSequenceFirstItem,   // Expect the first item of a flow sequence.
    FlowSequenceTrailItem,   // Expect the next item of a flow sequence, with the comma already written out
    FlowSequenceItem,        // Expect an item of a flow sequence.
    FlowMappingFirstKey,     // Expect the first key of a flow mapping.
    FlowMappingTrailKey,     // Expect the next key of a flow mapping, with the comma already written out
    FlowMappingKey,          // Expect a key of a flow mapping.
    FlowMappingSimpleValue,  // Expect a value for a simple key of a flow mapping.
    FlowMappingValue,        // Expect a value of a flow mapping.
    BlockSequenceFirstItem,  // Expect the first item of a block sequence.
    BlockSequenceItem,       // Expect an item of a block sequence.
    BlockMappingFirstKey,    // Expect the first key of a block mapping.
    BlockMappingKey,         // Expect the key of a block mapping.
    BlockMappingSimpleValue, // Expect a value for a simple key of a block mapping.
    BlockMappingValue,       // Expect a value of a block mapping.
    End,                     // Expect nothing.
}

/// The line break the emitter writes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Break {
    // Let the parser choose the break type.
    #[default]
    Any,
    Cr,   // Use CR for line breaks (Mac style).
    Ln,   // Use LN for line breaks (Unix style).
    Crln, // Use CR LN for line breaks (DOS style).
}

/// The emitter's analysis of an anchor.
#[derive(Clone, Debug, Default)]
pub(super) struct AnchorData {
    pub anchor: Option<Vec<u8>>, // The anchor value; None where Go's is nil.
    pub alias: bool,             // Is it an alias?
}

/// The emitter's analysis of a tag.
#[derive(Clone, Debug, Default)]
pub(super) struct TagData {
    pub handle: Vec<u8>, // The tag handle.
    pub suffix: Vec<u8>, // The tag suffix.
}

/// The emitter's analysis of a scalar.
#[derive(Clone, Debug, Default)]
pub(super) struct ScalarData {
    pub value: Vec<u8>,              // The scalar value.
    pub multiline: bool,             // Does the scalar contain line breaks?
    pub flow_plain_allowed: bool,    // Can the scalar be expessed in the flow plain style?
    pub block_plain_allowed: bool,   // Can the scalar be expressed in the block plain style?
    pub single_quoted_allowed: bool, // Can the scalar be expressed in the single quoted style?
    pub block_allowed: bool,         // Can the scalar be expressed in the literal or folded styles?
    pub style: i8,                   // The output style.
}

/// The emitter structure.
#[derive(Default)]
pub(super) struct Emitter {
    // Error handling
    pub error: ErrorType, // Error type.
    pub problem: String,  // Error description.

    // Writer stuff: what's written, whole (Go buffers and flushes it).
    pub output: Vec<u8>,

    pub encoding: Encoding, // The stream encoding.

    // Emitter stuff
    pub canonical: bool,   // If the output is in the canonical style?
    pub best_indent: i64,  // The number of indentation spaces.
    pub best_width: i64,   // The preferred width of the output lines.
    pub unicode: bool,     // Allow unescaped non-ASCII characters?
    pub line_break: Break, // The preferred line break.

    pub state: EmitterState,       // The current emitter state.
    pub states: Vec<EmitterState>, // The stack of states.

    pub events: Vec<Event>, // The event queue.
    pub events_head: usize, // The head of the event queue.

    pub indents: Vec<i64>, // The stack of indentation levels.

    pub tag_directives: Vec<TagDirective>, // The list of tag directives.

    pub indent: i64, // The current indentation level.

    pub compact_sequence_indent: bool, // Is '- ' is considered part of the indentation for sequence elements?

    pub flow_level: i64, // The current flow level.

    pub root_context: bool,       // Is it the document root context?
    pub sequence_context: bool,   // Is it a sequence context?
    pub mapping_context: bool,    // Is it a mapping context?
    pub simple_key_context: bool, // Is it a simple mapping key context?

    pub line: i64,        // The current line.
    pub column: i64,      // The current column.
    pub whitespace: bool, // If the last character was a whitespace?
    pub indention: bool,  // If the last character was an indentation character (' ', '-', '?', ':')?
    pub open_ended: bool, // If an explicit document end is required?

    pub space_above: bool, // Is there's an empty line above?
    pub foot_indent: i64,  // The indent used to write the foot comment above, or -1 if none.

    // Anchor analysis.
    pub anchor_data: AnchorData,

    // Tag analysis.
    pub tag_data: TagData,

    // Scalar analysis.
    pub scalar_data: ScalarData,

    // Comments
    pub head_comment: Vec<u8>,
    pub line_comment: Vec<u8>,
    pub foot_comment: Vec<u8>,
    pub tail_comment: Vec<u8>,

    pub key_line_comment: Vec<u8>,

    // Dumper stuff
    pub opened: bool, // If the stream was already opened?
    pub closed: bool, // If the stream was already closed?
}

// The byte at i, or NUL past the end: what Go's buffer, padded with NULs,
// holds there.
#[inline]
pub(super) fn at(b: &[u8], i: usize) -> u8 {
    b.get(i).copied().unwrap_or(0)
}

// Check if the character at the specified position is an alphabetical
// character, a digit, '_', or '-'.
pub(super) fn is_alpha(b: &[u8], i: usize) -> bool {
    let c = at(b, i);
    c.is_ascii_digit() || c.is_ascii_uppercase() || c.is_ascii_lowercase() || c == b'_' || c == b'-'
}

// Check if the character at the specified position is a digit.
pub(super) fn is_digit(b: &[u8], i: usize) -> bool {
    at(b, i).is_ascii_digit()
}

// Get the value of a digit.
pub(super) fn as_digit(b: &[u8], i: usize) -> i64 {
    i64::from(at(b, i)) - i64::from(b'0')
}

// Check if the character at the specified position is a hex-digit.
pub(super) fn is_hex(b: &[u8], i: usize) -> bool {
    at(b, i).is_ascii_hexdigit()
}

// Get the value of a hex-digit.
pub(super) fn as_hex(b: &[u8], i: usize) -> i64 {
    let bi = at(b, i);
    if bi.is_ascii_uppercase() && bi <= b'F' {
        return i64::from(bi) - i64::from(b'A') + 10;
    }
    if (b'a'..=b'f').contains(&bi) {
        return i64::from(bi) - i64::from(b'a') + 10;
    }
    i64::from(bi) - i64::from(b'0')
}

// Check if the character is ASCII.
pub(super) fn is_ascii(b: &[u8], i: usize) -> bool {
    at(b, i) <= 0x7F
}

// Check if the character at the start of the buffer can be printed unescaped.
pub(super) fn is_printable(b: &[u8], i: usize) -> bool {
    let c = at(b, i);
    (c == 0x0A) // . == #x0A
        || (0x20..=0x7E).contains(&c) // #x20 <= . <= #x7E
        || (c == 0xC2 && at(b, i + 1) >= 0xA0) // #0xA0 <= . <= #xD7FF
        || (c > 0xC2 && c < 0xED)
        || (c == 0xED && at(b, i + 1) < 0xA0)
        || (c == 0xEE)
        || (c == 0xEF // #xE000 <= . <= #xFFFD
            && !(at(b, i + 1) == 0xBB && at(b, i + 2) == 0xBF) // && . != #xFEFF
            && !(at(b, i + 1) == 0xBF && (at(b, i + 2) == 0xBE || at(b, i + 2) == 0xBF)))
}

// Check if the character at the specified position is NUL.
pub(super) fn is_z(b: &[u8], i: usize) -> bool {
    at(b, i) == 0x00
}

// Check if the beginning of the buffer is a BOM. [Go] The beginning: i
// isn't looked at, as yaml.v3's doesn't.
pub(super) fn is_bom(b: &[u8], _i: usize) -> bool {
    at(b, 0) == 0xEF && at(b, 1) == 0xBB && at(b, 2) == 0xBF
}

// Check if the character at the specified position is space.
pub(super) fn is_space(b: &[u8], i: usize) -> bool {
    at(b, i) == b' '
}

// Check if the character at the specified position is tab.
pub(super) fn is_tab(b: &[u8], i: usize) -> bool {
    at(b, i) == b'\t'
}

// Check if the character at the specified position is blank (space or tab).
pub(super) fn is_blank(b: &[u8], i: usize) -> bool {
    let c = at(b, i);
    c == b' ' || c == b'\t'
}

// Check if the character at the specified position is a line break.
pub(super) fn is_break(b: &[u8], i: usize) -> bool {
    let c = at(b, i);
    c == b'\r' // CR (#xD)
        || c == b'\n' // LF (#xA)
        || c == 0xC2 && at(b, i + 1) == 0x85 // NEL (#x85)
        || c == 0xE2 && at(b, i + 1) == 0x80 && at(b, i + 2) == 0xA8 // LS (#x2028)
        || c == 0xE2 && at(b, i + 1) == 0x80 && at(b, i + 2) == 0xA9 // PS (#x2029)
}

pub(super) fn is_crlf(b: &[u8], i: usize) -> bool {
    at(b, i) == b'\r' && at(b, i + 1) == b'\n'
}

// Check if the character is a line break or NUL.
pub(super) fn is_breakz(b: &[u8], i: usize) -> bool {
    is_break(b, i) || is_z(b, i)
}

// Check if the character is a line break, space, or NUL.
pub(super) fn is_spacez(b: &[u8], i: usize) -> bool {
    is_space(b, i) || is_breakz(b, i)
}

// Check if the character is a line break, space, tab, or NUL.
pub(super) fn is_blankz(b: &[u8], i: usize) -> bool {
    is_blank(b, i) || is_breakz(b, i)
}

// Determine the width of the character.
pub(super) fn width(b: u8) -> usize {
    if b & 0x80 == 0x00 {
        return 1;
    }
    if b & 0xE0 == 0xC0 {
        return 2;
    }
    if b & 0xF0 == 0xE0 {
        return 3;
    }
    if b & 0xF8 == 0xF0 {
        return 4;
    }
    0
}

/// next_cap is the capacity Go's append gives a full slice of tokens
/// growing by one: the doubling and the allocator's size classes, for
/// 144-byte elements, as Go 1.27 does it. The scanner looks back at the
/// queue's last tokens, which the queue's compaction (when full) drops, so
/// when it's full matters.
pub(super) fn next_cap(cap: usize) -> usize {
    const CAPS: &[usize] = &[
        1, 2, 4, 8, 18, 37, 75, 151, 341, 625, 1024, 1479, 2048, 2787, 3697, 4835, 6257, 8021, 10240, 13027, 16497,
        20821, 26225, 32995, 41472, 52053, 65308, 81863, 102570, 128455, 160768, 201159, 251676, 314823, 393728,
        492373, 615708, 769877, 962560, 1203427, 1504483, 1880803, 2351217,
    ];
    match CAPS.iter().find(|&&c| c > cap) {
        Some(&c) => c,
        None => cap + cap / 4,
    }
}
