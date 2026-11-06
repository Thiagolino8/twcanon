use crate::{
    extraction::{self, TextKind},
    runtime::Runtime,
};

pub struct Diagnostic {
    pub line: usize,
    pub original: String,
    pub updated: String,
}

pub struct Rewrites {
    pub text: String,
    pub diagnostics: Vec<Diagnostic>,
    pub stable: bool,
}

fn render(value: &str, kind: TextKind, quote: char) -> String {
    match kind {
        TextKind::String => serde_json::to_string(value).unwrap(),
        TextKind::Template => format!(
            "`{}`",
            value
                .replace('\\', "\\\\")
                .replace('`', "\\`")
                .replace("${", "\\${")
        ),
        TextKind::Attribute => format!(
            "{quote}{}{quote}",
            match quote {
                '\'' => value.replace('\'', "&#39;"),
                _ => value.replace('"', "&quot;"),
            }
        ),
    }
}

pub fn plan(filename: &str, source: &str, runtime: &mut Runtime) -> Result<Rewrites, String> {
    let classes = extraction::collect(filename, source)?;
    let mut replacements = Vec::new();
    let mut diagnostics = Vec::new();
    for class in classes {
        let Some(updated) = runtime
            .canonicalize(&class.value)?
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let start = class.start as usize;
        let end = class.end as usize;
        let quote = source[start..end].chars().next().unwrap();
        replacements.push((start, end, render(&updated, class.kind, quote)));
        diagnostics.push(Diagnostic {
            line: extraction::line_at(source, start + quote.len_utf8()),
            original: class.value,
            updated,
        });
    }
    let mut text = source.to_owned();
    replacements.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
    for (start, end, replacement) in replacements {
        text.replace_range(start..end, &replacement);
    }
    let mut stable = true;
    if !diagnostics.is_empty() {
        // Reparse the exact replacement syntax, including JSX entity handling.
        for class in extraction::collect(filename, &text)? {
            if runtime.canonicalize(&class.value)?.is_some() {
                stable = false;
            }
        }
    }
    Ok(Rewrites {
        text,
        diagnostics,
        stable,
    })
}
