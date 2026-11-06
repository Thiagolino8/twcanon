// Port of Tailwind CSS 4.3.4 utils/segment.ts. See LICENSE.tailwind.

/// Split on an ASCII separator outside quotes and balanced delimiters.
pub fn segment(input: &str, separator: u8) -> Vec<&str> {
    assert!(separator.is_ascii());
    let bytes = input.as_bytes();
    let mut closing = [0; 256];
    let mut depth = 0;
    let mut parts = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        let current = bytes[i];
        if depth == 0 && current == separator {
            parts.push(&input[start..i]);
            start = i + 1;
            i += 1;
            continue;
        }
        match current {
            b'\\' => i += 1,
            b'\'' | b'"' => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == current {
                        break;
                    }
                    i += 1;
                }
            }
            b'(' | b'[' | b'{' => {
                if let Some(slot) = closing.get_mut(depth) {
                    *slot = match current {
                        b'(' => b')',
                        b'[' => b']',
                        _ => b'}',
                    };
                }
                depth += 1;
            }
            b')' | b']' | b'}' if depth > 0 && closing.get(depth - 1) == Some(&current) => {
                depth -= 1;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(&input[start..]);
    parts
}
