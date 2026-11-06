use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use tailwind_canonical_native::imports::LoadedStylesheet;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Import {
    pub id: String,
    pub base: String,
    pub path: PathBuf,
}

#[derive(Default)]
pub struct Stylesheets {
    pub files: BTreeMap<PathBuf, String>,
    pub imports: Vec<Import>,
}

pub fn digest(contents: &[u8]) -> String {
    format!("{:x}", Sha256::digest(contents))
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("Falha ao ler {}: {error}", path.display()))
}

impl Stylesheets {
    pub fn read(&mut self, path: &Path) -> Result<String, String> {
        if let Some(content) = self.files.get(path) {
            return Ok(content.clone());
        }
        let content = read(path)?;
        self.files.insert(path.to_owned(), content.clone());
        Ok(content)
    }

    pub fn resolve(&mut self, id: &str, base: &Path) -> Result<PathBuf, String> {
        if id.starts_with('.') || Path::new(id).is_absolute() {
            let filename = if id.ends_with(".css") {
                id.to_owned()
            } else {
                format!("{id}.css")
            };
            return base.join(filename).canonicalize().map_err(|error| {
                format!(
                    "Stylesheet não encontrado: {id} em {}: {error}",
                    base.display()
                )
            });
        }
        let mut parts = id.split('/');
        let first = parts.next().unwrap_or_default();
        let package = if first.starts_with('@') {
            let second = parts
                .next()
                .filter(|part| !part.is_empty())
                .ok_or_else(|| format!("Nome de pacote inválido: {id}"))?;
            format!("{first}/{second}")
        } else {
            first.to_owned()
        };
        let subpath = parts.collect::<Vec<_>>().join("/");
        let package_json = base
            .ancestors()
            .map(|ancestor| {
                ancestor
                    .join("node_modules")
                    .join(&package)
                    .join("package.json")
            })
            .find(|path| path.is_file())
            .ok_or_else(|| format!("Pacote não encontrado: {package}"))?
            .canonicalize()
            .map_err(|error| format!("Falha ao resolver {package}: {error}"))?;
        let manifest = self.read(&package_json)?;
        let manifest: serde_json::Value = serde_json::from_str(&manifest).map_err(|error| {
            format!(
                "package.json inválido em {}: {error}",
                package_json.display()
            )
        })?;
        let stylesheet = if subpath.is_empty() {
            manifest
                .pointer("/exports/./style")
                .or_else(|| manifest.get("style"))
                .or_else(|| manifest.get("main"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("index.css")
        } else {
            &subpath
        };
        package_json
            .parent()
            .unwrap()
            .join(stylesheet)
            .canonicalize()
            .map_err(|error| format!("Falha ao resolver stylesheet de {id}: {error}"))
    }

    pub fn load(&mut self, id: &str, base: &str) -> Result<LoadedStylesheet, String> {
        let path = self.resolve(id, Path::new(base))?;
        let content = self.read(&path)?;
        self.imports.push(Import {
            id: id.into(),
            base: base.into(),
            path: path.clone(),
        });
        let path_string = path
            .to_str()
            .ok_or_else(|| format!("O caminho do stylesheet não é UTF-8: {}", path.display()))?;
        let base = path
            .parent()
            .and_then(|path| path.to_str())
            .ok_or_else(|| format!("O diretório do stylesheet não é UTF-8: {}", path.display()))?;
        Ok(LoadedStylesheet {
            path: path_string.into(),
            base: base.into(),
            content,
        })
    }

    pub fn fingerprint(&self) -> String {
        let mut hash = Sha256::new();
        for (path, content) in &self.files {
            hash.update(serde_json::to_vec(&(path, content.len())).unwrap());
            hash.update(content);
        }
        let mut imports = self
            .imports
            .iter()
            .map(|edge| {
                serde_json::to_string(&(edge.id.as_str(), edge.base.as_str(), &edge.path)).unwrap()
            })
            .collect::<Vec<_>>();
        imports.sort();
        for edge in imports {
            hash.update(edge);
        }
        format!("{:x}", hash.finalize())
    }
}
