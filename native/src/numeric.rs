// Numeric serialization must match ECMAScript, including exponent thresholds and -0.
pub fn format(value: f64) -> String {
    ryu_js::Buffer::new().format(value).to_owned()
}

pub fn dimension(input: &str) -> Option<(f64, Option<&str>)> {
    let unsigned = input.strip_prefix(['-', '+']).unwrap_or(input);
    let mut length = unsigned.bytes().take_while(u8::is_ascii_digit).count();
    if unsigned.as_bytes().get(length) == Some(&b'.') {
        length += 1;
        let fraction = unsigned[length..]
            .bytes()
            .take_while(u8::is_ascii_digit)
            .count();
        if fraction == 0 {
            return None;
        }
        length += fraction;
    } else if length == 0 {
        return None;
    }
    let unit = &unsigned[length..];
    if unit != "%" && !unit.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return None;
    }
    let number = input[..input.len() - unit.len()].parse().ok()?;
    Some((number, (!unit.is_empty()).then_some(unit)))
}

pub fn is_positive_integer(value: &str) -> bool {
    value
        .parse::<f64>()
        .is_ok_and(|n| n >= 0.0 && n.fract() == 0.0 && format(n) == value)
}

pub fn is_strict_positive_integer(value: &str) -> bool {
    value
        .parse::<f64>()
        .is_ok_and(|n| n > 0.0 && n.fract() == 0.0 && format(n) == value)
}

pub fn is_spacing_multiplier(value: &str) -> bool {
    value
        .parse::<f64>()
        .is_ok_and(|n| n >= 0.0 && n % 0.25 == 0.0 && format(n) == value)
}

/// Number(value.toFixed(digits)), including JavaScript's tie rule.
pub fn round_fixed(value: f64, digits: u32) -> f64 {
    assert!(digits <= 10);
    if value == 0.0 {
        return 0.0;
    }
    if !value.is_finite() || value.abs() >= 1e21 {
        return value;
    }
    let bits = value.abs().to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1_u64 << 52) - 1);
    let mantissa = if exponent == 0 {
        fraction
    } else {
        fraction | (1_u64 << 52)
    };
    let exponent = if exponent == 0 {
        -1074
    } else {
        exponent - 1075
    };
    let scale = 10_u128.pow(digits);
    let numerator = u128::from(mantissa) * scale;
    let rounded = if exponent >= 0 {
        numerator << exponent
    } else {
        let shift = (-exponent) as u32;
        if shift >= 128 {
            0
        } else {
            (numerator >> shift)
                + u128::from(numerator & ((1_u128 << shift) - 1) >= (1_u128 << (shift - 1)))
        }
    };
    let sign = if value.is_sign_negative() { "-" } else { "" };
    format!(
        "{sign}{}.{:0width$}",
        rounded / scale,
        rounded % scale,
        width = digits as usize
    )
    .parse()
    .unwrap()
}

#[cfg(test)]
mod tests {
    #[test]
    fn dimension_preserves_numeric_grammar() {
        let reference = regex::Regex::new(r"^([-+]?(?:[0-9]*\.)?[0-9]+)([a-zA-Z]+|%)?$").unwrap();
        let overflow = "9".repeat(400);
        for number in [
            "",
            "-",
            "+",
            ".",
            ".5",
            "+.5",
            "-.5",
            "0",
            "-0",
            "+0",
            "01",
            "0001",
            "1.0",
            "-001.25",
            "1.",
            "1e3",
            "1.2.3",
            "NaN",
            "Infinity",
            "١",
            "1\n",
            "\u{feff}1",
            "1\0",
            &overflow,
        ] {
            for unit in [
                "", "px", "REM", "%", "e", "e3", "px%", "px\n", " px", "-px", "π", "\u{a0}",
            ] {
                let input = format!("{number}{unit}");
                let expected = reference.captures(&input).and_then(|captures| {
                    let value = captures[1].parse::<f64>().ok()?;
                    Some((
                        value.to_bits(),
                        captures.get(2).map(|capture| capture.as_str()),
                    ))
                });
                let actual = super::dimension(&input).map(|(value, unit)| (value.to_bits(), unit));
                assert_eq!(actual, expected, "{input:?}");
            }
        }
    }
}
