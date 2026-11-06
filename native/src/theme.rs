// Port of Tailwind CSS 4.3.4 theme.ts. See LICENSE.tailwind.
use crate::{
    ast::AstNode,
    syntax::escape::{escape, unescape},
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const INLINE: u8 = 1;
pub const REFERENCE: u8 = 2;
pub const DEFAULT: u8 = 4;
pub const STATIC: u8 = 8;
pub const USED: u8 = 16;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub value: String,
    pub options: u8,
}

#[derive(Clone, Debug, Default)]
pub struct Theme {
    pub prefix: Option<String>,
    values: IndexMap<String, Entry>,
    keyframes: Vec<Arc<AstNode>>,
    ignored_options: u8,
    value_revision: Arc<()>,
}

fn ignored(namespace: &str) -> &'static [&'static str] {
    match namespace {
        "--font" => &["--font-weight", "--font-size"],
        "--inset" => &["--inset-shadow", "--inset-ring"],
        "--text" => &[
            "--text-color",
            "--text-decoration-color",
            "--text-decoration-thickness",
            "--text-indent",
            "--text-shadow",
            "--text-underline-offset",
        ],
        "--grid-column" => &["--grid-column-start", "--grid-column-end"],
        "--grid-row" => &["--grid-row-start", "--grid-row-end"],
        _ => &[],
    }
}

fn is_ignored(key: &str, namespace: &str) -> bool {
    ignored(namespace)
        .iter()
        .any(|n| key == *n || key.strip_prefix(n).is_some_and(|s| s.starts_with('-')))
}

fn utf16_slice(value: &str, start: usize) -> Option<String> {
    String::from_utf16(&value.encode_utf16().skip(start).collect::<Vec<_>>()).ok()
}

impl Theme {
    pub(crate) fn value_revision(&self) -> (&Arc<()>, u8) {
        (&self.value_revision, self.ignored_options)
    }

    // Signature compilation temporarily ignores INLINE without rewriting the theme.
    // Restoring the previous mask also preserves nested signature compilation.
    pub(crate) fn disable_inline(&mut self) -> u8 {
        std::mem::replace(&mut self.ignored_options, INLINE)
    }

    pub(crate) fn restore_inline(&mut self, previous: u8) {
        self.ignored_options = previous;
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn add(&mut self, key: &str, value: &str, options: u8) -> Result<(), String> {
        // A cache keeps a strong reference to this token. Copy-on-write gives
        // every subsequent mutation a distinct revision, including cloned themes.
        Arc::make_mut(&mut self.value_revision);
        if let Some(namespace) = key.strip_suffix("-*") {
            if value != "initial" {
                return Err(format!(
                    "Invalid theme value `{value}` for namespace `{key}`"
                ));
            }
            if key == "--*" {
                self.values.clear();
            } else {
                self.clear_namespace(namespace, 0);
            }
        }
        if options & DEFAULT != 0
            && self
                .values
                .get(key)
                .is_some_and(|e| e.options & DEFAULT == 0)
        {
            return Ok(());
        }
        if value == "initial" {
            self.values.shift_remove(key);
        } else {
            self.values.insert(
                key.into(),
                Entry {
                    value: value.into(),
                    options,
                },
            );
        }
        Ok(())
    }

    pub fn keys_in_namespaces(&self, namespaces: &[String]) -> Vec<String> {
        let mut keys = Vec::new();
        for namespace in namespaces {
            let prefix = format!("{namespace}-");
            for key in self.values.keys() {
                if let Some(suffix) = key.strip_prefix(&prefix)
                    && !key
                        .encode_utf16()
                        .skip(2)
                        .collect::<Vec<_>>()
                        .windows(2)
                        .any(|pair| pair == [45, 45])
                    && !is_ignored(key, namespace)
                {
                    keys.push(suffix.into());
                }
            }
        }
        keys
    }

    pub fn get(&self, keys: &[String]) -> Option<&str> {
        keys.iter()
            .find_map(|k| self.values.get(k).map(|e| e.value.as_str()))
    }

    pub fn has_default(&self, key: &str) -> bool {
        self.get_options(key) & DEFAULT == DEFAULT
    }

    pub fn get_options(&self, key: &str) -> u8 {
        self.unprefix_key(key)
            .and_then(|key| self.values.get(&unescape(&key)))
            .map_or(0, |e| e.options & !self.ignored_options)
    }

    pub fn entries(&self) -> Result<Vec<(String, Entry)>, String> {
        self.values
            .iter()
            .map(|(key, value)| {
                let mut value = value.clone();
                value.options &= !self.ignored_options;
                Ok((self.prefix_key(key)?, value))
            })
            .collect()
    }

    pub fn prefix_key(&self, key: &str) -> Result<String, String> {
        match self.prefix.as_deref().filter(|p| !p.is_empty()) {
            Some(prefix) => utf16_slice(key, 2)
                .map(|suffix| format!("--{prefix}-{suffix}"))
                .ok_or_else(|| "Theme key prefixing produced an unpaired UTF-16 surrogate".into()),
            None => Ok(key.into()),
        }
    }

    fn unprefix_key(&self, key: &str) -> Option<String> {
        match self.prefix.as_deref().filter(|p| !p.is_empty()) {
            Some(prefix) => utf16_slice(key, 3 + prefix.encode_utf16().count())
                .map(|suffix| format!("--{suffix}")),
            None => Some(key.into()),
        }
    }

    pub fn clear_namespace(&mut self, namespace: &str, options: u8) {
        Arc::make_mut(&mut self.value_revision);
        let keys: Vec<String> = self
            .values
            .keys()
            .filter(|k| {
                k.starts_with(namespace)
                    && (options == 0 || self.get_options(k) & options == options)
                    && !ignored(namespace).iter().any(|n| k.starts_with(n))
            })
            .cloned()
            .collect();
        for key in keys {
            self.values.shift_remove(&key);
        }
    }

    fn resolve_key(&self, candidate: Option<&str>, namespaces: &[String]) -> Option<String> {
        for namespace in namespaces {
            let mut key = match candidate {
                Some(c) => format!("{namespace}-{c}"),
                None => namespace.clone(),
            };
            if !self.values.contains_key(&key) {
                if let Some(candidate) = candidate.filter(|c| c.contains('.')) {
                    key = format!("{namespace}-{}", candidate.replace('.', "_"));
                    if !self.values.contains_key(&key) {
                        continue;
                    }
                } else {
                    continue;
                }
            }
            if !is_ignored(&key, namespace) {
                return Some(key);
            }
        }
        None
    }

    fn variable(&self, key: &str) -> Option<String> {
        let entry = self.values.get(key)?;
        let fallback = if entry.options & REFERENCE != 0 && !entry.value.is_empty() {
            format!(", {}", entry.value)
        } else {
            String::new()
        };
        Some(format!(
            "var({}{fallback})",
            escape(&self.prefix_key(key).ok()?)
        ))
    }

    pub fn mark_used_variable(&mut self, key: &str) -> bool {
        let Some(key) = self.unprefix_key(key) else {
            return false;
        };
        let key = unescape(&key);
        let Some(entry) = self.values.get_mut(&key) else {
            return false;
        };
        let used = entry.options & USED != 0;
        if !used {
            Arc::make_mut(&mut self.value_revision);
        }
        entry.options |= USED;
        !used
    }

    pub fn resolve(
        &self,
        candidate: Option<&str>,
        namespaces: &[String],
        options: u8,
    ) -> Option<String> {
        let key = self.resolve_key(candidate, namespaces)?;
        let entry = &self.values[&key];
        if (options | (entry.options & !self.ignored_options)) & INLINE != 0 {
            Some(entry.value.clone())
        } else {
            self.variable(&key)
        }
    }

    pub fn resolve_value(&self, candidate: Option<&str>, namespaces: &[String]) -> Option<&str> {
        self.values
            .get(&self.resolve_key(candidate, namespaces)?)
            .map(|e| e.value.as_str())
    }

    pub fn resolve_with(
        &self,
        candidate: &str,
        namespaces: &[String],
        nested: &[String],
    ) -> Option<(String, IndexMap<String, String>)> {
        let key = self.resolve_key(Some(candidate), namespaces)?;
        let mut extra = IndexMap::new();
        for name in nested {
            let nested = format!("{key}{name}");
            let Some(entry) = self.values.get(&nested) else {
                continue;
            };
            extra.insert(
                name.clone(),
                if entry.options & !self.ignored_options & INLINE != 0 {
                    entry.value.clone()
                } else {
                    self.variable(&nested)?
                },
            );
        }
        let entry = &self.values[&key];
        Some((
            if entry.options & !self.ignored_options & INLINE != 0 {
                entry.value.clone()
            } else {
                self.variable(&key)?
            },
            extra,
        ))
    }

    pub fn namespace(&self, namespace: &str) -> Vec<(Option<String>, String)> {
        let prefix = format!("{namespace}-");
        self.values
            .iter()
            .filter_map(|(key, entry)| {
                if key == namespace {
                    Some((None, entry.value.clone()))
                } else if key.starts_with(&format!("{prefix}-")) {
                    Some((Some(key[namespace.len()..].into()), entry.value.clone()))
                } else {
                    key.strip_prefix(&prefix)
                        .map(|k| (Some(k.into()), entry.value.clone()))
                }
            })
            .collect()
    }

    pub fn add_keyframes(&mut self, value: Arc<AstNode>) {
        if !self.keyframes.iter().any(|node| Arc::ptr_eq(node, &value)) {
            self.keyframes.push(value);
        }
    }
    pub fn keyframes(&self) -> &[Arc<AstNode>] {
        &self.keyframes
    }
}
