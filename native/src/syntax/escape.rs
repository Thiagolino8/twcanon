// Port of Tailwind CSS 4.3.4 utils/escape.ts. See LICENSE.tailwind.
pub fn escape(input: &str) -> String {
    if input == "-" {
        return "\\-".into();
    }
    let mut output = String::new();
    for (i, c) in input.chars().enumerate() {
        if c == '\0' {
            output.push('\u{fffd}');
        } else if matches!(c, '\u{1}'..='\u{1f}' | '\u{7f}')
            || c.is_ascii_digit() && (i == 0 || i == 1 && input.starts_with('-'))
        {
            use std::fmt::Write;
            write!(output, "\\{:x} ", u32::from(c)).unwrap();
        } else if c >= '\u{80}' || c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
            output.push(c);
        } else {
            output.push('\\');
            output.push(c);
        }
    }
    output
}

pub fn unescape(input: &str) -> String {
    let mut chars = input.chars().peekable();
    let mut output = String::new();
    while let Some(c) = chars.next() {
        if c != '\\' || chars.peek().is_none() {
            output.push(c);
            continue;
        }
        let mut codepoint = 0;
        let mut digits = 0;
        while digits < 6 && chars.peek().is_some_and(char::is_ascii_hexdigit) {
            codepoint = codepoint * 16 + chars.next().unwrap().to_digit(16).unwrap();
            digits += 1;
        }
        if digits == 0 {
            output.push(chars.next().unwrap());
        } else {
            if chars
                .peek()
                .is_some_and(|c| matches!(c, '\t' | '\n' | '\u{c}' | '\r' | ' '))
            {
                chars.next();
            }
            output.push(if codepoint == 0 {
                '\u{fffd}'
            } else {
                char::from_u32(codepoint).unwrap_or('\u{fffd}')
            });
        }
    }
    output
}
