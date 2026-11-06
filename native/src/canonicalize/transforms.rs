// Utility canonicalization passes from Tailwind CSS 4.3.4.
use super::{
    Canonicalizer, SignatureIndex,
    converter::{self, Convert},
};
use crate::{
    candidate::{Candidate, CandidateKind, Kind, Modifier, UtilityValue},
    compile::DesignSystem,
    css_functions, declaration, js_sort, numeric, print,
    signature::Options,
    syntax::{
        is_js_whitespace,
        value::{self, Value},
    },
    utilities,
};
use indexmap::IndexSet;
use regex::Regex;
use std::{collections::HashMap, sync::LazyLock};

pub(super) struct Spacing {
    value: f64,
    unit: Option<String>,
    rem: Option<f64>,
    values: HashMap<String, Option<f64>>,
}

impl Spacing {
    pub(super) fn new(system: &DesignSystem, rem: Option<f64>) -> Option<Self> {
        let value = css_functions::resolve_theme_value(&system.theme, "--spacing", true)?;
        let folded = declaration::constant_fold(&value, rem, true);
        let (value, unit) = numeric::dimension(&folded)?;
        Some(Self {
            value,
            unit: unit.map(str::to_owned),
            rem,
            values: HashMap::new(),
        })
    }
    pub(super) fn get(&mut self, input: &str) -> Option<f64> {
        if let Some(value) = self.values.get(input) {
            return *value;
        }
        let result = if self.value == 0.0 {
            None
        } else {
            numeric::dimension(&declaration::constant_fold(input, self.rem, true))
                .filter(|(_, unit)| *unit == self.unit.as_deref())
                .map(|(value, _)| value / self.value)
        };
        self.values.insert(input.into(), result);
        result
    }
}

fn modifier(candidate: &Candidate) -> Option<&Modifier> {
    match &candidate.kind {
        CandidateKind::Arbitrary { modifier, .. } | CandidateKind::Functional { modifier, .. } => {
            modifier.as_ref()
        }
        _ => None,
    }
}
fn set_modifier(candidate: &mut Candidate, new: Option<Modifier>) {
    match &mut candidate.kind {
        CandidateKind::Arbitrary { modifier, .. } | CandidateKind::Functional { modifier, .. } => {
            *modifier = new
        }
        _ => {}
    }
}
fn arbitrary_value(candidate: &mut Candidate) -> Option<&mut String> {
    match &mut candidate.kind {
        CandidateKind::Arbitrary { value, .. }
        | CandidateKind::Functional {
            value: Some(UtilityValue::Arbitrary { value, .. }),
            ..
        } => Some(value),
        _ => None,
    }
}
fn function_value(candidate: &Candidate) -> Option<&str> {
    match &candidate.kind {
        CandidateKind::Functional {
            value: Some(UtilityValue::Arbitrary { value, .. } | UtilityValue::Named { value, .. }),
            ..
        }
        | CandidateKind::Arbitrary { value, .. } => Some(value),
        _ => None,
    }
}

type Accept<'a> =
    dyn FnMut(&mut Canonicalizer, Candidate) -> Result<Option<Candidate>, String> + 'a;

impl Canonicalizer {
    pub fn transform_utility(
        &mut self,
        mut candidate: Candidate,
        options: Options,
    ) -> Result<Candidate, String> {
        self.prepare_storage(options.rem);
        candidate = candidate.clone_fresh();
        if let CandidateKind::Static { root } = &mut candidate.kind
            && let Some(direction) = root.strip_prefix("bg-gradient-to-")
            && ["t", "tr", "r", "br", "b", "bl", "l", "tl"].contains(&direction)
        {
            *root = format!("bg-linear-to-{direction}")
        }
        let mode = if modifier(&candidate).is_none() {
            Convert::MigrateModifier
        } else {
            Convert::All
        };
        let mut new_modifier = None;
        if let Some(value) = arbitrary_value(&mut candidate) {
            let (new_value, converted_modifier) =
                converter::convert(&self.design_system.theme, value, mode)?;
            if new_value != *value {
                *value = new_value;
                new_modifier = converted_modifier
            }
        }
        if new_modifier.is_some() {
            set_modifier(&mut candidate, new_modifier)
        }
        if let Some(value) = arbitrary_value(&mut candidate) {
            *value = converter::spacing_calc(value, &self.design_system.theme)
        }
        candidate = self.optimize_arbitrary_expression(candidate, options);
        candidate = self.replace_arbitrary_utility(candidate, options)?;
        candidate = self.replace_bare_utility(candidate, options)?;
        candidate = self.replace_deprecated_utility(candidate, options);
        candidate = self.drop_data_type(candidate, options);
        candidate = self.arbitrary_to_bare_utility(candidate, options);
        candidate = self.optimize_modifier(candidate, options);
        Ok(candidate)
    }

    fn printed(&self, candidate: &Candidate) -> String {
        print::candidate(candidate, self.design_system.theme.prefix.as_deref())
    }

    fn parse_unprefixed(&mut self, input: &str) -> Vec<Candidate> {
        let prefix = self
            .design_system
            .theme
            .prefix
            .as_deref()
            .filter(|p| !p.is_empty());
        let input = match prefix {
            Some(prefix) if !input.starts_with(&format!("{prefix}:")) => {
                format!("{prefix}:{input}")
            }
            _ => input.into(),
        };
        self.design_system.parse_candidate(&input)
    }

    fn same_candidates(&mut self, left: &Candidate, right: &Candidate, options: Options) -> bool {
        self.same_utility(&self.printed(left), &self.printed(right), options)
    }

    // Direct cache equality also equates the invalid symbol for the same input.
    fn signatures_equal(&mut self, left: &str, right: &str, options: Options) -> bool {
        if left == right {
            return true;
        }
        self.same_utility(left, right, options)
    }

    fn reasonable_spacing(&self, value: f64, rem: Option<f64>) -> bool {
        let Some(spacing) =
            css_functions::resolve_theme_value(&self.design_system.theme, "--spacing", true)
        else {
            return false;
        };
        let folded = declaration::constant_fold(&spacing, rem, true);
        let Some((spacing, unit)) = numeric::dimension(&folded) else {
            return false;
        };
        match unit {
            Some("px") => value * spacing <= 1536.0,
            Some("rem") => value * spacing <= 96.0,
            _ => false,
        }
    }

    fn spacing_factor(&mut self, value: &str) -> Option<f64> {
        self.spacing.as_mut()?.get(value)
    }

    fn try_parsed(
        &mut self,
        input: &str,
        accept: &mut Accept<'_>,
    ) -> Result<Option<Candidate>, String> {
        for candidate in self.parse_unprefixed(input) {
            if let Some(found) = accept(self, candidate)? {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    fn visit_replacements(
        &mut self,
        signature: &str,
        candidate: &Candidate,
        options: Options,
        arbitrary: bool,
        index: &SignatureIndex,
        accept: &mut Accept<'_>,
    ) -> Result<Option<Candidate>, String> {
        let replacements = index.get(signature).map(Vec::as_slice).unwrap_or_default();
        if replacements.len() > 1 {
            let mut positive = replacements.iter().filter(|value| !value.starts_with('-'));
            if let Some(replacement) = positive.next()
                && positive.next().is_none()
            {
                return self.try_parsed(replacement, accept);
            }
            return Ok(None);
        }
        if replacements.is_empty()
            && let Some(modifier) = modifier(candidate).cloned()
        {
            let mut stripped = candidate.clone_fresh();
            set_modifier(&mut stripped, None);
            if let Some(signature) = self.utility_signature(&self.printed(&stripped), options) {
                let mut reattach = |system: &mut Self, replacement: Candidate| {
                    // Object.assign({}, replacementCandidate, { modifier }) creates
                    // a new candidate without changing the parsed object's modifier.
                    let mut replacement = replacement.clone_fresh();
                    set_modifier(&mut replacement, Some(modifier.clone()));
                    accept(system, replacement)
                };
                if let Some(found) = self.visit_replacements(
                    &signature,
                    &stripped,
                    options,
                    arbitrary,
                    index,
                    &mut reattach,
                )? {
                    return Ok(Some(found));
                }
            }
        }
        if let [replacement] = replacements {
            return self.try_parsed(replacement, accept);
        }
        if !arbitrary || !replacements.is_empty() {
            return Ok(None);
        }
        let Some(value) = function_value(candidate) else {
            return Ok(None);
        };
        if options.rem.is_some()
            && matches!(
                candidate.kind,
                CandidateKind::Functional {
                    value: Some(UtilityValue::Arbitrary { .. }),
                    ..
                }
            )
            && let Some(bare) = self.spacing_factor(value)
            && numeric::is_spacing_multiplier(&numeric::format(bare))
            && self.reasonable_spacing(bare, options.rem)
        {
            let mut replacement = candidate.clone_fresh();
            if let CandidateKind::Functional { value, .. } = &mut replacement.kind {
                *value = Some(UtilityValue::Named {
                    value: numeric::format(bare),
                    fraction: None,
                });
            }
            if let Some(found) = accept(self, replacement)? {
                return Ok(Some(found));
            }
        }
        let mut spacing = self.spacing_factor(value);
        let negative = spacing.is_some_and(|n| n < 0.0);
        if negative {
            spacing = spacing.map(f64::abs)
        }
        let mut roots = self.design_system.utility_keys(Kind::Functional);
        js_sort::sort_by(&mut roots, |a, b| {
            a.starts_with('-').cmp(&b.starts_with('-'))
        });
        for root in roots {
            let root = if negative { format!("-{root}") } else { root };
            if let Some(found) = self.try_parsed(&format!("{root}-{value}"), accept)? {
                return Ok(Some(found));
            }
            if modifier(candidate).is_some() {
                // Upstream interpolates the object here, rather than printModifier.
                if let Some(found) =
                    self.try_parsed(&format!("{root}-{value}[object Object]"), accept)?
                {
                    return Ok(Some(found));
                }
            }
            if let Some(spacing) = spacing.filter(|n| {
                numeric::is_spacing_multiplier(&numeric::format(*n))
                    && self.reasonable_spacing(*n, options.rem)
            }) {
                let name = format!("{root}-{}", numeric::format(spacing));
                if let Some(found) = self.try_parsed(&name, accept)? {
                    return Ok(Some(found));
                }
                if modifier(candidate).is_some()
                    && let Some(found) = self.try_parsed(
                        &format!("{name}{}", print::modifier(&modifier(candidate).cloned())),
                        accept,
                    )?
                {
                    return Ok(Some(found));
                }
            }
            let name = format!("{root}-[{value}]");
            if let Some(found) = self.try_parsed(&name, accept)? {
                return Ok(Some(found));
            }
            if modifier(candidate).is_some()
                && let Some(found) = self.try_parsed(
                    &format!("{name}{}", print::modifier(&modifier(candidate).cloned())),
                    accept,
                )?
            {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    fn variables_preserved(
        &mut self,
        candidate: &Candidate,
        replacement: &Candidate,
    ) -> Result<bool, String> {
        let value = match &candidate.kind {
            CandidateKind::Arbitrary { value, .. }
            | CandidateKind::Functional {
                value: Some(UtilityValue::Arbitrary { value, .. }),
                ..
            } if value.contains("var(--") => value,
            _ => return Ok(true),
        };
        let css = self
            .design_system
            .candidates_to_css(&[self.printed(replacement)])?
            .into_iter()
            .map(Option::unwrap_or_default)
            .collect::<Vec<_>>()
            .join("\n");
        fn walk(nodes: &[Value], css: &str) -> Result<bool, String> {
            for node in nodes {
                if let Value::Function { value, nodes } = node {
                    if value == "var" {
                        let Some(first) = nodes.first() else {
                            return Err(
                                "Cannot read properties of undefined (reading 'value')".into()
                            );
                        };
                        let variable = match first {
                            Value::Word { value }
                            | Value::Function { value, .. }
                            | Value::Separator { value } => value,
                        };
                        let pattern = format!(r"var\({variable}[,)]\s*");
                        let expression = crate::js_regex::compile(&pattern)?;
                        let css_utf16: Vec<_> = css.encode_utf16().collect();
                        if expression.find(&css_utf16)?.is_none()
                            || css.contains(&format!("{variable}:"))
                        {
                            return Ok(false);
                        }
                    }
                    if !walk(nodes, css)? {
                        return Ok(false);
                    }
                }
            }
            Ok(true)
        }
        walk(&value::parse(value), &css)
    }

    fn replace_arbitrary_utility(
        &mut self,
        candidate: Candidate,
        options: Options,
    ) -> Result<Candidate, String> {
        if !matches!(
            candidate.kind,
            CandidateKind::Arbitrary { .. }
                | CandidateKind::Functional {
                    value: Some(UtilityValue::Arbitrary { .. }),
                    ..
                }
        ) {
            return Ok(candidate);
        }
        let index = self.shared_utility_index(options);
        let Some(signature) = self.utility_signature(&self.printed(&candidate), options) else {
            return Ok(candidate);
        };
        let mut accept = |system: &mut Self, replacement: Candidate| {
            Ok(
                if system.same_candidates(&candidate, &replacement, options)
                    && system.variables_preserved(&candidate, &replacement)?
                {
                    Some(replacement)
                } else {
                    None
                },
            )
        };
        Ok(self
            .visit_replacements(&signature, &candidate, options, true, &index, &mut accept)?
            .unwrap_or(candidate))
    }

    fn replace_bare_utility(
        &mut self,
        candidate: Candidate,
        options: Options,
    ) -> Result<Candidate, String> {
        if !matches!(
            candidate.kind,
            CandidateKind::Functional {
                value: Some(UtilityValue::Named { .. }),
                ..
            }
        ) {
            return Ok(candidate);
        }
        let index = self.shared_utility_index(options);
        let Some(signature) = self.utility_signature(&self.printed(&candidate), options) else {
            return Ok(candidate);
        };
        let mut accept = |system: &mut Self, replacement: Candidate| {
            Ok(system
                .same_candidates(&candidate, &replacement, options)
                .then_some(replacement))
        };
        Ok(self
            .visit_replacements(&signature, &candidate, options, false, &index, &mut accept)?
            .unwrap_or(candidate))
    }

    fn replace_deprecated_utility(&mut self, candidate: Candidate, options: Options) -> Candidate {
        let input = self.printed(&candidate);
        let prefix = self
            .design_system
            .theme
            .prefix
            .as_ref()
            .filter(|p| !p.is_empty())
            .map(|p| format!("{p}:"));
        let input = prefix
            .as_ref()
            .and_then(|p| input.strip_prefix(p))
            .unwrap_or(&input);
        let mut replacements = Vec::new();
        match input {
            "order-none" => replacements.push("order-0".into()),
            "break-words" => replacements.push("wrap-break-word".into()),
            "overflow-ellipsis" => replacements.push("text-ellipsis".into()),
            _ => {}
        }
        for (old, new) in [("start-", "inset-s-"), ("end-", "inset-e-")] {
            let (sign, input) = input
                .strip_prefix('-')
                .map_or(("", input), |rest| ("-", rest));
            if let Some(rest) = input.strip_prefix(old)
                && !rest.contains(['\n', '\r', '\u{2028}', '\u{2029}'])
            {
                replacements.push(format!("{sign}{new}{rest}"))
            }
        }
        for replacement in replacements {
            if self.same_utility(&self.printed(&candidate), &replacement, options)
                && let Some(replacement) = self.parse_unprefixed(&replacement).into_iter().next()
            {
                return replacement;
            }
        }
        candidate
    }

    fn drop_data_type(&mut self, mut candidate: Candidate, options: Options) -> Candidate {
        if matches!(
            &candidate.kind,
            CandidateKind::Functional {
                value: Some(UtilityValue::Arbitrary {
                    data_type: Some(_),
                    ..
                }),
                ..
            }
        ) {
            let mut replacement = candidate.clone();
            if let CandidateKind::Functional {
                value: Some(UtilityValue::Arbitrary { data_type, .. }),
                ..
            } = &mut replacement.kind
            {
                *data_type = None
            }
            if self.signatures_equal(
                &self.printed(&candidate),
                &self.printed(&replacement),
                options,
            ) {
                self.design_system.update_shared_candidate(&replacement);
                candidate = replacement
            }
        }
        candidate
    }

    fn arbitrary_to_bare_utility(&mut self, candidate: Candidate, options: Options) -> Candidate {
        let CandidateKind::Functional {
            value: Some(UtilityValue::Arbitrary { value, .. }),
            ..
        } = &candidate.kind
        else {
            return candidate;
        };
        let original = self.printed(&candidate);
        for value in value_replacements(value) {
            let mut replacement = candidate.clone();
            if let CandidateKind::Functional { value: target, .. } = &mut replacement.kind {
                *target = Some(value)
            }
            if self.signatures_equal(&original, &self.printed(&replacement), options) {
                self.design_system.update_shared_candidate(&replacement);
                return replacement;
            }
        }
        candidate
    }

    fn optimize_arbitrary_expression(
        &mut self,
        mut candidate: Candidate,
        options: Options,
    ) -> Candidate {
        let CandidateKind::Functional {
            value: Some(UtilityValue::Arbitrary { value, .. }),
            ..
        } = &candidate.kind
        else {
            return candidate;
        };
        let mut ast = value::parse(value);
        if matches!(&ast[..], [Value::Function { value, .. }] if value == "calc")
            && declaration::constant_fold_ast(&mut ast, None, false)
        {
            let mut replacement = candidate.clone_fresh();
            *arbitrary_value(&mut replacement).unwrap() = value::to_css(&ast);
            if self.same_candidates(&candidate, &replacement, options) {
                candidate = replacement;
            }
        }
        if matches!(&candidate.kind, CandidateKind::Functional { root, .. } if root.starts_with('-'))
        {
            if matches!(&ast[..], [Value::Function { value, .. }] if value == "var") {
                return candidate;
            }
            let mut folded = value::parse(&format!(
                "calc({} * -1)",
                function_value(&candidate).unwrap()
            ));
            if declaration::constant_fold_ast(&mut folded, None, false) {
                let mut replacement = candidate.clone_fresh();
                if let CandidateKind::Functional { root, .. } = &mut replacement.kind {
                    *root = root[1..].into()
                }
                *arbitrary_value(&mut replacement).unwrap() = value::to_css(&folded);
                if self.same_candidates(&candidate, &replacement, options) {
                    candidate = replacement;
                    ast = folded
                }
            }
        }
        if let [Value::Function { value, nodes }] = &ast[..]
            && value == "calc"
            && nodes.len() == 5
            && matches!(&nodes[1], Value::Separator { value } if value == " ")
            && matches!(&nodes[2], Value::Word { value } if value == "*")
            && matches!(&nodes[3], Value::Separator { value } if value == " ")
        {
            let arg = if matches!(&nodes[4], Value::Word { value } if value == "-1") {
                Some(&nodes[0])
            } else if matches!(&nodes[0], Value::Word { value } if value == "-1") {
                Some(&nodes[4])
            } else {
                None
            };
            if let Some(arg) = arg {
                let mut replacement = candidate.clone_fresh();
                if let CandidateKind::Functional { root, .. } = &mut replacement.kind {
                    *root = format!("-{root}")
                }
                *arbitrary_value(&mut replacement).unwrap() =
                    value::to_css(std::slice::from_ref(arg));
                if self.same_candidates(&candidate, &replacement, options) {
                    candidate = replacement
                }
            }
        }
        candidate
    }

    fn optimize_modifier(&mut self, candidate: Candidate, options: Options) -> Candidate {
        let Some(modifier) = modifier(&candidate) else {
            return candidate;
        };
        let input = self.printed(&candidate);
        let mut replacement = candidate.clone();
        set_modifier(&mut replacement, None);
        if self.signatures_equal(&input, &self.printed(&replacement), options) {
            self.design_system.update_shared_candidate(&replacement);
            return replacement;
        }
        let modifier = match modifier {
            Modifier::Named { value } | Modifier::Arbitrary { value } => value,
        };
        let value = modifier.strip_suffix('%').map_or_else(
            || modifier.clone(),
            |v| {
                if modifier.contains('.') {
                    numeric::format(utilities::number(v))
                } else {
                    v.into()
                }
            },
        );
        set_modifier(&mut replacement, Some(Modifier::Named { value }));
        if self.signatures_equal(&input, &self.printed(&replacement), options) {
            self.design_system.update_shared_candidate(&replacement);
            return replacement;
        }
        static FLOAT: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^[+-]?(?:Infinity|(?:[0-9]+\.?[0-9]*|\.[0-9]+)(?:[eE][+-]?[0-9]+)?)")
                .unwrap()
        });
        let number = FLOAT
            .find(modifier.trim_start_matches(is_js_whitespace))
            .and_then(|m| m.as_str().parse::<f64>().ok())
            .unwrap_or(f64::NAN);
        set_modifier(
            &mut replacement,
            Some(Modifier::Named {
                value: numeric::format(number * 100.0),
            }),
        );
        if self.signatures_equal(&input, &self.printed(&replacement), options) {
            self.design_system.update_shared_candidate(&replacement);
            return replacement;
        }
        candidate
    }
}

fn value_replacements(input: &str) -> Vec<UtilityValue> {
    fn collect(input: &str, seen: &mut IndexSet<String>, output: &mut Vec<UtilityValue>) {
        if !seen.insert(input.into()) {
            return;
        }
        output.push(UtilityValue::Named {
            value: input.into(),
            fraction: None,
        });
        if let Some(number) = input
            .strip_suffix('%')
            .filter(|v| numeric::is_spacing_multiplier(v))
        {
            output.push(UtilityValue::Named {
                value: number.into(),
                fraction: None,
            });
        }
        let parts: Vec<_> = input.split('/').collect();
        if parts.len() >= 2
            && numeric::is_positive_integer(parts[0])
            && numeric::is_positive_integer(parts[1])
        {
            output.push(UtilityValue::Named {
                value: parts[0].into(),
                fraction: Some(format!("{}/{}", parts[0], parts[1])),
            });
        }
        static NUMBERS: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"([0-9]+/[0-9]+)|([0-9]+\.?[0-9]+)").unwrap());
        let mut numbers: Vec<_> = NUMBERS
            .find_iter(input)
            .map(|m| m.as_str().to_owned())
            .collect::<IndexSet<_>>()
            .into_iter()
            .collect();
        js_sort::sort_by(&mut numbers, |a, b| a.len().cmp(&b.len()));
        for number in numbers {
            collect(&number, seen, output)
        }
    }
    let mut output = Vec::new();
    collect(input, &mut IndexSet::new(), &mut output);
    output
}
