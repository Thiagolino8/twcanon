// Tailwind 4.3.4 to-key-path and keyPathToCssProperty.
use crate::syntax::segment::segment;
use regex::Regex;
use std::sync::LazyLock;

pub fn to_key_path(input: &str) -> Vec<String> {
    let mut output = Vec::new();
    for part in segment(input, b'.') {
        if !part.contains('[') {
            output.push(part.into());
            continue;
        }
        let mut index = 0;
        while let Some(left) = part[index..].find('[').map(|i| i + index) {
            let Some(right) = part[left..].find(']').map(|i| i + left) else {
                break;
            };
            if left > index {
                output.push(part[index..left].into());
            }
            output.push(part[left + 1..right].into());
            index = right + 1;
        }
        if index < part.len() {
            output.push(part[index..].into());
        }
    }
    output
}

fn inherited_value(key: &str) -> Option<String> {
    match key {
        "__proto__" => Some("[object Object]".into()),
        "constructor" => Some("function Object() { [native code] }".into()),
        "__defineGetter__"
        | "__defineSetter__"
        | "__lookupGetter__"
        | "__lookupSetter__"
        | "hasOwnProperty"
        | "isPrototypeOf"
        | "propertyIsEnumerable"
        | "toLocaleString"
        | "toString"
        | "valueOf" => Some(format!("function {key}() {{ [native code] }}")),
        _ => None,
    }
}

pub fn key_path_to_css_property(path: &[String]) -> Result<Option<String>, String> {
    let first = path.first().map(String::as_str).unwrap_or("");
    let special = match first {
        "borderWidth" => Some("border-width".into()),
        "outlineWidth" => Some("outline-width".into()),
        "ringColor" => Some("ring-color".into()),
        "ringWidth" => Some("ring-width".into()),
        "transitionDuration" => Some("transition-duration".into()),
        "transitionTimingFunction" => Some("transition-timing-function".into()),
        _ => inherited_value(first),
    };
    if path.get(1).is_some_and(|s| s == "DEFAULT")
        && let Some(special) = special
    {
        return Ok(Some(format!("default-{special}")));
    }
    if first == "container"
        || path.iter().any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_%/.".contains(&b))
        })
    {
        return Ok(None);
    }
    if inherited_value(first).is_some() {
        return Err("part.replaceAll is not a function".into());
    }
    let namespace = match first {
        "animation" => "animate",
        "aspectRatio" => "aspect",
        "borderRadius" => "radius",
        "boxShadow" => "shadow",
        "colors" => "color",
        "containers" | "maxWidth" => "container",
        "fontFamily" => "font",
        "fontSize" => "text",
        "letterSpacing" => "tracking",
        "lineHeight" => "leading",
        "screens" => "breakpoint",
        "transitionTimingFunction" => "ease",
        _ => first,
    };
    static KEBAB: LazyLock<Regex> = LazyLock::new(|| Regex::new("([a-z])([A-Z])").unwrap());
    let mut output = Vec::new();
    for (index, part) in path.iter().enumerate() {
        let part = if index == 0 { namespace } else { part };
        if part == "DEFAULT" && index + 1 == path.len() {
            continue;
        }
        let part = if part == "1" && index + 1 != path.len() {
            ""
        } else {
            part
        };
        let mut part = part.replace('.', "_");
        if index == 0 || part.starts_with('-') || part == "lineHeight" {
            part = KEBAB
                .replace_all(&part, |caps: &regex::Captures<'_>| {
                    format!("{}-{}", &caps[1], caps[2].to_ascii_lowercase())
                })
                .into_owned();
        }
        output.push(part);
    }
    Ok(Some(output.join("-")))
}
