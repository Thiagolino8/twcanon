// Port of Tailwind CSS 4.3.4 utils/infer-data-type.ts. See LICENSE.tailwind.
use crate::{
    numeric,
    syntax::{math::has_math_fn, segment::segment},
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, sync::LazyLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DataType {
    Color,
    Length,
    Percentage,
    Ratio,
    Number,
    Integer,
    Url,
    Position,
    BgSize,
    LineWidth,
    Image,
    FamilyName,
    GenericName,
    AbsoluteSize,
    RelativeSize,
    Angle,
    Vector,
}

const NUMBER: &str = r"[+-]?[0-9]*\.?[0-9]+(?:[eE][+-]?[0-9]+)?";

fn pattern(suffix: &str) -> Regex {
    Regex::new(&format!("^{NUMBER}{suffix}$")).unwrap()
}

pub fn is_length(value: &str) -> bool {
    static LENGTH: LazyLock<Regex> = LazyLock::new(|| {
        pattern(
            "(cm|mm|Q|in|pc|pt|px|em|ex|ch|cap|ic|rem|rex|rch|rcap|ric|lh|rlh|vw|vh|vmin|vmax|vb|vi|svw|svh|svi|svb|svmin|svmax|lvw|lvh|lvi|lvb|lvmin|lvmax|dvw|dvh|dvi|dvb|dvmin|dvmax|cqw|cqh|cqi|cqb|cqmin|cqmax)",
        )
    });
    LENGTH.is_match(value)
        || value
            .get(..10)
            .is_some_and(|s| s.eq_ignore_ascii_case("--spacing("))
        || has_math_fn(value)
}

fn is_number(value: &str) -> bool {
    static PATTERN: LazyLock<Regex> = LazyLock::new(|| pattern(""));
    PATTERN.is_match(value) || has_math_fn(value)
}

fn is_percentage(value: &str) -> bool {
    static PATTERN: LazyLock<Regex> = LazyLock::new(|| pattern("%"));
    PATTERN.is_match(value) || has_math_fn(value)
}

fn is_url(value: &str) -> bool {
    value.starts_with("url(")
        && value.ends_with(')')
        && !value.contains(['\n', '\r', '\u{2028}', '\u{2029}'])
}

pub fn is_named_color(value: &str) -> bool {
    static NAMES: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
        include_str!("../data/colors.txt")
            .split_whitespace()
            .collect()
    });
    NAMES.contains(value.to_lowercase().as_str())
}

pub fn is_color(value: &str) -> bool {
    static PATTERN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i-u)^(rgba?|hsla?|hwb|color|(ok)?(lab|lch)|light-dark|color-mix|--alpha)\(")
            .unwrap()
    });
    value.starts_with('#') || PATTERN.is_match(value) || is_named_color(value)
}

impl DataType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Color => "color",
            Self::Length => "length",
            Self::Percentage => "percentage",
            Self::Ratio => "ratio",
            Self::Number => "number",
            Self::Integer => "integer",
            Self::Url => "url",
            Self::Position => "position",
            Self::BgSize => "bg-size",
            Self::LineWidth => "line-width",
            Self::Image => "image",
            Self::FamilyName => "family-name",
            Self::GenericName => "generic-name",
            Self::AbsoluteSize => "absolute-size",
            Self::RelativeSize => "relative-size",
            Self::Angle => "angle",
            Self::Vector => "vector",
        }
    }

    pub fn matches(self, value: &str) -> bool {
        match self {
            Self::Color => is_color(value),
            Self::Length => is_length(value),
            Self::Percentage => is_percentage(value),
            Self::Ratio => {
                static PATTERN: LazyLock<Regex> = LazyLock::new(|| {
                    Regex::new(&format!(r"^{NUMBER}[\t\n\x0b\x0c\r \x{{a0}}\x{{1680}}\x{{2000}}-\x{{200a}}\x{{2028}}\x{{2029}}\x{{202f}}\x{{205f}}\x{{3000}}\x{{feff}}]*/[\t\n\x0b\x0c\r \x{{a0}}\x{{1680}}\x{{2000}}-\x{{200a}}\x{{2028}}\x{{2029}}\x{{202f}}\x{{205f}}\x{{3000}}\x{{feff}}]*{NUMBER}$")).unwrap()
                });
                PATTERN.is_match(value) || has_math_fn(value)
            }
            Self::Number => is_number(value),
            Self::Integer => numeric::is_positive_integer(value),
            Self::Url => is_url(value),
            Self::Position => {
                let mut count = 0;
                for part in segment(value, b' ') {
                    if matches!(part, "center" | "top" | "right" | "bottom" | "left") {
                        count += 1;
                    } else if part.starts_with("var(") {
                        continue;
                    } else if is_length(part) || is_percentage(part) {
                        count += 1;
                    } else {
                        return false;
                    }
                }
                count > 0
            }
            Self::BgSize => {
                let mut count = 0;
                for size in segment(value, b',') {
                    if matches!(size, "cover" | "contain") {
                        count += 1;
                        continue;
                    }
                    let parts = segment(size, b' ');
                    if !matches!(parts.len(), 1 | 2) {
                        return false;
                    }
                    if parts
                        .iter()
                        .all(|s| *s == "auto" || is_length(s) || is_percentage(s))
                    {
                        count += 1;
                    }
                }
                count > 0
            }
            Self::LineWidth => segment(value, b' ')
                .iter()
                .all(|s| is_length(s) || is_number(s) || matches!(*s, "thin" | "medium" | "thick")),
            Self::Image => {
                let mut count = 0;
                for part in segment(value, b',') {
                    if part.starts_with("var(") {
                        continue;
                    }
                    if is_url(part)
                        || [
                            "element(",
                            "image(",
                            "cross-fade(",
                            "image-set(",
                            "conic-gradient(",
                            "linear-gradient(",
                            "radial-gradient(",
                            "repeating-conic-gradient(",
                            "repeating-linear-gradient(",
                            "repeating-radial-gradient(",
                        ]
                        .iter()
                        .any(|p| part.starts_with(p))
                    {
                        count += 1;
                    } else {
                        return false;
                    }
                }
                count > 0
            }
            Self::FamilyName => {
                let mut count = 0;
                for part in segment(value, b',') {
                    if part.starts_with(|c: char| c.is_ascii_digit()) {
                        return false;
                    }
                    if !part.starts_with("var(") {
                        count += 1;
                    }
                }
                count > 0
            }
            Self::GenericName => matches!(
                value,
                "serif"
                    | "sans-serif"
                    | "monospace"
                    | "cursive"
                    | "fantasy"
                    | "system-ui"
                    | "ui-serif"
                    | "ui-sans-serif"
                    | "ui-monospace"
                    | "ui-rounded"
                    | "math"
                    | "emoji"
                    | "fangsong"
            ),
            Self::AbsoluteSize => matches!(
                value,
                "xx-small"
                    | "x-small"
                    | "small"
                    | "medium"
                    | "large"
                    | "x-large"
                    | "xx-large"
                    | "xxx-large"
            ),
            Self::RelativeSize => matches!(value, "larger" | "smaller"),
            Self::Angle => {
                static PATTERN: LazyLock<Regex> = LazyLock::new(|| pattern("(deg|rad|grad|turn)"));
                PATTERN.is_match(value)
            }
            Self::Vector => {
                static PATTERN: LazyLock<Regex> = LazyLock::new(|| {
                    Regex::new(&format!("^{NUMBER} +{NUMBER} +{NUMBER}$")).unwrap()
                });
                PATTERN.is_match(value)
            }
        }
    }
}

pub fn infer(value: &str, types: &[DataType]) -> Option<DataType> {
    if value.starts_with("var(") {
        return None;
    }
    types.iter().copied().find(|t| t.matches(value))
}
