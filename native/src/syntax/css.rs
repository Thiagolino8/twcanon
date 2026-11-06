// Port of Tailwind CSS 4.3.4 css-parser.ts, without source-map tracking. See LICENSE.tailwind.
use super::is_js_whitespace;
use crate::ast::AstNode;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CssSyntaxError {
    pub message: String,
    pub offset: usize,
}

impl std::fmt::Display for CssSyntaxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for CssSyntaxError {}

fn error(message: impl Into<String>, offset: usize) -> CssSyntaxError {
    CssSyntaxError {
        message: message.into(),
        offset,
    }
}
fn trim(input: &str) -> &str {
    input.trim_matches(is_js_whitespace)
}
fn slice(chars: &[char], start: usize, end: usize) -> String {
    chars[start.min(chars.len())..end.min(chars.len()).max(start.min(chars.len()))]
        .iter()
        .collect()
}

pub fn parse_at_rule(buffer: &str, nodes: Vec<AstNode>) -> AstNode {
    let mut split = None;
    let mut offset = 0;
    for (index, c) in buffer.char_indices() {
        if offset >= 5 && matches!(c, ' ' | '\t' | '(') {
            split = Some(index);
            break;
        }
        offset += c.len_utf16();
    }
    let (name, params) = split.map_or((buffer, ""), |i| (&buffer[..i], &buffer[i..]));
    AstNode::AtRule {
        name: trim(name).into(),
        params: trim(params).into(),
        nodes,
    }
}

fn is_escaped(value: &str, index: usize) -> bool {
    value[..index]
        .bytes()
        .rev()
        .take_while(|c| *c == b'\\')
        .count()
        % 2
        == 1
}

fn trim_value(value: &str) -> &str {
    let value = value.trim_start_matches(is_js_whitespace);
    let trimmed = value.trim_end_matches(is_js_whitespace);
    if trimmed.len() < value.len() && is_escaped(value, trimmed.len()) {
        &value[..trimmed.len() + value[trimmed.len()..].chars().next().unwrap().len_utf8()]
    } else {
        trimmed
    }
}

fn declaration(buffer: &str, colon: Option<usize>, value_end: usize) -> Option<AstNode> {
    let colon = colon?;
    let property = trim(&buffer[..colon]).to_owned();
    let value = trim_value(if value_end > colon {
        &buffer[colon + 1..value_end]
    } else {
        ""
    });
    if let Some(before) = value.strip_suffix("!important")
        && !is_escaped(value, before.len())
    {
        return Some(AstNode::Declaration {
            identity: Default::default(),
            property: property.into(),
            value: Some(trim_value(before).into()),
            important: true,
        });
    }
    Some(AstNode::Declaration {
        identity: Default::default(),
        property: property.into(),
        value: Some(trim_value(&buffer[colon + 1..]).into()),
        important: false,
    })
}

fn append(node: AstNode, parents: &mut [AstNode], roots: &mut Vec<AstNode>) {
    if let Some(nodes) = parents.last_mut().and_then(AstNode::children_mut) {
        nodes.push(node);
    } else {
        roots.push(node);
    }
}

fn parse_string(chars: &[char], start: usize) -> Result<usize, CssSyntaxError> {
    let quote = chars[start];
    let mut i = start + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            c if c == quote => return Ok(i),
            ';' if chars.get(i + 1) == Some(&'\n')
                || chars.get(i + 1..i + 3) == Some(&['\r', '\n']) =>
            {
                return Err(error(
                    format!("Unterminated string: {}{quote}", slice(chars, start, i + 1)),
                    start,
                ));
            }
            '\n' => {
                return Err(error(
                    format!("Unterminated string: {}{quote}", slice(chars, start, i)),
                    start,
                ));
            }
            '\r' if chars.get(i + 1) == Some(&'\n') => {
                return Err(error(
                    format!("Unterminated string: {}{quote}", slice(chars, start, i)),
                    start,
                ));
            }
            _ => {}
        }
        i += 1;
    }
    Ok(start)
}

pub fn parse(input: &str) -> Result<Vec<AstNode>, CssSyntaxError> {
    let mut chars: Vec<char> = input.chars().collect();
    if chars.first() == Some(&'\u{feff}') {
        chars[0] = ' ';
    }
    let mut roots = Vec::new();
    let mut licenses = Vec::new();
    let mut parents = Vec::new();
    let mut buffer = String::new();
    let mut brackets = Vec::new();
    let mut i = 0;
    let mut buffer_start = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\r' && chars.get(i + 1) == Some(&'\n') {
            i += 1;
            continue;
        }
        if c == '\\' {
            if buffer.is_empty() {
                buffer_start = i;
            }
            buffer.push(c);
            if let Some(c) = chars.get(i + 1) {
                buffer.push(*c);
            }
            i += 1;
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            let start = i;
            for j in i + 2..chars.len().saturating_sub(1) {
                if chars[j] == '*' && chars[j + 1] == '/' {
                    i = j + 1;
                    break;
                }
            }
            if chars.get(start + 2) == Some(&'!') && i + 1 >= start + 4 {
                licenses.push(AstNode::Comment {
                    value: slice(&chars, start + 2, i - 1),
                });
            }
        } else if matches!(c, '\'' | '"') {
            let end = parse_string(&chars, i)?;
            buffer.push_str(&slice(&chars, i, end + 1));
            i = end;
        } else if matches!(c, ' ' | '\n' | '\t')
            && (chars
                .get(i + 1)
                .is_some_and(|c| matches!(c, ' ' | '\n' | '\t'))
                || chars.get(i + 1..i + 3) == Some(&['\r', '\n']))
        {
        } else if c == '\n' {
            if !buffer.is_empty() && !buffer.ends_with([' ', '\n', '\t']) {
                buffer.push(' ');
            }
        } else if c == '-' && chars.get(i + 1) == Some(&'-') && buffer.is_empty() {
            let start = i;
            let mut closing = Vec::new();
            let mut colon = None;
            let mut value_end = 0;
            let mut j = i + 2;
            while j < chars.len() {
                let mut c = chars[j];
                if c == '\\' {
                    j += 1;
                } else if matches!(c, '\'' | '"') {
                    j = parse_string(&chars, j)?;
                } else if c == '/' && chars.get(j + 1) == Some(&'*') {
                    for k in j + 2..chars.len().saturating_sub(1) {
                        c = chars[k];
                        if c == '*' && chars[k + 1] == '/' {
                            j = k + 1;
                            break;
                        }
                    }
                    j += 1;
                    continue;
                } else if colon.is_none() && c == ':' {
                    colon = Some(j - start);
                } else if c == ';' && closing.is_empty() {
                    buffer = slice(&chars, start, j);
                    i = j;
                    break;
                } else if matches!(c, '(' | '[' | '{') {
                    closing.push(match c {
                        '(' => ')',
                        '[' => ']',
                        _ => '}',
                    });
                } else if c == '}' && closing.is_empty() {
                    i = j - 1;
                    buffer = slice(&chars, start, j);
                    break;
                } else if matches!(c, ')' | ']' | '}') && closing.last() == Some(&c) {
                    closing.pop();
                }
                if !matches!(c, ' ' | '\t' | '\n' | '\r') {
                    value_end = (j + 1).min(chars.len()) - start;
                }
                j += 1;
            }
            if buffer.is_empty() {
                buffer = slice(&chars, start, chars.len());
                i = chars.len();
            }
            let colon = colon.map(|c| chars[start..start + c].iter().map(|c| c.len_utf8()).sum());
            let value_end = chars[start..(start + value_end).min(chars.len())]
                .iter()
                .map(|c| c.len_utf8())
                .sum();
            let node = declaration(&buffer, colon, value_end)
                .ok_or_else(|| error("Invalid custom property, expected a value", start))?;
            append(node, &mut parents, &mut roots);
            buffer.clear();
        } else if c == ';' && buffer.starts_with('@') {
            append(parse_at_rule(&buffer, Vec::new()), &mut parents, &mut roots);
            buffer.clear();
        } else if c == ';' && brackets.last() != Some(&')') {
            if buffer.is_empty() {
                i += 1;
                continue;
            }
            let node = declaration(&buffer, buffer.find(':'), buffer.len()).ok_or_else(|| {
                error(
                    format!("Invalid declaration: `{}`", trim(&buffer)),
                    buffer_start,
                )
            })?;
            append(node, &mut parents, &mut roots);
            buffer.clear();
        } else if c == '{' && brackets.last() != Some(&')') {
            brackets.push('}');
            let selector = trim(&buffer);
            let node = if selector.starts_with('@') {
                parse_at_rule(selector, Vec::new())
            } else {
                AstNode::Rule {
                    selector: selector.into(),
                    nodes: Vec::new(),
                }
            };
            parents.push(node);
            buffer.clear();
        } else if c == '}' && brackets.last() != Some(&')') {
            if brackets.pop().is_none() {
                return Err(error("Missing opening {", i));
            }
            if !buffer.is_empty() {
                if buffer.starts_with('@') {
                    append(parse_at_rule(&buffer, Vec::new()), &mut parents, &mut roots);
                } else if !parents.is_empty() {
                    let node =
                        declaration(&buffer, buffer.find(':'), buffer.len()).ok_or_else(|| {
                            error(
                                format!("Invalid declaration: `{}`", trim(&buffer)),
                                buffer_start,
                            )
                        })?;
                    append(node, &mut parents, &mut roots);
                }
            }
            if let Some(parent) = parents.pop() {
                append(parent, &mut parents, &mut roots);
            }
            buffer.clear();
        } else if c == '(' {
            brackets.push(')');
            buffer.push('(');
        } else if c == ')' {
            if brackets.last() != Some(&')') {
                return Err(error("Missing opening (", i));
            }
            brackets.pop();
            buffer.push(')');
        } else {
            if buffer.is_empty() && matches!(c, ' ' | '\n' | '\t') {
                i += 1;
                continue;
            }
            if buffer.is_empty() {
                buffer_start = i;
            }
            buffer.push(c);
        }
        i += 1;
    }
    if buffer.starts_with('@') {
        roots.push(parse_at_rule(&buffer, Vec::new()));
    } else if !buffer.is_empty()
        && brackets.is_empty()
        && let Some(node) = declaration(&buffer, buffer.find(':'), buffer.len())
    {
        append(node, &mut parents, &mut roots);
    }
    if !brackets.is_empty()
        && let Some(parent) = parents.last()
    {
        let label = match parent {
            AstNode::Rule { selector, .. } => selector.clone(),
            AstNode::AtRule { name, params, .. } => format!("{name} {params}"),
            _ => unreachable!(),
        };
        return Err(error(
            format!("Missing closing }} at {label}"),
            buffer_start,
        ));
    }
    licenses.extend(roots);
    Ok(licenses)
}
