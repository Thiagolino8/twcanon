use super::{Canonicalizer, OptionsKey};
use crate::{print, signature, syntax::segment::segment};
use indexmap::IndexSet;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalizeOptions {
    pub rem: Option<f64>,
    #[serde(default)]
    pub collapse: bool,
    #[serde(default)]
    pub logical_to_physical: bool,
}
impl CanonicalizeOptions {
    pub(super) fn signature(self) -> signature::Options {
        signature::Options {
            rem: self.rem,
            features: (u8::from(self.collapse) * signature::EXPAND_PROPERTIES)
                | (u8::from(self.logical_to_physical) * signature::LOGICAL_TO_PHYSICAL),
        }
    }
}

impl Canonicalizer {
    /// Canonicalize and deduplicate classes, retaining their insertion order.
    /// With collapse enabled, equivalent groups can become a single utility.
    pub fn canonicalize(
        &mut self,
        candidates: &[String],
        options: CanonicalizeOptions,
    ) -> Result<Vec<String>, String> {
        let signature_options = options.signature();
        self.prepare_storage(options.rem);
        let key = signature_options.into();
        let prefix = self
            .cache_prefix
            .entry(key)
            .or_insert_with(|| {
                self.design_system
                    .theme
                    .prefix
                    .as_ref()
                    .filter(|prefix| !prefix.is_empty())
                    .map(|prefix| format!("{prefix}:"))
                    .unwrap_or_default()
            })
            .clone();
        let mut result = IndexSet::new();
        for candidate in candidates {
            result.insert(self.canonical_candidate(candidate, signature_options, &prefix)?);
        }
        let result: Vec<_> = result.into_iter().collect();
        if result.len() <= 1 || !options.collapse {
            return Ok(result);
        }
        self.collapse_candidates(result, signature_options)
    }

    fn canonical_candidate(
        &mut self,
        input: &str,
        options: signature::Options,
        prefix: &str,
    ) -> Result<String, String> {
        let key = options.into();
        if let Some(result) = self
            .candidate_cache
            .get(&key)
            .and_then(|cache| cache.get(input))
        {
            return Ok(result.clone());
        }
        let result = self.compute_candidate(input, options, prefix)?;
        self.candidate_cache
            .entry(key)
            .or_default()
            .insert(input.into(), result.clone());
        Ok(result)
    }

    fn compute_candidate(
        &mut self,
        input: &str,
        options: signature::Options,
        prefix: &str,
    ) -> Result<String, String> {
        let key: OptionsKey = options.into();
        for mut candidate in self.design_system.parse_candidate(input) {
            let mut names = segment(input, b':');
            names.pop();
            if self.design_system.theme.prefix.is_some() && !names.is_empty() {
                names.remove(0);
            }
            let mut variants = Vec::new();
            for (index, variant) in candidate.variants.iter().rev().enumerate() {
                // parseCandidate reuses the interned parseVariant object for this
                // original spelling. Distinct spellings remain distinct cache keys.
                let name = names
                    .get(index)
                    .expect("parsed variant has an original spelling");
                let transformed = if let Some(result) = self
                    .variant_cache
                    .get(&key)
                    .and_then(|cache| cache.get(*name))
                {
                    result.clone()
                } else {
                    let result = self.transform_variant(variant.clone(), options)?;
                    self.variant_cache
                        .entry(key)
                        .or_default()
                        .insert((*name).into(), result.clone());
                    result
                };
                variants.extend(transformed);
            }
            if candidate.important || !variants.is_empty() {
                let important = candidate.important;
                candidate.variants.clear();
                candidate.important = false;
                let base = print::candidate(&candidate, self.design_system.theme.prefix.as_deref());
                let mut result = self.canonical_candidate(&base, options, prefix)?;
                if self.design_system.theme.prefix.is_some() && !variants.is_empty() {
                    result = result.chars().skip(prefix.chars().count()).collect();
                }
                if !variants.is_empty() {
                    result = format!(
                        "{}:{result}",
                        variants
                            .iter()
                            .map(print::variant)
                            .collect::<Vec<_>>()
                            .join(":")
                    );
                }
                if important {
                    result.push('!');
                }
                if self.design_system.theme.prefix.is_some() && !variants.is_empty() {
                    result = format!("{prefix}{result}");
                }
                return Ok(result);
            }
            let result = self.canonical_utility(input, options)?;
            if result != input {
                return Ok(result);
            }
        }
        Ok(input.into())
    }

    fn canonical_utility(
        &mut self,
        input: &str,
        options: signature::Options,
    ) -> Result<String, String> {
        let key = options.into();
        if let Some(result) = self
            .utility_cache
            .get(&key)
            .and_then(|cache| cache.get(input))
        {
            return Ok(result.clone());
        }
        let mut result = input.to_owned();
        for candidate in self.design_system.parse_candidate(input) {
            let replacement = self.transform_utility(candidate, options)?;
            let printed =
                print::candidate(&replacement, self.design_system.theme.prefix.as_deref());
            if printed != input {
                result = printed;
                break;
            }
        }
        self.utility_cache
            .entry(key)
            .or_default()
            .insert(input.into(), result.clone());
        Ok(result)
    }
}
