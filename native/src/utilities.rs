// Port of Tailwind CSS 4.3.4 utility registration/resolution helpers. See LICENSE.tailwind.
// data/utilities.json contains declarative CSS templates, not canonicalization results.
mod manual;
use crate::{
    ast::{AstNode, ContextValue},
    candidate::{Candidate, CandidateKind, Kind, Modifier, Registry, UtilityValue},
    data_type::is_length,
    numeric,
    syntax::{is_js_whitespace, math, segment::segment},
    theme::Theme,
};
use indexmap::IndexMap;
use regex::Regex;
use serde::Deserialize;
use std::{collections::BTreeSet, fmt, sync::LazyLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    pub root: String,
}

impl fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Utility implementation is not ported: {}",
            self.root
        )
    }
}

impl std::error::Error for CompileError {}

#[derive(Deserialize)]
struct Data {
    token: String,
    bindings: Vec<Binding>,
    registry: IndexMap<String, Vec<Kind>>,
    pending: Vec<Pending>,
    definitions: Vec<Definition>,
}

#[derive(Deserialize)]
struct Pending {
    root: String,
    kind: Kind,
}

#[derive(Deserialize)]
struct Binding {
    token: String,
    namespace: String,
    kind: BindingKind,
    fallback: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum BindingKind {
    Get,
    Resolve,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Definition {
    Static {
        root: String,
        nodes: Vec<AstNode>,
    },
    Functional {
        root: String,
        #[serde(flatten)]
        description: Description,
    },
    Color {
        root: String,
        #[serde(flatten)]
        description: Description,
    },
}

#[derive(Deserialize)]
struct Description {
    nodes: Vec<AstNode>,
    supports_negative: bool,
    supports_fractions: bool,
    theme_keys: Vec<String>,
    has_default: bool,
    default_value: Option<String>,
    static_values: IndexMap<String, Vec<AstNode>>,
    bare: Option<Bare>,
    negative_bare: Option<Bare>,
    zero_nodes: Option<Vec<AstNode>>,
    reject_empty: bool,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Bare {
    Spacing,
    NegativeSpacing,
    Fraction,
    Integer,
    #[serde(rename = "integer-%")]
    IntegerPercent,
    #[serde(rename = "integer-deg")]
    IntegerDegrees,
    #[serde(rename = "integer-px")]
    IntegerPixels,
    #[serde(rename = "integer-ms")]
    IntegerMilliseconds,
    GridRepeat,
    Opacity,
    FontStretch,
    MaskAngle,
    NegativeMaskAngle,
}

struct Builtins {
    data: Data,
    marker_start: char,
    index: IndexMap<String, Vec<(usize, bool)>>,
    pending: BTreeSet<(String, Kind)>,
}

static BUILTINS: LazyLock<Builtins> = LazyLock::new(|| {
    let bytes = miniz_oxide::inflate::decompress_to_vec(include_bytes!(concat!(
        env!("OUT_DIR"),
        "/utilities.json.deflate"
    )))
    .expect("falha ao descomprimir as definições de utilitários");
    let data: Data =
        serde_json::from_slice(&bytes).expect("built-in utility definitions must be valid");
    let marker_start = data.token.chars().next().expect("nonempty value marker");
    assert!(
        data.bindings
            .iter()
            .all(|binding| binding.token.starts_with(marker_start)),
        "template markers must share their first character"
    );
    let mut index: IndexMap<String, Vec<(usize, bool)>> = IndexMap::new();
    for (i, definition) in data.definitions.iter().enumerate() {
        let (root, negative) = match definition {
            Definition::Static { root, .. } => (root, false),
            Definition::Functional { root, description } => (root, description.supports_negative),
            Definition::Color { root, .. } => (root, false),
        };
        if negative {
            index.entry(format!("-{root}")).or_default().push((i, true));
        }
        index.entry(root.clone()).or_default().push((i, false));
    }
    let pending = data
        .pending
        .iter()
        .map(|item| (item.root.clone(), item.kind))
        .collect();
    Builtins {
        data,
        marker_start,
        index,
        pending,
    }
});

pub fn registry(prefix: Option<String>) -> Registry {
    Registry {
        prefix,
        utilities: BUILTINS
            .data
            .registry
            .iter()
            .map(|(root, kinds)| (root.clone(), kinds.iter().copied().collect()))
            .collect(),
        variants: Default::default(),
    }
}

pub(crate) fn root_kinds() -> IndexMap<String, Vec<Kind>> {
    BUILTINS.data.registry.clone()
}

pub fn keys(kind: Kind) -> Vec<String> {
    BUILTINS
        .data
        .registry
        .iter()
        .filter(|(_, kinds)| kinds.contains(&kind))
        .map(|(root, _)| root.clone())
        .collect()
}

// JavaScript Number(string), used by color alpha and the font-stretch matcher.
pub(crate) fn number(input: &str) -> f64 {
    let input = input.trim_matches(is_js_whitespace);
    if input.is_empty() {
        return 0.0;
    }
    if let Some(radix) = match input.get(..2) {
        Some("0x" | "0X") => Some(16),
        Some("0o" | "0O") => Some(8),
        Some("0b" | "0B") => Some(2),
        _ => None,
    } {
        let digits = &input[2..];
        if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
            return f64::NAN;
        }
        let bits_per_digit = match radix {
            16 => 4,
            8 => 3,
            _ => 1,
        };
        let bits: Vec<bool> = digits
            .chars()
            .flat_map(|c| {
                let digit = c.to_digit(radix).unwrap();
                (0..bits_per_digit)
                    .rev()
                    .map(move |bit| digit & (1 << bit) != 0)
            })
            .skip_while(|bit| !*bit)
            .collect();
        if bits.len() > 1024 {
            return f64::INFINITY;
        }
        let mut leading = bits
            .iter()
            .take(53)
            .fold(0_u64, |n, bit| n * 2 + u64::from(*bit));
        if bits.get(53) == Some(&true) && (leading & 1 != 0 || bits.iter().skip(54).any(|bit| *bit))
        {
            leading += 1;
        }
        return leading as f64 * 2.0_f64.powi(bits.len().saturating_sub(53) as i32);
    }
    static DECIMAL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^[+-]?(?:(?:[0-9]+\.?[0-9]*|\.[0-9]+)(?:[eE][+-]?[0-9]+)?|Infinity)$").unwrap()
    });
    if !DECIMAL.is_match(input) {
        return f64::NAN;
    }
    input.parse().unwrap_or(f64::NAN)
}

pub fn with_alpha(value: &str, alpha: &str) -> String {
    let converted = number(alpha);
    let alpha = if converted.is_nan() {
        alpha.to_owned()
    } else {
        format!("{}%", numeric::format(converted * 100.0))
    };
    if alpha == "100%" {
        value.into()
    } else {
        format!("color-mix(in oklab, {value} {alpha}, transparent)")
    }
}

pub fn replace_alpha(value: &str, alpha: &str) -> String {
    let converted = number(alpha);
    let alpha = if converted.is_nan() {
        alpha.to_owned()
    } else {
        format!("{}%", numeric::format(converted * 100.0))
    };
    format!("oklab(from {value} l a b / {alpha})")
}

pub fn as_color(value: &str, modifier: Option<&Modifier>, theme: &Theme) -> Option<String> {
    match modifier {
        None => Some(value.into()),
        Some(Modifier::Arbitrary { value: alpha }) => Some(with_alpha(value, alpha)),
        Some(Modifier::Named { value: alpha }) => {
            if let Some(alpha) = theme
                .resolve(Some(alpha), &["--opacity".into()], 0)
                .filter(|value| !value.is_empty())
            {
                return Some(with_alpha(value, &alpha));
            }
            numeric::is_spacing_multiplier(alpha).then(|| with_alpha(value, &format!("{alpha}%")))
        }
    }
}

fn bare_value(kind: Bare, value: &str, fraction: Option<&str>, theme: &Theme) -> Option<String> {
    match kind {
        Bare::Spacing | Bare::NegativeSpacing => {
            if theme
                .resolve(None, &["--spacing".into()], 0)
                .is_none_or(|value| value.is_empty())
                || !numeric::is_spacing_multiplier(value)
            {
                return None;
            }
            Some(format!(
                "--spacing({}{value})",
                if matches!(kind, Bare::NegativeSpacing) {
                    "-"
                } else {
                    ""
                }
            ))
        }
        Bare::Fraction => {
            let fraction = fraction?;
            let parts = segment(fraction, b'/');
            (parts.len() >= 2
                && numeric::is_spacing_multiplier(parts[0])
                && numeric::is_spacing_multiplier(parts[1]))
            .then(|| fraction.into())
        }
        Bare::GridRepeat => numeric::is_strict_positive_integer(value)
            .then(|| format!("repeat({value}, minmax(0, 1fr))")),
        Bare::Opacity => numeric::is_spacing_multiplier(value).then(|| format!("{value}%")),
        Bare::FontStretch => {
            let num = number(value.strip_suffix('%')?);
            (num.fract() == 0.0 && (50.0..=200.0).contains(&num)).then(|| value.into())
        }
        Bare::MaskAngle | Bare::NegativeMaskAngle => {
            if !numeric::is_positive_integer(value) {
                return None;
            }
            let sign = if matches!(kind, Bare::NegativeMaskAngle) {
                "-"
            } else {
                ""
            };
            Some(match value {
                "0" => "0deg".into(),
                "1" => format!("{sign}1deg"),
                _ => format!("calc(1deg * {sign}{value})"),
            })
        }
        Bare::Integer
        | Bare::IntegerPercent
        | Bare::IntegerDegrees
        | Bare::IntegerPixels
        | Bare::IntegerMilliseconds => {
            if !numeric::is_positive_integer(value) {
                return None;
            }
            let suffix = match kind {
                Bare::IntegerPercent => "%",
                Bare::IntegerDegrees => "deg",
                Bare::IntegerPixels => "px",
                Bare::IntegerMilliseconds => "ms",
                _ => "",
            };
            Some(format!("{value}{suffix}"))
        }
    }
}

struct Templates<'a> {
    bindings: &'a Bindings,
    value: &'a str,
}

// createUtilities captures these values when it registers the handlers.
pub(crate) struct Bindings {
    values: Vec<(&'static str, String)>,
}

impl Bindings {
    pub(crate) fn new(theme: &Theme) -> Self {
        let values = BUILTINS
            .data
            .bindings
            .iter()
            .map(|binding| {
                let keys = [binding.namespace.clone()];
                let resolved = match binding.kind {
                    BindingKind::Get => theme.get(&keys).map(str::to_owned),
                    BindingKind::Resolve => theme.resolve(None, &keys, 0),
                }
                .unwrap_or_else(|| binding.fallback.clone());
                (binding.token.as_str(), resolved)
            })
            .collect();
        Self { values }
    }
}

impl<'a> Templates<'a> {
    fn new(bindings: &'a Bindings, value: &'a str) -> Self {
        Self { bindings, value }
    }

    // Replacements are opaque: a user's value containing a marker is not evaluated again.
    fn text(&self, input: &str) -> String {
        let mut consumed = 0;
        let mut output = String::with_capacity(input.len());
        for (offset, _) in input.match_indices(BUILTINS.marker_start) {
            if offset < consumed {
                continue;
            }
            if let Some((token, value)) = self
                .bindings
                .values
                .iter()
                .map(|(token, value)| (*token, value.as_str()))
                .chain(std::iter::once((BUILTINS.data.token.as_str(), self.value)))
                .find(|(token, _)| input[offset..].starts_with(token))
            {
                output.push_str(&input[consumed..offset]);
                output.push_str(value);
                consumed = offset + token.len();
            }
        }
        output.push_str(&input[consumed..]);
        output
    }

    fn nodes(&self, template: &[AstNode]) -> Vec<AstNode> {
        template
            .iter()
            .map(|node| match node {
                AstNode::Rule { selector, nodes } => AstNode::Rule {
                    selector: self.text(selector),
                    nodes: self.nodes(nodes),
                },
                AstNode::AtRule {
                    name,
                    params,
                    nodes,
                } => AstNode::AtRule {
                    name: self.text(name),
                    params: self.text(params),
                    nodes: self.nodes(nodes),
                },
                AstNode::Declaration {
                    property,
                    value,
                    important,
                    ..
                } => AstNode::Declaration {
                    identity: Default::default(),
                    property: self.text(property).into(),
                    value: value.as_ref().map(|value| self.text(value)),
                    important: *important,
                },
                AstNode::Comment { value } => AstNode::Comment {
                    value: self.text(value),
                },
                AstNode::AtRoot { nodes } => AstNode::AtRoot {
                    nodes: self.nodes(nodes),
                },
                AstNode::Context { context, nodes } => AstNode::Context {
                    context: context
                        .iter()
                        .map(|(key, value)| {
                            (
                                key.clone(),
                                match value {
                                    ContextValue::String(value) => {
                                        ContextValue::String(self.text(value))
                                    }
                                    ContextValue::Bool(value) => ContextValue::Bool(*value),
                                },
                            )
                        })
                        .collect(),
                    nodes: self.nodes(nodes),
                },
            })
            .collect()
    }
}

fn handle(description: &Description, value: &str, bindings: &Bindings) -> Option<Vec<AstNode>> {
    if description.reject_empty && value.is_empty() {
        return None;
    }
    let zero = value == "--spacing(0)"
        || value == "--spacing(-0)"
        || numeric::dimension(value)
            .is_some_and(|(n, unit)| n == 0.0 && (unit.is_none() || is_length(value)));
    let nodes = if zero {
        description
            .zero_nodes
            .as_deref()
            .unwrap_or(&description.nodes)
    } else {
        &description.nodes
    };
    Some(Templates::new(bindings, value).nodes(nodes))
}

fn functional(
    description: &Description,
    candidate: &CandidateKind,
    negative: bool,
    theme: &Theme,
    bindings: &Bindings,
) -> Option<Vec<AstNode>> {
    let CandidateKind::Functional {
        value: candidate_value,
        modifier,
        ..
    } = candidate
    else {
        return None;
    };
    let mut value;
    match candidate_value {
        None => {
            if modifier.is_some() {
                return None;
            }
            value = if description.has_default {
                description
                    .default_value
                    .as_ref()
                    .map(|value| Templates::new(bindings, "").text(value))
            } else {
                theme.resolve(None, &description.theme_keys, 0)
            };
        }
        Some(UtilityValue::Arbitrary {
            value: arbitrary, ..
        }) => {
            if modifier.is_some() {
                return None;
            }
            value = Some(arbitrary.clone());
        }
        Some(UtilityValue::Named {
            value: named,
            fraction,
        }) => {
            value = theme.resolve(
                Some(fraction.as_deref().unwrap_or(named)),
                &description.theme_keys,
                0,
            );
            if value.is_some() && modifier.is_some() && fraction.is_none() {
                return None;
            }
            if value.is_none()
                && description.supports_fractions
                && let Some(fraction) = fraction
            {
                let parts = segment(fraction, b'/');
                if parts.len() < 2
                    || !numeric::is_positive_integer(parts[0])
                    || !numeric::is_positive_integer(parts[1])
                {
                    return None;
                }
                value = Some(format!("calc({} / {} * 100%)", parts[0], parts[1]));
            }
            if value.is_none()
                && negative
                && let Some(kind) = description.negative_bare
            {
                value = bare_value(kind, named, fraction.as_deref(), theme);
                if !value.as_ref().is_some_and(|value| value.contains('/')) && modifier.is_some() {
                    return None;
                }
                if let Some(value) = value {
                    return handle(description, &value, bindings);
                }
            }
            if value.is_none()
                && let Some(kind) = description.bare
            {
                value = bare_value(kind, named, fraction.as_deref(), theme);
                if !value.as_ref().is_some_and(|value| value.contains('/')) && modifier.is_some() {
                    return None;
                }
            }
            if value.is_none()
                && !negative
                && modifier.is_none()
                && let Some(nodes) = description.static_values.get(named)
            {
                return Some(Templates::new(bindings, "").nodes(nodes));
            }
        }
    }
    let value = value?;
    handle(
        description,
        &if negative {
            math::add_whitespace(&format!("calc({value} * -1)"))
        } else {
            value
        },
        bindings,
    )
}

fn color(
    description: &Description,
    candidate: &CandidateKind,
    theme: &Theme,
    bindings: &Bindings,
) -> Option<Vec<AstNode>> {
    let CandidateKind::Functional {
        value: Some(value),
        modifier,
        ..
    } = candidate
    else {
        return None;
    };
    let resolved = match value {
        UtilityValue::Arbitrary { value, .. } => value.clone(),
        UtilityValue::Named { value, .. } => match value.as_str() {
            "inherit" => "inherit".into(),
            "transparent" => "transparent".into(),
            "current" => "currentcolor".into(),
            _ => theme.resolve(Some(value), &description.theme_keys, 0)?,
        },
    };
    if resolved.is_empty() && matches!(value, UtilityValue::Named { .. }) {
        return None;
    }
    handle(
        description,
        &as_color(&resolved, modifier.as_ref(), theme)?,
        bindings,
    )
}

/// Compile the utility body. Variant application, importance, and CSS functions belong to later stages.
/// Unported implementations produce an explicit error rather than treating valid classes as invalid.
pub fn compile_base(
    candidate: &Candidate,
    theme: &Theme,
) -> Result<Vec<Vec<AstNode>>, CompileError> {
    compile_base_with_bindings(candidate, theme, &Bindings::new(theme))
}

pub(crate) fn compile_base_with_bindings(
    candidate: &Candidate,
    theme: &Theme,
    bindings: &Bindings,
) -> Result<Vec<Vec<AstNode>>, CompileError> {
    let (root, kind) = match &candidate.kind {
        CandidateKind::Arbitrary {
            property,
            value,
            modifier,
        } => {
            return Ok(as_color(value, modifier.as_ref(), theme)
                .map(|value| vec![vec![AstNode::decl(property, value)]])
                .unwrap_or_default());
        }
        CandidateKind::Static { root } => (root, Kind::Static),
        CandidateKind::Functional { root, .. } => (root, Kind::Functional),
    };
    if let Some(nodes) = manual::compile(candidate, theme) {
        return Ok(nodes);
    }
    if BUILTINS.pending.contains(&(root.clone(), kind)) {
        return Err(CompileError { root: root.clone() });
    }
    let mut output = Vec::new();
    for &(index, negative) in BUILTINS.index.get(root).into_iter().flatten() {
        let compiled = match &BUILTINS.data.definitions[index] {
            Definition::Static { nodes, .. } if kind == Kind::Static => {
                Some(Templates::new(bindings, "").nodes(nodes))
            }
            Definition::Functional { description, .. } if kind == Kind::Functional => {
                functional(description, &candidate.kind, negative, theme, bindings)
            }
            Definition::Color { description, .. } if kind == Kind::Functional => {
                color(description, &candidate.kind, theme, bindings)
            }
            _ => None,
        };
        if let Some(nodes) = compiled {
            output.push(nodes);
        }
    }
    Ok(output)
}
