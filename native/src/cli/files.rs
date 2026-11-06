use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const GENERATED: [&str; 3] = [
    "src/lib/tailwind-merge.ts",
    "src/lib/tailwind-merge-data.ts",
    "src/types/icon.ts",
];

pub fn absolute(base: &Path, path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for part in base.join(path).components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            part => result.push(part.as_os_str()),
        }
    }
    result
}

fn source(path: &Path) -> bool {
    let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    [".ts", ".tsx", ".js", ".jsx"]
        .iter()
        .any(|suffix| filename.ends_with(suffix))
        && !filename.ends_with(".d.ts")
        && !filename.ends_with(".gen.ts")
}

fn scan(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("Falha ao listar {}: {error}", directory.display()))?
    {
        let entry =
            entry.map_err(|error| format!("Falha ao listar {}: {error}", directory.display()))?;
        let kind = entry
            .file_type()
            .map_err(|error| format!("Falha ao ler {}: {error}", entry.path().display()))?;
        if kind.is_dir() {
            scan(&entry.path(), files)?;
        } else if kind.is_file() && source(&entry.path()) {
            files.push(entry.path());
        }
    }
    Ok(())
}

pub fn sources(root: &Path, targets: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let source_root = root.join("src");
    let direct = !targets.is_empty()
        && targets.iter().all(|path| {
            path.starts_with(&source_root)
                && source(path)
                && fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file())
                && path.canonicalize().is_ok_and(|actual| actual == *path)
        });
    let mut files = if direct {
        targets.to_vec()
    } else {
        let mut files = Vec::new();
        scan(&source_root, &mut files)?;
        files
    };
    files.retain(|path| {
        !GENERATED
            .iter()
            .any(|generated| path == &root.join(generated))
            && (targets.is_empty() || targets.iter().any(|target| path.starts_with(target)))
    });
    files.sort();
    files.dedup();
    Ok(files)
}

pub fn has_classes(source: &str) -> bool {
    ["className", "class", "twMerge", "mergeClassName", "cn"]
        .iter()
        .any(|word| {
            source.match_indices(word).any(|(start, _)| {
                !source
                    .as_bytes()
                    .get(start + word.len())
                    .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            })
        })
}

pub fn unchanged(path: &Path, original: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Falha ao ler {}: {error}", path.display()))?;
    if metadata.is_symlink() || path.canonicalize().ok().as_deref() != Some(path) {
        return Err(format!(
            "Correção recusada em caminho com symlink ou não canônico: {}",
            path.display()
        ));
    }
    if !metadata.is_file()
        || fs::read(path).map_err(|error| error.to_string())? != original.as_bytes()
    {
        return Err(format!(
            "O arquivo mudou durante a execução: {}",
            path.display()
        ));
    }
    if metadata.permissions().readonly() {
        return Err(format!("Arquivo somente leitura: {}", path.display()));
    }
    // A rename only needs directory access; check the file's own ACL as well.
    OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|error| format!("Sem acesso de escrita a {}: {error}", path.display()))?;
    Ok(())
}

pub fn write(path: &Path, contents: &[u8]) -> Result<(), String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = path
        .parent()
        .ok_or_else(|| format!("Caminho inválido: {}", path.display()))?;
    let temporary = parent.join(format!(
        ".tailwind-canonical-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| format!("Falha ao criar {}: {error}", temporary.display()))?;
    let result = (|| {
        if let Ok(metadata) = fs::metadata(path) {
            file.set_permissions(metadata.permissions())?;
        }
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(format!("Falha ao escrever {}: {error}", path.display()));
    }
    Ok(())
}
