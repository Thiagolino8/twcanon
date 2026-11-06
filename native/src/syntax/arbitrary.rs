// Port of Tailwind CSS 4.3.4 utils/is-valid-arbitrary.ts. See LICENSE.tailwind.
use super::{
    math,
    value::{self, Value},
};

fn underscores(input: &str, preserve: bool) -> String {
    let mut output = String::new();
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'_') {
            chars.next();
            output.push('_');
        } else if c == '_' && !preserve {
            output.push(' ');
        } else {
            output.push(c);
        }
    }
    output
}

fn decode_nodes(nodes: &mut [Value]) {
    for node in nodes {
        match node {
            Value::Function { value, nodes } => {
                if value == "url" || value.ends_with("_url") {
                    *value = underscores(value, false);
                    continue;
                }
                let preserve_first = value == "var"
                    || value.ends_with("_var")
                    || value == "theme"
                    || value.ends_with("_theme");
                *value = underscores(value, false);
                for (i, child) in nodes.iter_mut().enumerate() {
                    if preserve_first
                        && i == 0
                        && let Value::Word { value } = child
                    {
                        *value = underscores(value, true);
                        continue;
                    }
                    decode_nodes(std::slice::from_mut(child));
                }
            }
            Value::Word { value } | Value::Separator { value } => {
                *value = underscores(value, false)
            }
        }
    }
}

pub fn decode(input: &str) -> String {
    if !input.contains('(') {
        return underscores(input, false);
    }
    let mut nodes = value::parse(input);
    decode_nodes(&mut nodes);
    math::add_whitespace(&value::to_css(&nodes))
}

pub fn decode_valid(input: &str) -> Option<String> {
    is_valid(input).then(|| decode(input))
}

pub fn is_valid(input: &str) -> bool {
    let mut stack = ['\0'; 256];
    let mut depth = 0;
    let mut chars = input.chars();
    while let Some(current) = chars.next() {
        match current {
            '\\' => {
                chars.next();
            }
            '\'' | '"' => {
                while let Some(next) = chars.next() {
                    if next == '\\' {
                        chars.next();
                    } else if next == current {
                        break;
                    }
                }
            }
            '(' | '[' => {
                if let Some(slot) = stack.get_mut(depth) {
                    *slot = if current == '(' { ')' } else { ']' };
                }
                depth += 1;
            }
            ')' | ']' | '}' => {
                if depth == 0 {
                    return false;
                }
                if stack.get(depth - 1) == Some(&current) {
                    depth -= 1;
                } else if current != '}' {
                    return false;
                }
            }
            ';' if depth == 0 => return false,
            _ => {}
        }
    }
    depth == 0
}
