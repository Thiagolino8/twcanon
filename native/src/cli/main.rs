#![forbid(unsafe_code)]

mod extraction;
mod files;
mod rewrites;
mod runtime;
mod stylesheets;

use runtime::Runtime;
use std::{collections::BTreeSet, env, fs, path::PathBuf, process::ExitCode};

struct Options {
    check: bool,
    cache: bool,
    css: PathBuf,
    targets: Vec<PathBuf>,
}

fn options(root: &std::path::Path) -> Result<Option<Options>, String> {
    let mut options = Options {
        check: false,
        cache: env::var_os("CI").is_none_or(|value| value.is_empty()),
        css: root.join("src/index.css"),
        targets: Vec::new(),
    };
    let mut arguments = env::args_os().skip(1);
    let mut positional = false;
    while let Some(argument) = arguments.next() {
        if positional {
            options
                .targets
                .push(files::absolute(root, argument.as_ref()));
            continue;
        }
        match argument.to_str() {
            Some("--") => positional = true,
            Some("--check") => options.check = true,
            Some("--serial") => {}
            Some("--no-cache") => options.cache = false,
            Some("--css") => {
                let value = arguments.next().ok_or("Falta o caminho após --css.")?;
                options.css = files::absolute(root, value.as_ref());
            }
            Some("--help" | "-h") => {
                println!(
                    "Uso: twcanon [--check] [--css arquivo.css] [--no-cache] [caminhos...]\n\nSem --check, corrige os arquivos em src/.\n--serial é aceito para compatibilidade."
                );
                return Ok(None);
            }
            Some("--version" | "-v") => {
                println!("{}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            Some(flag) if flag.starts_with('-') => {
                return Err(format!("Argumento desconhecido: {flag}"));
            }
            _ => options
                .targets
                .push(files::absolute(root, argument.as_ref())),
        }
    }
    Ok(Some(options))
}

fn run() -> Result<u8, String> {
    let root = env::current_dir().map_err(|error| error.to_string())?;
    let Some(options) = options(&root)? else {
        return Ok(0);
    };
    let paths = files::sources(&root, &options.targets)?;
    let mut runtime = Runtime::new(options.css)?;
    let cache_path = root.join(".cache/twcanon.json");
    if options.cache
        && let Ok(content) = fs::read(&cache_path)
    {
        runtime.restore(&content);
    }
    let mut plans = Vec::new();
    for path in &paths {
        let source = fs::read_to_string(path)
            .map_err(|error| format!("Falha ao ler {}: {error}", path.display()))?;
        let hash = stylesheets::digest(source.as_bytes());
        if runtime.sources.get(path) == Some(&hash) {
            continue;
        }
        if !files::has_classes(&source) {
            runtime.sources.insert(path.clone(), hash);
            continue;
        }
        let filename = path
            .strip_prefix(&root)
            .unwrap()
            .as_os_str()
            .to_str()
            .ok_or_else(|| format!("Nome de arquivo não é UTF-8: {}", path.display()))?;
        let plan = rewrites::plan(filename, &source, &mut runtime)?;
        if plan.diagnostics.is_empty() {
            runtime.sources.insert(path.clone(), hash);
        } else {
            runtime.sources.remove(path);
            if !options.check && plan.stable {
                runtime
                    .sources
                    .insert(path.clone(), stylesheets::digest(plan.text.as_bytes()));
            }
            plans.push((path, source, plan));
        }
    }
    // Parsing and canonicalization must succeed for the entire batch before writing.
    if !options.check {
        for (path, source, _) in &plans {
            files::unchanged(&root, path, source)?;
        }
        for (index, (path, source, plan)) in plans.iter().enumerate() {
            if let Err(error) = files::unchanged(&root, path, source)
                .and_then(|()| files::write(path, plan.text.as_bytes()))
            {
                if index == 0 {
                    return Err(error);
                }
                let written = plans[..index]
                    .iter()
                    .map(|(path, _, _)| {
                        format!("  {}", path.strip_prefix(&root).unwrap().display())
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                return Err(format!(
                    "{error}\nArquivos já alterados neste lote:\n{written}"
                ));
            }
        }
    }
    if options.targets.is_empty() {
        let existing: BTreeSet<_> = paths.iter().collect();
        runtime.sources.retain(|path, _| existing.contains(path));
    }
    if options.cache
        && let Some(cache) = runtime.snapshot()
    {
        // The cache is optional. Source errors remain fatal above.
        if fs::create_dir_all(cache_path.parent().unwrap()).is_ok()
            && let Ok(content) = serde_json::to_vec(&cache)
        {
            let _ = files::write(&cache_path, &content);
        }
    }
    if plans.is_empty() {
        println!("Nenhuma classe Tailwind fora do padrão canônico.");
    } else {
        let mut count = 0;
        for (path, _, plan) in &plans {
            println!("{}", path.strip_prefix(&root).unwrap().display());
            for diagnostic in &plan.diagnostics {
                println!(
                    "  L{}: {} -> {}",
                    diagnostic.line, diagnostic.original, diagnostic.updated
                );
                count += 1;
            }
        }
        println!(
            "{} {count} ocorrência(s) em {} arquivo(s).",
            if options.check {
                "Encontradas"
            } else {
                "Atualizadas"
            },
            plans.len()
        );
    }
    Ok(u8::from(options.check && !plans.is_empty()))
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}
