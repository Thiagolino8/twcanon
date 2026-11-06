// Port of Tailwind CSS 4.3.4 value-parser.ts. See LICENSE.tailwind.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Value {
    Word { value: String },
    Function { value: String, nodes: Vec<Value> },
    Separator { value: String },
}

impl Value {
    pub fn word(value: impl Into<String>) -> Self {
        Self::Word {
            value: value.into(),
        }
    }

    pub fn text_mut(&mut self) -> &mut String {
        match self {
            Self::Word { value } | Self::Function { value, .. } | Self::Separator { value } => {
                value
            }
        }
    }
}

pub fn to_css(ast: &[Value]) -> String {
    let mut output = String::new();
    write_css(ast, &mut output);
    output
}

fn write_css(ast: &[Value], output: &mut String) {
    for node in ast {
        match node {
            Value::Word { value } | Value::Separator { value } => output.push_str(value),
            Value::Function { value, nodes } => {
                output.push_str(value);
                output.push('(');
                write_css(nodes, output);
                output.push(')');
            }
        }
    }
}

struct FlatNode {
    value: Value,
    children: Vec<usize>,
}

fn append(
    value: Value,
    parent: Option<usize>,
    arena: &mut Vec<FlatNode>,
    roots: &mut Vec<usize>,
) -> usize {
    let index = arena.len();
    arena.push(FlatNode {
        value,
        children: Vec::new(),
    });
    match parent {
        Some(parent) => arena[parent].children.push(index),
        None => roots.push(index),
    }
    index
}

fn take_node(index: usize, arena: &mut [FlatNode]) -> Value {
    let mut node = std::mem::replace(&mut arena[index].value, Value::word(""));
    let children = std::mem::take(&mut arena[index].children);
    if let Value::Function { nodes, .. } = &mut node {
        *nodes = children.into_iter().map(|i| take_node(i, arena)).collect();
    }
    node
}

const fn is_separator(c: char) -> bool {
    matches!(c, ':' | ',' | '=' | '>' | '<' | '\n' | ' ' | '\t')
}

/// Preserve the upstream token shapes, including its recovery for malformed CSS.
pub fn parse(input: &str) -> Vec<Value> {
    let input = input.replace("\r\n", "\n");
    let chars: Vec<char> = input.chars().collect();
    let mut arena = Vec::new();
    let mut roots = Vec::new();
    let mut stack = Vec::new();
    let mut buffer = String::new();
    let mut i = 0;
    while i < chars.len() {
        let current = chars[i];
        match current {
            '\\' => {
                buffer.push(current);
                if let Some(next) = chars.get(i + 1) {
                    buffer.push(*next);
                } else {
                    buffer.push_str("undefined");
                }
                i += 1;
            }
            '/' => {
                if !buffer.is_empty() {
                    append(
                        Value::word(std::mem::take(&mut buffer)),
                        stack.last().copied(),
                        &mut arena,
                        &mut roots,
                    );
                }
                append(
                    Value::word("/"),
                    stack.last().copied(),
                    &mut arena,
                    &mut roots,
                );
            }
            c if is_separator(c) => {
                if !buffer.is_empty() {
                    append(
                        Value::word(std::mem::take(&mut buffer)),
                        stack.last().copied(),
                        &mut arena,
                        &mut roots,
                    );
                }
                let start = i;
                while i + 1 < chars.len() && is_separator(chars[i + 1]) {
                    i += 1;
                }
                append(
                    Value::Separator {
                        value: chars[start..=i].iter().collect(),
                    },
                    stack.last().copied(),
                    &mut arena,
                    &mut roots,
                );
            }
            '\'' | '"' => {
                let start = i;
                let mut j = i + 1;
                while j < chars.len() {
                    if chars[j] == '\\' {
                        j += 2;
                        continue;
                    }
                    if chars[j] == current {
                        i = j;
                        break;
                    }
                    j += 1;
                }
                buffer.extend(chars[start..=i].iter());
            }
            '(' => {
                let index = append(
                    Value::Function {
                        value: std::mem::take(&mut buffer),
                        nodes: Vec::new(),
                    },
                    stack.last().copied(),
                    &mut arena,
                    &mut roots,
                );
                stack.push(index);
            }
            ')' => {
                let tail = stack.pop();
                if !buffer.is_empty() {
                    let value = Value::word(std::mem::take(&mut buffer));
                    if let Some(parent) = tail {
                        append(value, Some(parent), &mut arena, &mut roots);
                    }
                }
            }
            _ => buffer.push(current),
        }
        i += 1;
    }
    if !buffer.is_empty() {
        append(Value::word(buffer), None, &mut arena, &mut roots);
    }
    roots
        .into_iter()
        .map(|i| take_node(i, &mut arena))
        .collect()
}
