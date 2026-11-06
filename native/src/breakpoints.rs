// Port of Tailwind CSS 4.3.4 utils/compare-breakpoints.ts. See LICENSE.tailwind.
use crate::syntax::is_js_whitespace;
use regex::Regex;
use serde::Deserialize;
use std::{cmp::Ordering, sync::LazyLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Asc,
    Desc,
}

pub fn compare(left: &str, right: &str, direction: Direction) -> Ordering {
    if left == right {
        return Ordering::Equal;
    }
    let bucket = |value: &str| match value.find('(') {
        Some(index) => value[..index].to_owned(),
        None => value
            .chars()
            .filter(|c| !c.is_ascii_digit() && *c != '.')
            .collect(),
    };
    let order = bucket(left)
        .encode_utf16()
        .cmp(bucket(right).encode_utf16());
    if order != Ordering::Equal {
        return order;
    }
    static FLOAT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^[+-]?(?:Infinity|(?:[0-9]+\.?[0-9]*|\.[0-9]+)(?:[eE][+-]?[0-9]+)?)").unwrap()
    });
    let parse = |input: &str| {
        FLOAT
            .find(input.trim_start_matches(is_js_whitespace))
            .and_then(|m| m.as_str().parse::<f64>().ok())
            .unwrap_or(f64::NAN)
    };
    let difference = match direction {
        Direction::Asc => parse(left) - parse(right),
        Direction::Desc => parse(right) - parse(left),
    };
    difference
        .partial_cmp(&0.0)
        .unwrap_or_else(|| left.encode_utf16().cmp(right.encode_utf16()))
}
