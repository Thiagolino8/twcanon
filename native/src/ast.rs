// Port of Tailwind CSS 4.3.4 ast.ts node shapes and serialization. See LICENSE.tailwind.
use compact_str::CompactString;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Internal reference identity, omitted from the public AST representation.
/// Cloning preserves shared JS object identity; independently constructed or
/// deserialized declarations receive fresh identities.
#[derive(Clone, Copy, Debug)]
pub struct DeclarationIdentity(pub(crate) usize);
impl Default for DeclarationIdentity {
    fn default() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ContextValue {
    String(String),
    Bool(bool),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AstNode {
    Rule {
        selector: String,
        nodes: Vec<AstNode>,
    },
    AtRule {
        name: String,
        params: String,
        nodes: Vec<AstNode>,
    },
    Declaration {
        #[serde(skip)]
        identity: DeclarationIdentity,
        property: CompactString,
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<String>,
        important: bool,
    },
    Comment {
        value: String,
    },
    Context {
        context: IndexMap<String, ContextValue>,
        nodes: Vec<AstNode>,
    },
    AtRoot {
        nodes: Vec<AstNode>,
    },
}

impl PartialEq for AstNode {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Rule {
                    selector: a,
                    nodes: b,
                },
                Self::Rule {
                    selector: c,
                    nodes: d,
                },
            ) => a == c && b == d,
            (
                Self::AtRule {
                    name: a,
                    params: b,
                    nodes: c,
                },
                Self::AtRule {
                    name: d,
                    params: e,
                    nodes: f,
                },
            ) => a == d && b == e && c == f,
            (
                Self::Declaration {
                    property: a,
                    value: b,
                    important: c,
                    ..
                },
                Self::Declaration {
                    property: d,
                    value: e,
                    important: f,
                    ..
                },
            ) => a == d && b == e && c == f,
            (Self::Comment { value: a }, Self::Comment { value: b }) => a == b,
            (
                Self::Context {
                    context: a,
                    nodes: b,
                },
                Self::Context {
                    context: c,
                    nodes: d,
                },
            ) => a == c && b == d,
            (Self::AtRoot { nodes: a }, Self::AtRoot { nodes: b }) => a == b,
            _ => false,
        }
    }
}
impl Eq for AstNode {}

pub fn rule(selector: &str, nodes: Vec<AstNode>) -> AstNode {
    if selector.starts_with('@') {
        crate::syntax::css::parse_at_rule(selector, nodes)
    } else {
        AstNode::Rule {
            selector: selector.into(),
            nodes,
        }
    }
}

impl AstNode {
    pub fn decl(property: impl Into<CompactString>, value: impl Into<String>) -> Self {
        Self::Declaration {
            identity: DeclarationIdentity::default(),
            property: property.into(),
            value: Some(value.into()),
            important: false,
        }
    }

    pub(crate) fn fresh_declarations(&mut self) {
        if let Self::Declaration { identity, .. } = self {
            *identity = DeclarationIdentity::default();
        }
        if let Some(nodes) = self.children_mut() {
            for node in nodes {
                node.fresh_declarations();
            }
        }
    }

    pub fn children_mut(&mut self) -> Option<&mut Vec<Self>> {
        match self {
            Self::Rule { nodes, .. }
            | Self::AtRule { nodes, .. }
            | Self::Context { nodes, .. }
            | Self::AtRoot { nodes } => Some(nodes),
            _ => None,
        }
    }
}

pub fn to_css(ast: &[AstNode]) -> String {
    fn write(nodes: &[AstNode], depth: usize, output: &mut String) {
        use std::fmt::Write;
        let indent = "  ".repeat(depth);
        for node in nodes {
            match node {
                AstNode::Declaration {
                    property,
                    value,
                    important,
                    ..
                } => writeln!(
                    output,
                    "{indent}{property}: {}{};",
                    value.as_deref().unwrap_or("undefined"),
                    if *important { " !important" } else { "" }
                )
                .unwrap(),
                AstNode::Rule { selector, nodes } => {
                    writeln!(output, "{indent}{selector} {{").unwrap();
                    write(nodes, depth + 1, output);
                    writeln!(output, "{indent}}}").unwrap();
                }
                AstNode::AtRule {
                    name,
                    params,
                    nodes,
                } => {
                    if nodes.is_empty() {
                        writeln!(output, "{indent}{name} {params};").unwrap();
                    } else {
                        writeln!(
                            output,
                            "{indent}{name}{} {{",
                            if params.is_empty() {
                                String::new()
                            } else {
                                format!(" {params}")
                            }
                        )
                        .unwrap();
                        write(nodes, depth + 1, output);
                        writeln!(output, "{indent}}}").unwrap();
                    }
                }
                AstNode::Comment { value } => writeln!(output, "{indent}/*{value}*/").unwrap(),
                AstNode::Context { .. } | AstNode::AtRoot { .. } => {}
            }
        }
    }
    let mut output = String::new();
    write(ast, 0, &mut output);
    output
}
