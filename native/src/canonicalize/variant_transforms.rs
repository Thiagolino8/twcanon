// Variant canonicalization passes from Tailwind CSS 4.3.4.
use super::{
    Canonicalizer,
    converter::{self, Convert},
};
use crate::{
    candidate::{Variant, VariantValue},
    numeric, print,
    signature::Options,
    syntax::{
        attribute, is_js_whitespace,
        segment::segment,
        selector::{self, Selector},
        value::{self, Value},
    },
    variant_cache::Identity,
};

fn text(node: &Selector) -> Option<&str> {
    match node {
        Selector::Selector { value }
        | Selector::Function { value, .. }
        | Selector::Value { value }
        | Selector::Combinator { value } => Some(value),
        _ => None,
    }
}
fn is_attribute(node: &Selector) -> bool {
    text(node).is_some_and(|v| v.starts_with('[') && v.ends_with(']'))
}
fn single(nodes: &[Selector]) -> bool {
    !matches!(nodes, [Selector::List { .. }])
}

fn convert_theme(variant: &mut Variant, theme: &crate::theme::Theme) -> Result<(), String> {
    match variant {
        Variant::Arbitrary { selector, .. }
        | Variant::Functional {
            value: Some(VariantValue::Arbitrary { value: selector }),
            ..
        } => {
            *selector = converter::convert(theme, selector, Convert::MigrateThemeOnly)?.0;
        }
        Variant::Compound { variant, .. } => convert_theme(variant, theme)?,
        _ => {}
    }
    Ok(())
}

fn arbitrary_to_bare(variant: &mut Variant) {
    if let Variant::Functional {
        root,
        value: Some(value),
        ..
    } = variant
        && let VariantValue::Arbitrary { value: input } = value
    {
        let named = match root.as_str() {
            "data" if !input.contains('=') => Some(input.clone()),
            "aria"
                if ["=true", "=\"true\"", "='true'"]
                    .iter()
                    .any(|s| input.ends_with(s)) =>
            {
                let key = segment(input, b'=').first().copied().unwrap_or("");
                if key.ends_with(['~', '|', '^', '$', '*']) {
                    None
                } else {
                    Some(input[..input.find('=').unwrap()].into())
                }
            }
            "supports"
                if input
                    .as_bytes()
                    .first()
                    .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'-')
                    && input
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-') =>
            {
                Some(input.clone())
            }
            _ => None,
        };
        if let Some(value_string) = named {
            *value = VariantValue::Named {
                value: value_string,
            };
        }
    }
    if let Variant::Compound { variant, .. } = variant {
        arbitrary_to_bare(variant);
    }
}

fn remove_nesting(nodes: &mut Vec<Selector>) {
    let mut i = 0;
    while i < nodes.len() {
        if matches!(&nodes[i], Selector::Selector { value } if value == "&") {
            nodes.remove(i);
            continue;
        }
        if !matches!(&nodes[i], Selector::Function { .. }) {
            if let Some(children) = nodes[i].children_mut() {
                remove_nesting(children);
            }
            if matches!(&nodes[i], Selector::Compound { nodes } if nodes.len() == 1) {
                let Selector::Compound {
                    nodes: mut children,
                } = nodes.remove(i)
                else {
                    unreachable!()
                };
                nodes.insert(i, children.remove(0));
            }
        }
        i += 1;
    }
}

fn remove_not(nodes: &mut Vec<Value>) -> bool {
    let mut found = false;
    nodes.retain_mut(|node| {
        if matches!(node, Value::Word { value } if value == "not") {
            found = true;
            return false;
        }
        if let Value::Function { nodes, .. } = node {
            found |= remove_not(nodes);
        }
        true
    });
    found
}
fn normalize_whitespace(nodes: &mut [Value]) {
    for node in nodes {
        match node {
            Value::Separator { value } if value != " " && value.chars().all(is_js_whitespace) => {
                *value = " ".into()
            }
            Value::Function { nodes, .. } => normalize_whitespace(nodes),
            _ => {}
        }
    }
}

impl Canonicalizer {
    pub fn transform_variant(
        &mut self,
        mut variant: Variant,
        options: Options,
    ) -> Result<Vec<Variant>, String> {
        self.prepare_storage(options.rem);
        convert_theme(&mut variant, &self.design_system.theme)?;
        arbitrary_to_bare(&mut variant);
        let prefix = self.modernize_variant(&mut variant, true, options.rem, None)?;
        let mut variants = prefix.into_iter().chain([variant]).collect::<Vec<_>>();
        for variant in &mut variants {
            self.replace_arbitrary_variant(variant, options.rem);
        }
        Ok(variants)
    }

    fn replace_arbitrary_variant(&mut self, variant: &mut Variant, rem: Option<f64>) {
        // The upstream initializes the index before walking, including compounds.
        let index = self.shared_variant_index(rem);
        self.replace_variant_from_index(variant, rem, &index, None);
    }
    fn replace_variant_from_index(
        &mut self,
        variant: &mut Variant,
        rem: Option<f64>,
        index: &super::SignatureIndex,
        source: Option<Identity>,
    ) {
        let mut child_source = source.and_then(|id| self.design_system.shared_variant_child(id));
        if !matches!(variant, Variant::Compound { .. }) {
            let input = print::variant(variant);
            if let Some(signature) = self.variant_signature(&input, rem)
                && let Some(found) = index.get(&signature)
                && !found.contains(&input)
                && let Some(first) = found.first()
                && let Some(replacement) = self.design_system.parse_variant(first)
            {
                child_source = self
                    .design_system
                    .shared_variant(first)
                    .and_then(|id| self.design_system.shared_variant_child(id));
                *variant = replacement;
            }
        }
        if let Some(source) = source {
            self.design_system
                .replace_shared_variant(source, variant, child_source);
        }
        if let Variant::Compound { variant, .. } = variant {
            self.replace_variant_from_index(variant, rem, index, child_source);
        }
    }

    fn modernize_variant(
        &mut self,
        variant: &mut Variant,
        top: bool,
        rem: Option<f64>,
        source: Option<Identity>,
    ) -> Result<Option<Variant>, String> {
        let mut child_source = source.and_then(|id| self.design_system.shared_variant_child(id));
        if let Variant::Compound {
            root,
            modifier,
            variant,
        } = variant
            && matches!(root.as_str(), "has" | "not" | "in")
            && modifier.is_some()
            && let Variant::Functional {
                modifier: child, ..
            }
            | Variant::Compound {
                modifier: child, ..
            } = variant.as_mut()
        {
            *child = modifier.take();
            if let Some(child_source) = child_source {
                self.design_system.replace_shared_variant(
                    child_source,
                    variant,
                    self.design_system.shared_variant_child(child_source),
                );
            }
        }
        let mut replacement_source = None;
        let prefix =
            self.modernize_arbitrary_variant(variant, top, rem, &mut replacement_source)?;
        if let Some(replacement_source) = replacement_source {
            child_source = self.design_system.shared_variant_child(replacement_source);
        }
        if !matches!(variant, Variant::Compound { .. }) {
            child_source = None;
        }
        if let Some(source) = source {
            self.design_system
                .replace_shared_variant(source, variant, child_source);
        }
        if prefix.is_some() {
            return Ok(prefix);
        }
        if let Variant::Compound { variant, .. } = variant {
            self.modernize_variant(variant, false, rem, child_source)?;
        }
        Ok(None)
    }

    fn modernize_arbitrary_variant(
        &mut self,
        variant: &mut Variant,
        top: bool,
        rem: Option<f64>,
        source: &mut Option<Identity>,
    ) -> Result<Option<Variant>, String> {
        let Variant::Arbitrary {
            selector: input,
            relative: false,
        } = variant
        else {
            return Ok(None);
        };
        let mut nodes = selector::parse(input);
        if !single(&nodes) {
            return Ok(None);
        }
        if matches!(&nodes[..], [Selector::Complex { .. }]) {
            let Selector::Complex { nodes: children } = nodes.remove(0) else {
                unreachable!()
            };
            nodes = children;
        }
        if top {
            if let [
                Selector::Selector { value: left },
                Selector::Combinator { value: combinator },
                Selector::Selector { value: right },
            ] = &nodes[..]
                && left == "&"
                && right == "*"
                && matches!(combinator.as_str(), ">" | " ")
            {
                if let Some(replacement) =
                    self.design_system
                        .parse_variant(if combinator == ">" { "*" } else { "**" })
                {
                    *variant = replacement;
                }
                return Ok(None);
            }
            if let [Selector::Compound { nodes: children }] = &nodes[..]
                && let [
                    Selector::Selector { value },
                    Selector::Function {
                        value: function,
                        nodes: args,
                    },
                ] = &children[..]
                && value == "&"
                && function == ":has"
                && matches!(&args[..], [Selector::Selector { .. }])
            {
                let input = format!("has-[{}]", selector::to_css(args, true));
                *variant = self
                    .design_system
                    .parse_variant(&input)
                    .ok_or_else(|| "Unknown variant kind".to_string())?;
                *source = self.design_system.shared_variant(&input);
                return Ok(None);
            }
            if let [
                Selector::Selector { .. },
                Selector::Combinator { value: combinator },
                Selector::Selector { value },
            ] = &nodes[..]
                && combinator == " "
                && value == "&"
            {
                let input = format!("in-[{}]", selector::to_css(&nodes[..1], true));
                *variant = self
                    .design_system
                    .parse_variant(&input)
                    .ok_or_else(|| "Unknown variant kind".to_string())?;
                *source = self.design_system.shared_variant(&input);
                return Ok(None);
            }
            if matches!(nodes.first(), Some(Selector::Selector { value }) if value == "@media" || value == "@supports")
            {
                let target = print::variant(variant);
                let mut parsed = value::parse(&selector::to_css(&nodes, true));
                let contains_not = remove_not(&mut parsed);
                parsed = value::parse(&value::to_css(&parsed));
                normalize_whitespace(&mut parsed);
                if contains_not
                    && let Some(replacement) = self
                        .design_system
                        .parse_variant(&format!("not-[{}]", value::to_css(&parsed)))
                    && self.variant_signatures_equal(&target, &print::variant(&replacement), rem)
                {
                    *source = self
                        .design_system
                        .shared_variant(&format!("not-[{}]", value::to_css(&parsed)));
                    *variant = replacement;
                    return Ok(None);
                }
            }
        }
        let mut prefix = None;
        if top
            && let [
                Selector::Selector { value: left },
                Selector::Combinator { value: combinator },
                target @ Selector::Selector { .. },
            ] = &nodes[..]
            && left == "&"
            && matches!(combinator.as_str(), ">" | " ")
            && (text(target).is_some_and(|value| value.starts_with(':')) || is_attribute(target))
        {
            prefix = self
                .design_system
                .parse_variant(if combinator == ">" { "*" } else { "**" });
            nodes = vec![target.clone()];
        }
        remove_nesting(&mut nodes);
        let [target] = nodes.as_slice() else {
            return Ok(None);
        };
        let mut target = target;
        if let Selector::Function { value, nodes } = target
            && value == ":is"
        {
            if !single(nodes) || nodes.len() != 1 {
                return Ok(None);
            }
            if matches!(&nodes[0], Selector::Selector { .. }) && !is_attribute(&nodes[0]) {
                return Ok(None);
            }
            target = &nodes[0];
        }
        if matches!(
            target,
            Selector::Function { .. } | Selector::Selector { .. }
        ) && text(target).is_some_and(|value| value.starts_with(':'))
        {
            if !self.modernize_pseudo(variant, target, prefix.as_ref(), rem, source) {
                return Ok(None);
            }
        } else if matches!(target, Selector::Selector { .. }) && is_attribute(target) {
            let Some(attribute) = attribute::parse(text(target).unwrap()) else {
                return Ok(None);
            };
            if let Some(name) = attribute.attribute.strip_prefix("data-") {
                *variant = Variant::Functional {
                    root: "data".into(),
                    modifier: None,
                    value: Some(match &attribute.value {
                        None => VariantValue::Named { value: name.into() },
                        Some(value) => VariantValue::Arbitrary {
                            value: attribute_value(name, value, &attribute),
                        },
                    }),
                };
            } else if let Some(name) = attribute.attribute.strip_prefix("aria-") {
                *variant = Variant::Functional {
                    root: "aria".into(),
                    modifier: None,
                    value: Some(match &attribute.value {
                        None => VariantValue::Arbitrary { value: name.into() },
                        Some(value)
                            if attribute.operator.as_deref() == Some("=")
                                && value == "true"
                                && attribute.sensitivity.is_none() =>
                        {
                            VariantValue::Named { value: name.into() }
                        }
                        Some(value) => VariantValue::Arbitrary {
                            value: attribute_value(&attribute.attribute, value, &attribute),
                        },
                    }),
                };
            } else {
                *variant = Variant::Arbitrary {
                    selector: text(target).unwrap().into(),
                    relative: false,
                };
            }
        }
        Ok(prefix)
    }

    // False corresponds to the upstream's `continue`, which also skips a pending prefix.
    fn modernize_pseudo(
        &mut self,
        variant: &mut Variant,
        target: &Selector,
        prefix: Option<&Variant>,
        rem: Option<f64>,
        source: &mut Option<Identity>,
    ) -> bool {
        let mut node = target;
        let mut compound_not = false;
        if let Selector::Function { value, nodes } = target
            && value == ":not"
        {
            compound_not = true;
            let [child] = nodes.as_slice() else {
                return false;
            };
            if !matches!(child, Selector::Selector { .. } | Selector::Function { .. })
                || !text(child).is_some_and(|v| v.starts_with(':'))
            {
                return false;
            }
            node = child;
        }
        let value = text(node).unwrap();
        let mut new_variant = None;
        if let Selector::Function { nodes, .. } = node {
            if value == ":nth-child"
                && let [Selector::Value { value }] = &nodes[..]
                && matches!(value.as_str(), "odd" | "even")
            {
                new_variant = Some(
                    if compound_not {
                        compound_not = false;
                        if value == "odd" { "even" } else { "odd" }
                    } else {
                        value
                    }
                    .to_owned(),
                );
            }
            if new_variant.is_none() {
                for (selector, root) in [
                    (":nth-child", "nth"),
                    (":nth-last-child", "nth-last"),
                    (":nth-of-type", "nth-of-type"),
                    (":nth-last-of-type", "nth-of-last-type"),
                ] {
                    if value == selector && nodes.len() == 1 {
                        new_variant = Some(
                            if let [Selector::Value { value }] = &nodes[..]
                                && numeric::is_positive_integer(value)
                            {
                                format!("{root}-{value}")
                            } else {
                                format!("{root}-[{}]", selector::to_css(nodes, true))
                            },
                        );
                        break;
                    }
                }
            }
        }
        if new_variant.is_none()
            && compound_not
            && self.variant_signatures_equal(
                &print::variant(variant),
                &format!("not-[{value}]"),
                rem,
            )
        {
            new_variant = Some(format!("[&{value}]"));
        }
        let Some(mut input) = new_variant else {
            if prefix.is_some() {
                *variant = Variant::Arbitrary {
                    selector: if target.is_pseudo_element() {
                        format!("&{}", text(target).unwrap())
                    } else {
                        text(target).unwrap().into()
                    },
                    relative: false,
                };
                return true;
            }
            return false;
        };
        if compound_not {
            input = format!("not-{input}");
        }
        let Some(replacement) = self.design_system.parse_variant(&input) else {
            return false;
        };
        *source = self.design_system.shared_variant(&input);
        *variant = replacement;
        true
    }

    fn variant_signatures_equal(&mut self, left: &str, right: &str, rem: Option<f64>) -> bool {
        left == right
            || matches!((self.variant_signature(left, rem), self.variant_signature(right, rem)), (Some(left), Some(right)) if left == right)
    }
}

fn attribute_value(name: &str, value: &str, attribute: &attribute::Attribute) -> String {
    format!(
        "{name}{}{quote}{value}{quote}{}",
        attribute.operator.as_deref().unwrap_or("null"),
        attribute
            .sensitivity
            .map(|s| format!(" {s}"))
            .unwrap_or_default(),
        quote = attribute.quote.map(|q| q.to_string()).unwrap_or_default()
    )
}
