use crate::stylesheets::{Import, Stylesheets, digest};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::PathBuf};
use tailwind_canonical_native::{
    canonicalize::{CanonicalizeOptions, Canonicalizer},
    configuration,
    syntax::is_js_whitespace,
};

const CACHE_VERSION: u32 = 1;

#[derive(Deserialize, Serialize)]
pub struct Cache {
    version: u32,
    executable: String,
    entrypoint: PathBuf,
    graph: String,
    files: BTreeMap<PathBuf, String>,
    imports: Vec<Import>,
    values: BTreeMap<String, Option<String>>,
    pub sources: BTreeMap<PathBuf, String>,
}

pub struct Runtime {
    entrypoint: PathBuf,
    executable: String,
    stylesheets: Stylesheets,
    engine: Option<Canonicalizer>,
    values: BTreeMap<String, Option<String>>,
    pub sources: BTreeMap<PathBuf, String>,
}

impl Runtime {
    pub fn new(entrypoint: PathBuf) -> Result<Self, String> {
        let executable = std::env::current_exe()
            .and_then(fs::read)
            .map_err(|error| format!("Falha ao identificar o executável: {error}"))?;
        Ok(Self {
            entrypoint,
            executable: digest(&executable),
            stylesheets: Stylesheets::default(),
            engine: None,
            values: BTreeMap::new(),
            sources: BTreeMap::new(),
        })
    }

    pub fn restore(&mut self, content: &[u8]) {
        let Ok(cache) = serde_json::from_slice::<Cache>(content) else {
            return;
        };
        if cache.version != CACHE_VERSION
            || cache.executable != self.executable
            || cache.entrypoint != self.entrypoint
            || !cache.files.contains_key(&self.entrypoint)
        {
            return;
        }
        let mut stylesheets = Stylesheets::default();
        for (path, hash) in &cache.files {
            let Ok(content) = stylesheets.read(path) else {
                return;
            };
            if digest(content.as_bytes()) != *hash {
                return;
            }
        }
        for edge in &cache.imports {
            if stylesheets
                .resolve(&edge.id, edge.base.as_ref())
                .ok()
                .as_ref()
                != Some(&edge.path)
            {
                return;
            }
        }
        stylesheets.imports = cache.imports;
        if stylesheets.fingerprint() != cache.graph {
            return;
        }
        self.stylesheets = stylesheets;
        self.values = cache.values;
        self.sources = cache.sources;
    }

    pub fn snapshot(&self) -> Option<Cache> {
        if self.stylesheets.files.is_empty() {
            return None;
        }
        Some(Cache {
            version: CACHE_VERSION,
            executable: self.executable.clone(),
            entrypoint: self.entrypoint.clone(),
            graph: self.stylesheets.fingerprint(),
            files: self
                .stylesheets
                .files
                .iter()
                .map(|(path, content)| (path.clone(), digest(content.as_bytes())))
                .collect(),
            imports: self.stylesheets.imports.clone(),
            values: self.values.clone(),
            sources: self.sources.clone(),
        })
    }

    fn engine(&mut self) -> Result<&mut Canonicalizer, String> {
        if self.engine.is_none() {
            let css = self.stylesheets.read(&self.entrypoint)?;
            let base = self
                .entrypoint
                .parent()
                .and_then(|path| path.to_str())
                .ok_or_else(|| "O caminho do CSS deve ser UTF-8.".to_owned())?;
            self.stylesheets.imports.clear();
            let configuration =
                configuration::load(&css, base, &mut |id, base| self.stylesheets.load(id, base))?;
            self.engine = Some(Canonicalizer::new(configuration.design_system));
        }
        Ok(self.engine.as_mut().unwrap())
    }

    pub fn canonicalize(&mut self, value: &str) -> Result<Option<String>, String> {
        if let Some(cached) = self.values.get(value) {
            return Ok(cached.clone());
        }
        let classes: Vec<String> = value
            .split(is_js_whitespace)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect();
        if classes.is_empty() {
            self.values.insert(value.into(), None);
            return Ok(None);
        }
        let original = classes.join(" ");
        let engine = self.engine()?;
        let canonical = engine.canonicalize(
            &classes,
            CanonicalizeOptions {
                rem: Some(16.0),
                collapse: true,
                logical_to_physical: true,
            },
        )?;
        let mut ordered = engine.design_system.get_class_order(&canonical)?;
        ordered.sort_by_key(|(_, order)| *order);
        let canonical = ordered
            .into_iter()
            .map(|(class, _)| class)
            .collect::<Vec<_>>()
            .join(" ");
        let canonical = (canonical != original).then_some(canonical);
        self.values.insert(value.into(), canonical.clone());
        Ok(canonical)
    }
}
