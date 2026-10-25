//! decode.go's parser (events into a tree of nodes, comments on them),
//! and its decoder for the types runui decodes a Taskfile and a compose
//! file into: their documents, a task, an include, a service and what it's
//! built from, the rest kept as nodes (yaml.Node), walked in order.

use std::collections::{HashMap, HashSet};

use super::YamlError;
use super::h::*;
use super::resolve::*;

// Kinds of node.
pub(crate) const DOCUMENT_NODE: u32 = 1;
pub(crate) const SEQUENCE_NODE: u32 = 1 << 1;
pub(crate) const MAPPING_NODE: u32 = 1 << 2;
pub(crate) const SCALAR_NODE: u32 = 1 << 3;
pub(crate) const ALIAS_NODE: u32 = 1 << 4;

// Node styles.
pub(crate) const TAGGED_STYLE: u32 = 1;
pub(crate) const DOUBLE_QUOTED_STYLE: u32 = 1 << 1;
pub(crate) const SINGLE_QUOTED_STYLE: u32 = 1 << 2;
pub(crate) const LITERAL_STYLE: u32 = 1 << 3;
pub(crate) const FOLDED_STYLE: u32 = 1 << 4;
pub(crate) const FLOW_STYLE: u32 = 1 << 5;

/// NodeId is a node in a Tree: what Go's *Node is.
pub(crate) type NodeId = usize;

/// Node is an element in the YAML document hierarchy, as yaml.v3's Node is:
/// its kind, style, tag, value, anchor, the node an alias points to, the
/// nodes it holds, its comments, and where it was.
#[derive(Clone, Debug, Default)]
pub(crate) struct Node {
    pub kind: u32,
    pub style: u32,
    pub tag: String,
    pub value: String,
    pub anchor: String,
    pub alias: Option<NodeId>,
    pub content: Vec<NodeId>,
    pub head_comment: String,
    pub line_comment: String,
    pub foot_comment: String,
    pub line: i64,
    pub column: i64,
}

impl Node {
    // IsZero returns whether the node has all of its fields unset.
    fn is_zero(&self) -> bool {
        self.kind == 0
            && self.style == 0
            && self.tag.is_empty()
            && self.value.is_empty()
            && self.anchor.is_empty()
            && self.alias.is_none()
            && self.content.is_empty()
            && self.head_comment.is_empty()
            && self.line_comment.is_empty()
            && self.foot_comment.is_empty()
            && self.line == 0
            && self.column == 0
    }

    fn indicated_string(&self) -> bool {
        self.kind == SCALAR_NODE
            && (short_tag(&self.tag) == STR_TAG
                || (self.tag.is_empty() || self.tag == "!")
                    && self.style & (SINGLE_QUOTED_STYLE | DOUBLE_QUOTED_STYLE | LITERAL_STYLE | FOLDED_STYLE) != 0)
    }
}

/// Tree holds the nodes of documents, each a NodeId: nodes point at one
/// another through it, the way Go's pointers do.
#[derive(Clone, Debug, Default)]
pub(crate) struct Tree {
    pub nodes: Vec<Node>,
}

impl Tree {
    pub fn add(&mut self, n: Node) -> NodeId {
        self.nodes.push(n);
        self.nodes.len() - 1
    }

    pub fn get(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }

    pub fn get_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id]
    }

    // ShortTag returns the short form of the YAML tag that indicates data type for
    // the node. If the Tag field isn't explicitly defined, one will be computed
    // based on the node properties.
    fn short_tag_of(&self, id: NodeId) -> String {
        let n = self.get(id);
        if n.indicated_string() {
            return STR_TAG.to_string();
        }
        if n.tag.is_empty() || n.tag == "!" {
            match n.kind {
                MAPPING_NODE => return MAP_TAG.to_string(),
                SEQUENCE_NODE => return SEQ_TAG.to_string(),
                ALIAS_NODE => {
                    if let Some(a) = n.alias {
                        return self.short_tag_of(a);
                    }
                }
                SCALAR_NODE => {
                    return resolve("", &n.value).map(|(t, _)| t).unwrap_or_default();
                }
                // Special case to make the zero value convenient.
                0 if n.is_zero() => return NULL_TAG.to_string(),
                _ => {}
            }
            return String::new();
        }
        short_tag(&n.tag)
    }
}

// go_lossy is bytes as text, each byte that doesn't start a character
// U+FFFD, as Go's utf8.DecodeRune reads them.
fn go_lossy(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let (r, n) = runui_core::golang::decode_rune(&b[i..]);
        out.push(r.unwrap_or('\u{FFFD}'));
        i += n.max(1);
    }
    out
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

// ----------------------------------------------------------------------------
// Parser, produces a node tree out of a libyaml event stream.

struct Composer<'t> {
    parser: Parser,
    event: Event,
    tree: &'t mut Tree,
    anchors: HashMap<String, NodeId>,
    done_init: bool,
    textless: bool,
}

impl<'t> Composer<'t> {
    // newParser: a parser of a document in memory, an empty one read as a
    // line break.
    fn new(b: &[u8], tree: &'t mut Tree) -> Composer<'t> {
        let b = if b.is_empty() { b"\n".to_vec() } else { b.to_vec() };
        Composer::from_reader(b, tree)
    }

    // newParserFromReader: a parser of what a reader gives, as it comes.
    fn from_reader(b: Vec<u8>, tree: &'t mut Tree) -> Composer<'t> {
        Composer {
            parser: Parser::new(b),
            event: Event::default(),
            tree,
            anchors: HashMap::new(),
            done_init: false,
            textless: false,
        }
    }

    fn init(&mut self) -> Result<(), YamlError> {
        if self.done_init {
            return Ok(());
        }
        self.anchors = HashMap::new();
        self.expect(EventType::StreamStart)?;
        self.done_init = true;
        Ok(())
    }

    // expect consumes an event from the event stream and
    // checks that it's of the expected type.
    fn expect(&mut self, e: EventType) -> Result<(), YamlError> {
        if self.event.typ == EventType::NoEvent && !self.parser.parse(&mut self.event) {
            return Err(self.fail());
        }
        if self.event.typ == EventType::StreamEnd {
            return Err(YamlError("yaml: attempted to go past the end of stream; corrupted value?".to_string()));
        }
        if self.event.typ != e {
            self.parser.problem = format!("expected {} event but got {}", e.name(), self.event.typ.name());
            return Err(self.fail());
        }
        self.event = Event::default();
        Ok(())
    }

    // peek peeks at the next event in the event stream,
    // puts the results into p.event and returns the event type.
    fn peek(&mut self) -> Result<EventType, YamlError> {
        if self.event.typ != EventType::NoEvent {
            return Ok(self.event.typ);
        }
        // It's curious choice from the underlying API to generally return a
        // positive result on success, but on this case return true in an error
        // scenario. This was the source of bugs in the past (issue #666).
        if !self.parser.parse(&mut self.event) || self.parser.error != ErrorType::None {
            return Err(self.fail());
        }
        Ok(self.event.typ)
    }

    fn fail(&self) -> YamlError {
        let p = &self.parser;
        let mut line = 0;
        if p.context_mark.line != 0 {
            line = p.context_mark.line;
            // Scanner errors don't iterate line before returning error
            if p.error == ErrorType::Scanner {
                line += 1;
            }
        } else if p.problem_mark.line != 0 {
            line = p.problem_mark.line;
            // Scanner errors don't iterate line before returning error
            if p.error == ErrorType::Scanner {
                line += 1;
            }
        }
        let place = if line != 0 { format!("line {line}: ") } else { String::new() };
        let msg = if !p.problem.is_empty() { p.problem.as_str() } else { "unknown problem parsing YAML content" };
        YamlError(format!("yaml: {place}{msg}"))
    }

    fn anchor(&mut self, n: NodeId, anchor: &Option<Vec<u8>>) {
        if let Some(anchor) = anchor {
            let a = text(anchor);
            self.tree.get_mut(n).anchor = a.clone();
            self.anchors.insert(a, n);
        }
    }

    fn parse(&mut self) -> Result<Option<NodeId>, YamlError> {
        self.init()?;
        match self.peek()? {
            EventType::Scalar => self.scalar().map(Some),
            EventType::Alias => self.alias().map(Some),
            EventType::MappingStart => self.mapping().map(Some),
            EventType::SequenceStart => self.sequence().map(Some),
            EventType::DocumentStart => self.document().map(Some),
            // Happens when attempting to decode an empty buffer.
            EventType::StreamEnd => Ok(None),
            EventType::TailComment => panic!("internal error: unexpected tail comment event (please report)"),
            t => panic!("internal error: attempted to parse unknown event (please report): {}", t.name()),
        }
    }

    fn node(&mut self, kind: u32, default_tag: &str, tag: &str, value: &str) -> NodeId {
        let mut style = 0;
        let tag = if !tag.is_empty() && tag != "!" {
            style = TAGGED_STYLE;
            short_tag(tag)
        } else if !default_tag.is_empty() {
            default_tag.to_string()
        } else if kind == SCALAR_NODE {
            resolve("", value).map(|(t, _)| t).unwrap_or_default()
        } else {
            String::new()
        };
        let mut n = Node { kind, tag, value: value.to_string(), style, ..Node::default() };
        if !self.textless {
            n.line = self.event.start_mark.line + 1;
            n.column = self.event.start_mark.column + 1;
            n.head_comment = text(&self.event.head_comment);
            n.line_comment = text(&self.event.line_comment);
            n.foot_comment = text(&self.event.foot_comment);
        }
        self.tree.add(n)
    }

    fn parse_child(&mut self, parent: NodeId) -> Result<NodeId, YamlError> {
        let child = self.parse()?.expect("a node, the stream not ended");
        self.tree.get_mut(parent).content.push(child);
        Ok(child)
    }

    fn document(&mut self) -> Result<NodeId, YamlError> {
        let n = self.node(DOCUMENT_NODE, "", "", "");
        self.expect(EventType::DocumentStart)?;
        self.parse_child(n)?;
        if self.peek()? == EventType::DocumentEnd {
            self.tree.get_mut(n).foot_comment = text(&self.event.foot_comment);
        }
        self.expect(EventType::DocumentEnd)?;
        Ok(n)
    }

    fn alias(&mut self) -> Result<NodeId, YamlError> {
        let anchor = text(self.event.anchor.as_deref().unwrap_or_default());
        let n = self.node(ALIAS_NODE, "", "", &anchor);
        let Some(&target) = self.anchors.get(&anchor) else {
            return Err(YamlError(format!("yaml: unknown anchor '{anchor}' referenced")));
        };
        self.tree.get_mut(n).alias = Some(target);
        self.expect(EventType::Alias)?;
        Ok(n)
    }

    fn scalar(&mut self) -> Result<NodeId, YamlError> {
        let parsed_style = self.event.style;
        let mut node_style = 0;
        if parsed_style & DOUBLE_QUOTED_SCALAR_STYLE != 0 {
            node_style = DOUBLE_QUOTED_STYLE;
        } else if parsed_style & SINGLE_QUOTED_SCALAR_STYLE != 0 {
            node_style = SINGLE_QUOTED_STYLE;
        } else if parsed_style & LITERAL_SCALAR_STYLE != 0 {
            node_style = LITERAL_STYLE;
        } else if parsed_style & FOLDED_SCALAR_STYLE != 0 {
            node_style = FOLDED_STYLE;
        }
        let node_value = text(&self.event.value);
        let node_tag = text(&self.event.tag);
        let mut default_tag = "";
        if node_style == 0 {
            if node_value == "<<" {
                default_tag = MERGE_TAG;
            }
        } else {
            default_tag = STR_TAG;
        }
        let n = self.node(SCALAR_NODE, default_tag, &node_tag, &node_value);
        self.tree.get_mut(n).style |= node_style;
        let anchor = self.event.anchor.clone();
        self.anchor(n, &anchor);
        self.expect(EventType::Scalar)?;
        Ok(n)
    }

    fn sequence(&mut self) -> Result<NodeId, YamlError> {
        let tag = text(&self.event.tag);
        let n = self.node(SEQUENCE_NODE, SEQ_TAG, &tag, "");
        if self.event.style & FLOW_SEQUENCE_STYLE != 0 {
            self.tree.get_mut(n).style |= FLOW_STYLE;
        }
        let anchor = self.event.anchor.clone();
        self.anchor(n, &anchor);
        self.expect(EventType::SequenceStart)?;
        while self.peek()? != EventType::SequenceEnd {
            self.parse_child(n)?;
        }
        let node = self.tree.get_mut(n);
        node.line_comment = text(&self.event.line_comment);
        node.foot_comment = text(&self.event.foot_comment);
        self.expect(EventType::SequenceEnd)?;
        Ok(n)
    }

    fn mapping(&mut self) -> Result<NodeId, YamlError> {
        let tag = text(&self.event.tag);
        let n = self.node(MAPPING_NODE, MAP_TAG, &tag, "");
        let mut block = true;
        if self.event.style & FLOW_MAPPING_STYLE != 0 {
            block = false;
            self.tree.get_mut(n).style |= FLOW_STYLE;
        }
        let anchor = self.event.anchor.clone();
        self.anchor(n, &anchor);
        self.expect(EventType::MappingStart)?;
        while self.peek()? != EventType::MappingEnd {
            let k = self.parse_child(n)?;
            if block && !self.tree.get(k).foot_comment.is_empty() {
                // Must be a foot comment for the prior value when being dedented.
                let content = &self.tree.get(n).content;
                if content.len() > 2 {
                    let prior = content[content.len() - 3];
                    let foot = std::mem::take(&mut self.tree.get_mut(k).foot_comment);
                    self.tree.get_mut(prior).foot_comment = foot;
                }
            }
            let v = self.parse_child(n)?;
            if self.tree.get(k).foot_comment.is_empty() && !self.tree.get(v).foot_comment.is_empty() {
                let foot = std::mem::take(&mut self.tree.get_mut(v).foot_comment);
                self.tree.get_mut(k).foot_comment = foot;
            }
            if self.peek()? == EventType::TailComment {
                if self.tree.get(k).foot_comment.is_empty() {
                    self.tree.get_mut(k).foot_comment = text(&self.event.foot_comment);
                }
                self.expect(EventType::TailComment)?;
            }
        }
        let line_comment = text(&self.event.line_comment);
        let foot_comment = text(&self.event.foot_comment);
        let node = self.tree.get_mut(n);
        node.line_comment = line_comment;
        node.foot_comment = foot_comment;
        if node.style & FLOW_STYLE == 0 && !node.foot_comment.is_empty() && node.content.len() > 1 {
            let last_key = node.content[node.content.len() - 2];
            let foot = std::mem::take(&mut node.foot_comment);
            self.tree.get_mut(last_key).foot_comment = foot;
        }
        self.expect(EventType::MappingEnd)?;
        Ok(n)
    }
}

/// unmarshal_doc is yaml.Unmarshal into a Node: the first document of data
/// read into tree, None for none.
pub(crate) fn unmarshal_doc(data: &[u8], tree: &mut Tree) -> Result<Option<NodeId>, YamlError> {
    Composer::new(data, tree).parse()
}

// ----------------------------------------------------------------------------
// Decoder, unmarshals a node into a provided value.

/// TaskfileDoc is what runui reads of a Taskfile: its includes and its
/// tasks, as nodes, walked in the order written.
#[derive(Clone, Debug, Default)]
pub(crate) struct TaskfileDoc {
    pub includes: Option<NodeId>,
    pub tasks: Option<NodeId>,
}

/// Task is the part of a task runui shows.
#[derive(Clone, Debug, Default)]
pub(crate) struct Task {
    pub desc: String,
    pub summary: String,
    pub internal: bool,
}

/// Include is an entry of a Taskfile's includes.
#[derive(Clone, Debug, Default)]
pub(crate) struct Include {
    pub taskfile: String,
    pub optional: bool,
    pub internal: bool,
    pub flatten: bool,
}

/// ComposeDoc is what runui reads of a compose file: its services.
#[derive(Clone, Debug, Default)]
pub(crate) struct ComposeDoc {
    pub services: Option<NodeId>,
}

/// Service is what a service is made from: its image, or what it's built
/// from, as a node.
#[derive(Clone, Debug, Default)]
pub(crate) struct Service {
    pub image: String,
    pub build: Option<NodeId>,
}

/// Build is a service's build when it's a map: its context.
#[derive(Clone, Debug, Default)]
pub(crate) struct Build {
    pub context: String,
}

/// GoValue is a value decoded into an interface{}, as Go's decoder makes
/// one: what a merge's keys are compared by.
#[derive(Clone, Debug)]
enum GoValue {
    Nil,
    Bool(bool),
    Int(i64),
    Uint(u64),
    Float(f64),
    Str(String),
    Time(String),
    // a map[string]interface{} (string_keys) or map[interface{}]interface{}
    Map { string_keys: bool, entries: Vec<(GoValue, GoValue)> },
    Slice(Vec<GoValue>),
}

/// HashKey is a GoValue a Go map can be keyed by.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum HashKey {
    Nil,
    Bool(bool),
    Int(i64),
    Uint(u64),
    Float(u64),
    Str(String),
    Time(String),
}

impl GoValue {
    fn from_resolved(r: Resolved) -> GoValue {
        match r {
            Resolved::Null => GoValue::Nil,
            Resolved::Bool(b) => GoValue::Bool(b),
            Resolved::Int(i) => GoValue::Int(i),
            Resolved::Uint(u) => GoValue::Uint(u),
            Resolved::Float(f) => GoValue::Float(f),
            Resolved::Str(s) => GoValue::Str(s),
            Resolved::Timestamp(t) => GoValue::Time(t),
        }
    }

    // the key a Go map keeps it by: an error for a map or a slice, which
    // Go can't hash; None for NaN, which no key equals.
    fn hash_key(&self) -> Result<Option<HashKey>, YamlError> {
        Ok(Some(match self {
            GoValue::Nil => HashKey::Nil,
            GoValue::Bool(b) => HashKey::Bool(*b),
            GoValue::Int(i) => HashKey::Int(*i),
            GoValue::Uint(u) => HashKey::Uint(*u),
            GoValue::Float(f) if f.is_nan() => return Ok(None),
            GoValue::Float(f) => HashKey::Float(if *f == 0.0 { 0 } else { f.to_bits() }),
            GoValue::Str(s) => HashKey::Str(s.clone()),
            GoValue::Time(t) => HashKey::Time(t.clone()),
            GoValue::Map { .. } | GoValue::Slice(_) => {
                return Err(YamlError(format!("yaml: runtime error: hash of unhashable type {}", self.type_name())));
            }
        }))
    }

    fn type_name(&self) -> &'static str {
        match self {
            GoValue::Map { string_keys: true, .. } => "map[string]interface {}",
            GoValue::Map { string_keys: false, .. } => "map[interface {}]interface {}",
            GoValue::Slice(_) => "[]interface {}",
            _ => "interface {}",
        }
    }

    // %#v of the value, near enough to Go's for an error message.
    fn go_syntax(&self) -> String {
        match self {
            GoValue::Nil => "interface {}(nil)".to_string(),
            GoValue::Bool(b) => b.to_string(),
            GoValue::Int(i) => i.to_string(),
            GoValue::Uint(u) => format!("0x{u:x}"),
            GoValue::Float(f) => {
                if f.is_finite() && (f.abs() >= 1e21 || (*f != 0.0 && f.abs() < 1e-4)) {
                    let s = format!("{f:e}");
                    match s.split_once('e') {
                        Some((m, e)) if e.starts_with('-') => format!("{m}e-{:0>2}", &e[1..]),
                        Some((m, e)) => format!("{m}e+{e:0>2}"),
                        None => s,
                    }
                } else if f.is_nan() {
                    "NaN".to_string()
                } else if f.is_infinite() {
                    if *f > 0.0 { "+Inf".to_string() } else { "-Inf".to_string() }
                } else {
                    f.to_string()
                }
            }
            GoValue::Str(s) | GoValue::Time(s) => runui_core::golang::quote(s),
            GoValue::Slice(items) => {
                let items: Vec<String> = items.iter().map(GoValue::go_syntax).collect();
                format!("[]interface {{}}{{{}}}", items.join(", "))
            }
            GoValue::Map { entries, .. } => {
                let mut items: Vec<String> =
                    entries.iter().map(|(k, v)| format!("{}:{}", k.go_syntax(), v.go_syntax())).collect();
                items.sort();
                format!("{}{{{}}}", self.type_name(), items.join(", "))
            }
        }
    }

    // Go's == on interface values: the same type and an equal value.
    fn go_equal(&self, other: &GoValue) -> bool {
        match (self.hash_key(), other.hash_key()) {
            (Ok(Some(a)), Ok(Some(b))) => a == b,
            _ => false,
        }
    }
}

/// Out is where a node is decoded to: a reflect.Value of one of the types
/// runui decodes into.
enum Out<'a> {
    Taskfile(&'a mut TaskfileDoc),
    Task(&'a mut Task),
    Include(&'a mut Include),
    Compose(&'a mut ComposeDoc),
    Service(&'a mut Service),
    Build(&'a mut Build),
    Node(&'a mut Option<NodeId>),
    Str(&'a mut String),
    Bool(&'a mut bool),
    Iface(&'a mut GoValue),
    Map(&'a mut GoValue),
}

impl Out<'_> {
    // the Go type's name, as reflect's Type.String says it
    fn type_name(&self) -> &'static str {
        match self {
            Out::Taskfile(_) => {
                r#"struct { Includes yaml.Node "yaml:\"includes\""; Tasks yaml.Node "yaml:\"tasks\"" }"#
            }
            Out::Task(_) => "project.task",
            Out::Include(_) => "project.include",
            Out::Compose(_) => r#"struct { Services yaml.Node "yaml:\"services\"" }"#,
            Out::Service(_) => r#"struct { Image string "yaml:\"image\""; Build yaml.Node "yaml:\"build\"" }"#,
            Out::Build(_) => r#"struct { Context string "yaml:\"context\"" }"#,
            Out::Node(_) => "yaml.Node",
            Out::Str(_) => "string",
            Out::Bool(_) => "bool",
            Out::Iface(_) => "interface {}",
            Out::Map(m) => m.type_name(),
        }
    }

    // a struct's fields, by their yaml names
    fn fields(&self) -> &'static [&'static str] {
        match self {
            Out::Taskfile(_) => &["includes", "tasks"],
            Out::Task(_) => &["desc", "summary", "internal"],
            Out::Include(_) => &["taskfile", "optional", "internal", "flatten"],
            Out::Compose(_) => &["services"],
            Out::Service(_) => &["image", "build"],
            Out::Build(_) => &["context"],
            _ => &[],
        }
    }

    fn is_struct(&self) -> bool {
        !self.fields().is_empty()
    }

    // the field a struct keeps at idx, as somewhere to decode to
    fn field(&mut self, idx: usize) -> Out<'_> {
        match self {
            Out::Taskfile(d) => match idx {
                0 => Out::Node(&mut d.includes),
                _ => Out::Node(&mut d.tasks),
            },
            Out::Task(t) => match idx {
                0 => Out::Str(&mut t.desc),
                1 => Out::Str(&mut t.summary),
                _ => Out::Bool(&mut t.internal),
            },
            Out::Include(i) => match idx {
                0 => Out::Str(&mut i.taskfile),
                1 => Out::Bool(&mut i.optional),
                2 => Out::Bool(&mut i.internal),
                _ => Out::Bool(&mut i.flatten),
            },
            Out::Compose(c) => Out::Node(&mut c.services),
            Out::Service(s) => match idx {
                0 => Out::Str(&mut s.image),
                _ => Out::Node(&mut s.build),
            },
            Out::Build(b) => Out::Str(&mut b.context),
            _ => unreachable!("a field of a struct"),
        }
    }
}

// 400,000 decode operations is ~500kb of dense object declarations, or
// ~5kb of dense object declarations with 10000% alias expansion
const ALIAS_RATIO_RANGE_LOW: i64 = 400000;

// 4,000,000 decode operations is ~5MB of dense object declarations, or
// ~4.5MB of dense object declarations with 10% alias expansion
const ALIAS_RATIO_RANGE_HIGH: i64 = 4000000;

// alias_ratio_range is the range over which we scale allowed alias ratios
const ALIAS_RATIO_RANGE: f64 = (ALIAS_RATIO_RANGE_HIGH - ALIAS_RATIO_RANGE_LOW) as f64;

fn allowed_alias_ratio(decode_count: i64) -> f64 {
    if decode_count <= ALIAS_RATIO_RANGE_LOW {
        // allow 99% to come from alias expansion for small-to-medium documents
        0.99
    } else if decode_count >= ALIAS_RATIO_RANGE_HIGH {
        // allow 10% to come from alias expansion for very large documents
        0.10
    } else {
        // scale smoothly from 99% down to 10% over the range.
        // this maps to 396,000 - 400,000 allowed alias-driven decodes over the range.
        // 400,000 decode operations is ~100MB of allocations in worst-case scenarios (single-item maps).
        0.99 - 0.89 * ((decode_count - ALIAS_RATIO_RANGE_LOW) as f64 / ALIAS_RATIO_RANGE)
    }
}

struct Decoder<'t> {
    tree: &'t Tree,
    aliases: HashSet<NodeId>,
    terrors: Vec<String>,

    unique_keys: bool,
    decode_count: i64,
    alias_count: i64,
    alias_depth: i64,

    merged_fields: Option<HashSet<HashKey>>,
}

type DResult<T> = Result<T, YamlError>;

impl<'t> Decoder<'t> {
    fn new(tree: &'t Tree) -> Decoder<'t> {
        Decoder {
            tree,
            aliases: HashSet::new(),
            terrors: Vec::new(),
            unique_keys: true,
            decode_count: 0,
            alias_count: 0,
            alias_depth: 0,
            merged_fields: None,
        }
    }

    fn type_error(&self) -> Option<YamlError> {
        if self.terrors.is_empty() {
            return None;
        }
        Some(YamlError(format!("yaml: unmarshal errors:\n  {}", self.terrors.join("\n  "))))
    }

    fn terror(&mut self, n: NodeId, tag: &str, out: &Out) {
        let node = self.tree.get(n);
        let tag = if !node.tag.is_empty() { node.tag.as_str() } else { tag };
        let mut value = node.value.clone();
        if tag != SEQ_TAG && tag != MAP_TAG {
            if value.len() > 10 {
                // [Go] seven bytes, which can end inside a character: the
                // bytes that don't make one are each U+FFFD here, as Go's
                // range and JSON read them
                value = format!(" `{}...`", go_lossy(&value.as_bytes()[..7]));
            } else {
                value = format!(" `{value}`");
            }
        }
        self.terrors.push(format!(
            "line {}: cannot unmarshal {}{} into {}",
            node.line,
            short_tag(tag),
            value,
            out.type_name()
        ));
    }

    fn unmarshal(&mut self, n: NodeId, out: &mut Out) -> DResult<bool> {
        self.decode_count += 1;
        if self.alias_depth > 0 {
            self.alias_count += 1;
        }
        if self.alias_count > 100
            && self.decode_count > 1000
            && self.alias_count as f64 / self.decode_count as f64 > allowed_alias_ratio(self.decode_count)
        {
            return Err(YamlError("yaml: document contains excessive aliasing".to_string()));
        }
        if let Out::Node(node) = out {
            **node = Some(n);
            return Ok(true);
        }
        let kind = self.tree.get(n).kind;
        match kind {
            DOCUMENT_NODE => return self.document(n, out),
            ALIAS_NODE => return self.alias(n, out),
            _ => {}
        }
        match kind {
            SCALAR_NODE => self.scalar(n, out),
            MAPPING_NODE => self.mapping(n, out),
            SEQUENCE_NODE => self.sequence(n, out),
            0 if self.tree.get(n).is_zero() => Ok(self.null(out)),
            _ => Err(YamlError(format!("yaml: cannot decode node with unknown kind {kind}"))),
        }
    }

    fn document(&mut self, n: NodeId, out: &mut Out) -> DResult<bool> {
        let content = &self.tree.get(n).content;
        if content.len() == 1 {
            let c = content[0];
            self.unmarshal(c, out)?;
            return Ok(true);
        }
        Ok(false)
    }

    fn alias(&mut self, n: NodeId, out: &mut Out) -> DResult<bool> {
        if self.aliases.contains(&n) {
            // TODO this could actually be allowed in some circumstances.
            return Err(YamlError(format!("yaml: anchor '{}' value contains itself", self.tree.get(n).value)));
        }
        self.aliases.insert(n);
        self.alias_depth += 1;
        let target = self.tree.get(n).alias.expect("an alias's node");
        let good = self.unmarshal(target, out)?;
        self.alias_depth -= 1;
        self.aliases.remove(&n);
        Ok(good)
    }

    fn null(&mut self, out: &mut Out) -> bool {
        match out {
            Out::Iface(v) | Out::Map(v) => {
                **v = GoValue::Nil;
                true
            }
            _ => false,
        }
    }

    fn scalar(&mut self, n: NodeId, out: &mut Out) -> DResult<bool> {
        let node = self.tree.get(n);
        let (tag, resolved) = if node.indicated_string() {
            (STR_TAG.to_string(), Resolved::Str(node.value.clone()))
        } else {
            let (tag, resolved) = resolve(&node.tag, &node.value)?;
            if tag == BINARY_TAG {
                let Resolved::Str(s) = &resolved else { unreachable!() };
                let (data, err) = super::base64::go_base64_decode(s);
                if err.is_some() {
                    return Err(YamlError("yaml: !!binary value contains invalid base64 data".to_string()));
                }
                (tag, Resolved::Str(String::from_utf8_lossy(&data).into_owned()))
            } else {
                (tag, resolved)
            }
        };
        if resolved == Resolved::Null {
            return Ok(self.null(out));
        }
        let value = node.value.clone();
        match out {
            Out::Str(s) => {
                **s = match resolved {
                    Resolved::Str(r) => r,
                    _ => value,
                };
                return Ok(true);
            }
            Out::Iface(v) => {
                **v = GoValue::from_resolved(resolved);
                return Ok(true);
            }
            Out::Bool(b) => match resolved {
                Resolved::Bool(r) => {
                    **b = r;
                    return Ok(true);
                }
                Resolved::Str(ref r) => {
                    // This offers some compatibility with the 1.1 spec (https://yaml.org/type/bool.html).
                    // It only works if explicitly attempting to unmarshal into a typed bool value.
                    match r.as_str() {
                        "y" | "Y" | "yes" | "Yes" | "YES" | "on" | "On" | "ON" => {
                            **b = true;
                            return Ok(true);
                        }
                        "n" | "N" | "no" | "No" | "NO" | "off" | "Off" | "OFF" => {
                            **b = false;
                            return Ok(true);
                        }
                        _ => {}
                    }
                }
                _ => {}
            },
            _ => {}
        }
        self.terror(n, &tag, out);
        Ok(false)
    }

    fn sequence(&mut self, n: NodeId, out: &mut Out) -> DResult<bool> {
        let content = self.tree.get(n).content.clone();
        match out {
            Out::Iface(v) => {
                // No type hints. Will have to use a generic sequence.
                let mut items = Vec::new();
                for c in content {
                    let mut e = GoValue::Nil;
                    if self.unmarshal(c, &mut Out::Iface(&mut e))? {
                        items.push(e);
                    }
                }
                **v = GoValue::Slice(items);
                Ok(true)
            }
            _ => {
                self.terror(n, SEQ_TAG, out);
                Ok(false)
            }
        }
    }

    fn mapping(&mut self, n: NodeId, out: &mut Out) -> DResult<bool> {
        let content = self.tree.get(n).content.clone();
        let l = content.len();
        if self.unique_keys {
            let nerrs = self.terrors.len();
            let mut i = 0;
            while i < l {
                let ni = self.tree.get(content[i]);
                let mut j = i + 2;
                while j < l {
                    let nj = self.tree.get(content[j]);
                    if ni.kind == nj.kind && ni.value == nj.value {
                        self.terrors.push(format!(
                            "line {}: mapping key {} already defined at line {}",
                            nj.line,
                            runui_core::golang::quote(&nj.value),
                            ni.line
                        ));
                    }
                    j += 2;
                }
                i += 2;
            }
            if self.terrors.len() > nerrs {
                return Ok(false);
            }
        }
        if out.is_struct() {
            return self.mapping_struct(n, out);
        }
        match out {
            Out::Map(_) => self.mapping_map(n, out),
            Out::Iface(v) => {
                let string_keys = self.is_string_map(n);
                **v = GoValue::Map { string_keys, entries: Vec::new() };
                self.mapping_map(n, &mut Out::Map(v))
            }
            _ => {
                self.terror(n, MAP_TAG, out);
                Ok(false)
            }
        }
    }

    fn is_string_map(&self, n: NodeId) -> bool {
        let node = self.tree.get(n);
        if node.kind != MAPPING_NODE {
            return false;
        }
        node.content.iter().step_by(2).all(|&k| {
            let t = self.tree.short_tag_of(k);
            t == STR_TAG || t == MERGE_TAG
        })
    }

    fn is_merge(&self, n: NodeId) -> bool {
        let n = self.tree.get(n);
        n.kind == SCALAR_NODE && n.value == "<<" && (n.tag.is_empty() || n.tag == "!" || short_tag(&n.tag) == MERGE_TAG)
    }

    // d.mapping for a map out: its keys and values decoded into interface{}
    // (the keys into strings for a map of strings).
    fn mapping_map(&mut self, n: NodeId, out: &mut Out) -> DResult<bool> {
        let Out::Map(map) = out else { unreachable!() };
        let content = self.tree.get(n).content.clone();
        let mut merged_fields = self.merged_fields.take();
        let mut merge_node = None;
        // [Go] the map was made before (for an interface{}, or the one a
        // merge goes into): it isn't new by Go's measure, out.IsNil()
        let map_is_new = false;
        let string_keys = matches!(&**map, GoValue::Map { string_keys: true, .. });
        let mut i = 0;
        while i < content.len() {
            if self.is_merge(content[i]) {
                merge_node = Some(content[i + 1]);
                i += 2;
                continue;
            }
            let mut k = GoValue::Nil;
            let ok = if string_keys {
                let mut s = String::new();
                let ok = self.unmarshal(content[i], &mut Out::Str(&mut s))?;
                k = GoValue::Str(s);
                ok
            } else {
                self.unmarshal(content[i], &mut Out::Iface(&mut k))?
            };
            if ok {
                if let Some(mf) = merged_fields.as_mut() {
                    // a NaN key is none: no key equals it
                    if let Some(key) = k.hash_key()? {
                        if mf.contains(&key) {
                            i += 2;
                            continue;
                        }
                        mf.insert(key);
                    }
                }
                if matches!(k, GoValue::Map { .. } | GoValue::Slice(_)) {
                    self.merged_fields = merged_fields;
                    return Err(YamlError(format!("yaml: invalid map key: {}", k.go_syntax())));
                }
                let mut e = GoValue::Nil;
                let value = content[i + 1];
                let decoded = self.unmarshal(value, &mut Out::Iface(&mut e))?;
                let GoValue::Map { entries, .. } = &mut **map else { unreachable!() };
                let at = entries.iter().position(|(ek, _)| ek.go_equal(&k));
                if decoded || self.tree.short_tag_of(value) == NULL_TAG && (map_is_new || at.is_none()) {
                    match at {
                        Some(at) => entries[at].1 = e,
                        None => entries.push((k, e)),
                    }
                }
            }
            i += 2;
        }
        self.merged_fields = merged_fields;
        if let Some(m) = merge_node {
            self.merge(n, m, out)?;
        }
        Ok(true)
    }

    fn mapping_struct(&mut self, n: NodeId, out: &mut Out) -> DResult<bool> {
        let fields = out.fields();
        let type_name = out.type_name();
        let content = self.tree.get(n).content.clone();

        let mut merged_fields = self.merged_fields.take();
        let mut merge_node = None;
        let mut done_fields = vec![false; fields.len()];
        let mut name = String::new();
        let mut i = 0;
        while i < content.len() {
            let ni = content[i];
            if self.is_merge(ni) {
                merge_node = Some(content[i + 1]);
                i += 2;
                continue;
            }
            if !self.unmarshal(ni, &mut Out::Str(&mut name))? {
                i += 2;
                continue;
            }
            let sname = name.clone();
            if let Some(mf) = merged_fields.as_mut() {
                let key = HashKey::Str(sname.clone());
                if mf.contains(&key) {
                    i += 2;
                    continue;
                }
                mf.insert(key);
            }
            let line = self.tree.get(ni).line;
            if let Some(idx) = fields.iter().position(|f| *f == sname) {
                if self.unique_keys {
                    if done_fields[idx] {
                        self.terrors.push(format!("line {line}: field {sname} already set in type {type_name}"));
                        i += 2;
                        continue;
                    }
                    done_fields[idx] = true;
                }
                let value = content[i + 1];
                self.unmarshal(value, &mut out.field(idx))?;
            }
            i += 2;
        }

        self.merged_fields = merged_fields;
        if let Some(m) = merge_node {
            self.merge(n, m, out)?;
        }
        Ok(true)
    }

    fn merge(&mut self, parent: NodeId, merge: NodeId, out: &mut Out) -> DResult<()> {
        let fresh = self.merged_fields.is_none();
        if fresh {
            self.merged_fields = Some(HashSet::new());
            let keys: Vec<NodeId> = self.tree.get(parent).content.iter().step_by(2).copied().collect();
            for key in keys {
                let mut k = GoValue::Nil;
                if self.unmarshal(key, &mut Out::Iface(&mut k))? {
                    let hk = k.hash_key()?;
                    if let (Some(hk), Some(mf)) = (hk, self.merged_fields.as_mut()) {
                        mf.insert(hk);
                    }
                }
            }
        }

        let fail_want_map = || YamlError("yaml: map merge requires map or sequence of maps as the value".to_string());
        let m = self.tree.get(merge);
        match m.kind {
            MAPPING_NODE => {
                self.unmarshal(merge, out)?;
            }
            ALIAS_NODE => {
                if let Some(a) = m.alias
                    && self.tree.get(a).kind != MAPPING_NODE
                {
                    return Err(fail_want_map());
                }
                self.unmarshal(merge, out)?;
            }
            SEQUENCE_NODE => {
                for ni in m.content.clone() {
                    let node = self.tree.get(ni);
                    if node.kind == ALIAS_NODE {
                        if let Some(a) = node.alias
                            && self.tree.get(a).kind != MAPPING_NODE
                        {
                            return Err(fail_want_map());
                        }
                    } else if node.kind != MAPPING_NODE {
                        return Err(fail_want_map());
                    }
                    self.unmarshal(ni, out)?;
                }
            }
            _ => return Err(fail_want_map()),
        }

        if fresh {
            self.merged_fields = None;
        }
        Ok(())
    }
}

/// Doc is a YAML document read as yaml.v3 reads one: its nodes.
pub(crate) struct Doc {
    pub tree: Tree,
}

impl Doc {
    /// get is a node of the document.
    pub fn get(&self, n: NodeId) -> &Node {
        self.tree.get(n)
    }

    /// pairs are a mapping's keys and values in the order written: the
    /// keys' values (Value, as written) and the values' nodes. Nothing for
    /// a node that isn't a mapping.
    pub fn pairs(&self, n: Option<NodeId>) -> Vec<(String, NodeId)> {
        let Some(n) = n else { return Vec::new() };
        let node = self.tree.get(n);
        if node.kind != MAPPING_NODE {
            return Vec::new();
        }
        node.content.as_chunks::<2>().0.iter().map(|[k, v]| (self.tree.get(*k).value.clone(), *v)).collect()
    }
}

fn decode_into(tree: &Tree, n: NodeId, out: &mut Out) -> Result<(), YamlError> {
    let mut d = Decoder::new(tree);
    d.unmarshal(n, out)?;
    match d.type_error() {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

/// unmarshal_taskfile is yaml.Unmarshal of a Taskfile into its includes
/// and tasks.
pub(crate) fn unmarshal_taskfile(data: &[u8]) -> Result<(Doc, TaskfileDoc), YamlError> {
    let mut tree = Tree::default();
    let root = unmarshal_doc(data, &mut tree)?;
    let mut doc = TaskfileDoc::default();
    if let Some(r) = root {
        decode_into(&tree, r, &mut Out::Taskfile(&mut doc))?;
    }
    Ok((Doc { tree }, doc))
}

/// unmarshal_compose is yaml.Unmarshal of a compose file into its
/// services.
pub(crate) fn unmarshal_compose(data: &[u8]) -> Result<(Doc, ComposeDoc), YamlError> {
    let mut tree = Tree::default();
    let root = unmarshal_doc(data, &mut tree)?;
    let mut doc = ComposeDoc::default();
    if let Some(r) = root {
        decode_into(&tree, r, &mut Out::Compose(&mut doc))?;
    }
    Ok((Doc { tree }, doc))
}

impl Doc {
    /// decode_task is Node.Decode into a task: what decoded, and the error.
    pub fn decode_task(&self, n: NodeId) -> (Task, Result<(), YamlError>) {
        let mut t = Task::default();
        let r = decode_into(&self.tree, n, &mut Out::Task(&mut t));
        (t, r)
    }

    /// decode_include is Node.Decode into an include.
    pub fn decode_include(&self, n: NodeId) -> (Include, Result<(), YamlError>) {
        let mut inc = Include::default();
        let r = decode_into(&self.tree, n, &mut Out::Include(&mut inc));
        (inc, r)
    }

    /// decode_service is Node.Decode into a service.
    pub fn decode_service(&self, n: NodeId) -> (Service, Result<(), YamlError>) {
        let mut s = Service::default();
        let r = decode_into(&self.tree, n, &mut Out::Service(&mut s));
        (s, r)
    }

    /// decode_build is Node.Decode into a service's build.
    pub fn decode_build(&self, n: NodeId) -> (Build, Result<(), YamlError>) {
        let mut b = Build::default();
        let r = decode_into(&self.tree, n, &mut Out::Build(&mut b));
        (b, r)
    }
}
