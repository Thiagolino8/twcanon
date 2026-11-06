// Port of Tailwind CSS 4.3.4 attribute-selector-parser.ts. See LICENSE.tailwind.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attribute {
    pub attribute: String,
    pub operator: Option<String>,
    pub quote: Option<char>,
    pub value: Option<String>,
    pub sensitivity: Option<char>,
}

const fn whitespace(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
}

pub fn parse(input: &str) -> Option<Attribute> {
    let bytes = input.as_bytes();
    if !input.starts_with('[') || !input.ends_with(']') {
        return None;
    }
    let end = bytes.len() - 1;
    let mut i = 1;
    while i < end && whitespace(bytes[i]) {
        i += 1;
    }
    let start = i;
    while i < end {
        let c = bytes[i];
        if c == b'\\' {
            i += 1;
            if i < bytes.len() {
                i += input[i..].chars().next()?.len_utf8();
            }
        } else if c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_') || c >= 128 {
            i += input[i..].chars().next()?.len_utf8();
        } else {
            break;
        }
    }
    if i == start || i > end {
        return None;
    }
    let attribute = input[start..i].to_owned();
    while i < end && whitespace(bytes[i]) {
        i += 1;
    }
    let mut result = Attribute {
        attribute,
        operator: None,
        quote: None,
        value: None,
        sensitivity: None,
    };
    if i == end {
        return Some(result);
    }
    if bytes[i] == b'=' {
        result.operator = Some("=".into());
        i += 1;
    } else if matches!(bytes[i], b'~' | b'|' | b'^' | b'$' | b'*')
        && bytes.get(i + 1) == Some(&b'=')
    {
        result.operator = Some(input[i..i + 2].to_owned());
        i += 2;
    } else {
        return None;
    }
    while i < end && whitespace(bytes[i]) {
        i += 1;
    }
    if i == end {
        return None;
    }
    if matches!(bytes[i], b'\'' | b'"') {
        let quote = bytes[i];
        result.quote = Some(char::from(quote));
        i += 1;
        let start = i;
        let mut j = i;
        // Upstream keeps the last matching quote when malformed input contains several.
        while j < end {
            if bytes[j] == quote {
                i = j + 1;
            } else if bytes[j] == b'\\' {
                j += 1;
            }
            j += 1;
        }
        result.value = Some(if i > start {
            input[start..i - 1].to_owned()
        } else {
            String::new()
        });
    } else {
        let start = i;
        while i < end && !whitespace(bytes[i]) {
            i += 1;
        }
        result.value = Some(input[start..i].to_owned());
    }
    while i < end && whitespace(bytes[i]) {
        i += 1;
    }
    if i == end {
        return Some(result);
    }
    result.sensitivity = Some(match bytes[i] {
        b'i' | b'I' => 'i',
        b's' | b'S' => 's',
        _ => return None,
    });
    i += 1;
    while i < end && whitespace(bytes[i]) {
        i += 1;
    }
    (i == end).then_some(result)
}
