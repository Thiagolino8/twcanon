// Port of Tailwind CSS 4.3.4 candidate.ts printing. See LICENSE.tailwind.
use crate::candidate::{Candidate, CandidateKind, Modifier, UtilityValue, Variant, VariantValue};
use crate::syntax::{
    is_js_whitespace,
    value::{self, Value},
};

fn text(node: &Value) -> &str {
    match node {
        Value::Word { value } | Value::Separator { value } | Value::Function { value, .. } => value,
    }
}

fn symbol(value: &str) -> bool {
    matches!(value, "~" | ">" | "+" | "-" | "*" | "/")
}

fn compact(nodes: &mut Vec<Value>) {
    let mut drop = vec![false; nodes.len()];
    for i in 0..nodes.len() {
        match &nodes[i] {
            Value::Word { value } if symbol(value) => {
                if i > 0
                    && i + 1 < nodes.len()
                    && matches!(&nodes[i - 1], Value::Separator { value } if value == " ")
                    && matches!(&nodes[i + 1], Value::Separator { value } if value == " ")
                    && (i < 2 || !symbol(text(&nodes[i - 2])))
                    && nodes.get(i + 2).is_none_or(|n| !symbol(text(n)))
                {
                    drop[i - 1] = true;
                    drop[i + 1] = true;
                }
            }
            Value::Separator { value }
                if !value.is_empty() && value.trim_matches(is_js_whitespace).is_empty() =>
            {
                if i == 0 || i + 1 == nodes.len() {
                    drop[i] = true;
                }
            }
            Value::Separator { value } if value.trim_matches(is_js_whitespace) == "," => {
                *nodes[i].text_mut() = ",".into();
            }
            Value::Function { value, .. }
                if value.starts_with("--")
                    && i > 0
                    && !matches!(&nodes[i - 1], Value::Separator { value } if value == ",")
                    && (i < 2 || symbol(text(&nodes[i - 2]))) =>
            {
                let original = std::mem::replace(&mut nodes[i], Value::word(""));
                nodes[i] = Value::Function {
                    value: String::new(),
                    nodes: vec![original],
                };
                continue;
            }
            _ => {}
        }
        if let Value::Function { nodes, .. } = &mut nodes[i] {
            compact(nodes);
        }
    }
    let mut i = 0;
    nodes.retain(|_| {
        let keep = !drop[i];
        i += 1;
        keep
    });
}

fn escape(value: &str) -> String {
    value.replace('_', "\\_").replace(' ', "_")
}

fn escape_nodes(nodes: &mut [Value]) {
    for node in nodes {
        match node {
            Value::Function { value, nodes } => {
                let url = value == "url" || value.ends_with("_url");
                *value = escape(value);
                if !url {
                    escape_nodes(nodes);
                }
            }
            Value::Separator { value } => *value = escape(value),
            Value::Word { value } if !value.starts_with("--") => *value = escape(value),
            _ => {}
        }
    }
}

pub fn arbitrary(input: &str) -> String {
    let mut ast = value::parse(input);
    arbitrary_ast(&mut ast)
}

pub(crate) fn arbitrary_ast(ast: &mut Vec<Value>) -> String {
    compact(ast);
    escape_nodes(ast);
    value::to_css(ast)
}

fn arbitrary_value(input: &str, data_type: Option<&str>) -> String {
    let ast = value::parse(input);
    let var = matches!(ast.as_slice(), [Value::Function { value, .. }] if value == "var");
    let mut output = String::from(if var { "(" } else { "[" });
    if let Some(data_type) = data_type.filter(|t| !t.is_empty()) {
        output.push_str(data_type);
        output.push(':');
    }
    let raw = if var {
        input.get(4..input.len().saturating_sub(1)).unwrap_or("")
    } else {
        input
    };
    output.push_str(&arbitrary(raw));
    output.push(if var { ')' } else { ']' });
    output
}

pub(crate) fn modifier(modifier: &Option<Modifier>) -> String {
    match modifier {
        None => String::new(),
        Some(Modifier::Named { value }) => format!("/{value}"),
        Some(Modifier::Arbitrary { value }) => format!("/{}", arbitrary_value(value, None)),
    }
}

pub fn variant(item: &Variant) -> String {
    match item {
        Variant::Static { root } => root.clone(),
        Variant::Arbitrary { selector, .. } => {
            let ast = value::parse(selector);
            let selector = match ast.as_slice() {
                [
                    Value::Word { value: a },
                    Value::Separator { value: b },
                    Value::Function { value: c, nodes },
                ] if a == "&" && b == ":" && c == "is" => value::to_css(nodes),
                _ => selector.clone(),
            };
            format!("[{}]", arbitrary(&selector))
        }
        Variant::Functional {
            root,
            value,
            modifier: m,
        } => {
            let mut result = root.clone();
            if let Some(value) = value {
                if root != "@" {
                    result.push('-');
                }
                match value {
                    VariantValue::Arbitrary { value } => {
                        result.push_str(&arbitrary_value(value, None))
                    }
                    VariantValue::Named { value } => result.push_str(value),
                }
            }
            result.push_str(&modifier(m));
            result
        }
        Variant::Compound {
            root,
            modifier: m,
            variant: nested,
        } => format!("{root}-{}{}", variant(nested), modifier(m)),
    }
}

pub fn candidate(candidate: &Candidate, prefix: Option<&str>) -> String {
    candidate_with_root(candidate, prefix).0
}

// Expose the printed root's byte range so a root-only search can reuse all
// variant, arbitrary-value and modifier formatting without textual matching.
pub(crate) fn candidate_with_root(
    candidate: &Candidate,
    prefix: Option<&str>,
) -> (String, Option<std::ops::Range<usize>>) {
    let mut parts = Vec::new();
    if let Some(prefix) = prefix.filter(|p| !p.is_empty()) {
        parts.push(prefix.to_owned());
    }
    parts.extend(candidate.variants.iter().rev().map(variant));
    let root_offset = parts.iter().map(|part| part.len() + 1).sum::<usize>();
    let root = match &candidate.kind {
        CandidateKind::Static { root } | CandidateKind::Functional { root, .. } => {
            Some(root_offset..root_offset + root.len())
        }
        _ => None,
    };
    let mut base = match &candidate.kind {
        CandidateKind::Static { root } => root.clone(),
        CandidateKind::Arbitrary {
            property,
            value,
            modifier: m,
        } => format!("[{property}:{}]{}", arbitrary(value), modifier(m)),
        CandidateKind::Functional {
            root,
            value,
            modifier: m,
        } => {
            let mut base = root.clone();
            if let Some(value) = value {
                base.push('-');
                match value {
                    UtilityValue::Arbitrary { value, data_type } => {
                        base.push_str(&arbitrary_value(value, data_type.as_deref()))
                    }
                    UtilityValue::Named { value, .. } => base.push_str(value),
                }
            }
            base.push_str(&modifier(m));
            base
        }
    };
    if candidate.important {
        base.push('!');
    }
    parts.push(base);
    (parts.join(":"), root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_template_matches_reprinting_with_variants_modifiers_and_unicode() {
        let values = [
            None,
            Some(UtilityValue::Named {
                value: "m-😀-3/4".into(),
                fraction: Some("3/4".into()),
            }),
            Some(UtilityValue::Arbitrary {
                data_type: Some("length".into()),
                value: "calc(var(--m) + var(--😀))".into(),
            }),
        ];
        let modifiers = [
            None,
            Some(Modifier::Named {
                value: "m-😀".into(),
            }),
            Some(Modifier::Arbitrary {
                value: "var(--m-😀)".into(),
            }),
        ];
        for value in &values {
            for modifier in &modifiers {
                for important in [false, true] {
                    let candidate = Candidate {
                        identity: Default::default(),
                        kind: CandidateKind::Functional {
                            root: "m".into(),
                            value: value.clone(),
                            modifier: modifier.clone(),
                        },
                        variants: vec![
                            Variant::Arbitrary {
                                selector: "&:is(.m, .😀)".into(),
                                relative: false,
                            },
                            Variant::Static {
                                root: "😀".into()
                            },
                        ],
                        important,
                        raw: String::new(),
                    };
                    for prefix in [None, Some(""), Some("😀")] {
                        let (template, Some(range)) = candidate_with_root(&candidate, prefix)
                        else {
                            panic!("Functional root missing");
                        };
                        for root in ["p", "-p", "m-long", "é", "😀", ""] {
                            let mut replaced = candidate.clone();
                            let CandidateKind::Functional { root: current, .. } =
                                &mut replaced.kind
                            else {
                                unreachable!();
                            };
                            *current = root.into();
                            let assembled = format!(
                                "{}{root}{}",
                                &template[..range.start],
                                &template[range.end..]
                            );
                            assert_eq!(assembled, super::candidate(&replaced, prefix));
                        }
                    }
                }
            }
        }
    }
}
