// Numeric serialization must match ECMAScript, including exponent thresholds and -0.
use regex::Regex;
use std::sync::LazyLock;

pub fn format(value: f64) -> String {
    ryu_js::Buffer::new().format(value).to_owned()
}

pub fn dimension(input: &str) -> Option<(f64, Option<&str>)> {
    static PATTERN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^([-+]?(?:[0-9]*\.)?[0-9]+)([a-zA-Z]+|%)?$").unwrap());
    let captures = PATTERN.captures(input)?;
    let value = captures[1].parse::<f64>().ok()?;
    (!value.is_nan()).then(|| (value, captures.get(2).map(|c| c.as_str())))
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
