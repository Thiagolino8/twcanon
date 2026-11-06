// Port of Tailwind CSS 4.3.4 utils/math-operators.ts. See LICENSE.tailwind.
use super::is_js_whitespace;

pub const FUNCTIONS: &[&str] = &[
    "calc", "min", "max", "clamp", "mod", "rem", "sin", "cos", "tan", "asin", "acos", "atan",
    "atan2", "pow", "sqrt", "hypot", "log", "exp", "round",
];

pub fn has_math_fn(input: &str) -> bool {
    input.contains('(')
        && FUNCTIONS
            .iter()
            .any(|name| input.contains(&format!("{name}(")))
}

const fn operator(c: char) -> bool {
    matches!(c, '+' | '-' | '*' | '/')
}

pub fn add_whitespace(input: &str) -> String {
    if !FUNCTIONS.iter().any(|name| input.contains(name)) {
        return input.into();
    }
    let chars: Vec<char> = input.chars().collect();
    let mut result = String::new();
    let mut formattable = Vec::new();
    let mut value_pos = None;
    let mut last_value_pos = None;
    for (i, current) in chars.iter().copied().enumerate() {
        if current.is_ascii_digit()
            || (value_pos.is_some() && (current == '%' || current.is_ascii_alphabetic()))
        {
            value_pos = Some(i);
        } else {
            last_value_pos = value_pos;
            value_pos = None;
        }
        let formats = formattable.last() == Some(&true);
        if current == '(' {
            result.push(current);
            let mut start = i;
            while start > 0
                && (chars[start - 1].is_ascii_digit() || chars[start - 1].is_ascii_lowercase())
            {
                start -= 1;
            }
            let name: String = chars[start..i].iter().collect();
            formattable.push(FUNCTIONS.contains(&name.as_str()) || (formats && name.is_empty()));
        } else if current == ')' {
            result.push(current);
            formattable.pop();
        } else if current == ',' && formats {
            result.push_str(", ");
        } else if current == ' ' && formats && result.ends_with(' ') {
            continue;
        } else if operator(current) && formats {
            let trimmed = result.trim_end_matches(is_js_whitespace);
            let mut previous = trimmed.chars().rev();
            let prev = previous.next();
            let prev_prev = previous.next();
            let next = chars.get(i + 1).copied();
            if (matches!(prev, Some('e' | 'E')) && prev_prev.is_some_and(|c| c.is_ascii_digit()))
                || prev.is_some_and(operator)
                || matches!(prev, Some('(' | ','))
            {
                result.push(current);
            } else if i > 0 && chars[i - 1] == ' ' {
                result.push(current);
                result.push(' ');
            } else if prev.is_some_and(|c| c.is_ascii_digit())
                || next.is_some_and(|c| c.is_ascii_digit())
                || prev == Some(')')
                || next == Some('(')
                || next.is_some_and(operator)
                || (i > 0 && last_value_pos == Some(i - 1))
            {
                result.push(' ');
                result.push(current);
                result.push(' ');
            } else {
                result.push(current);
            }
        } else {
            result.push(current);
        }
    }
    result
}
