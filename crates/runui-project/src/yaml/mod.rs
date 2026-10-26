//! go.yaml.in/yaml/v3 (v3.0.5), the parts runui uses, ported: YAML read
//! into a tree of nodes (yaml.Unmarshal into a struct of Nodes), and a
//! node decoded into a task, an include, a service or its build (Node.
//! Decode). The scanner, parser and resolver are libyaml's as yaml.v3 has
//! them, so a Taskfile or a compose file reads as the Go build reads it,
//! and its errors read the same. Taken from aemui's aem-local, which ports
//! the encoder too.

mod base64;
mod h;
mod node;
mod parser;
mod reader;
// kept whole, as yaml.v3 has it, though runui resolves only what it decodes
#[allow(dead_code)]
mod resolve;
mod scanner;

pub(crate) use node::{
    Doc, Include, MAPPING_NODE, NodeId as NodeRef, SCALAR_NODE, SEQUENCE_NODE, Task, Tree, unmarshal_compose,
    unmarshal_doc, unmarshal_taskfile,
};

/// YamlError is an error yaml.v3 returns, in its words ("yaml: line 3:
/// mapping values are not allowed in this context").
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct YamlError(pub String);

impl std::fmt::Display for YamlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for YamlError {}
