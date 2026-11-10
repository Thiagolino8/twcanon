// CSS-only backward-compatible theme resolution from Tailwind CSS 4.3.4.
// Structural definitions are generated from the pinned default-theme source.
use crate::{
    compat,
    syntax::{escape::unescape, is_js_whitespace},
    theme::{DEFAULT, Theme},
    utilities::with_alpha,
};
use indexmap::IndexMap;
use serde::Deserialize;
use std::sync::LazyLock;

#[derive(Deserialize)]
struct Definitions {
    definitions: IndexMap<String, Expression>,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum Expression {
    String { value: String },
    Array { values: Vec<Expression> },
    Object { entries: Vec<Property> },
    Theme { path: String },
    Bare,
}
#[derive(Deserialize)]
struct Property {
    key: Option<String>,
    value: Expression,
}
static DEFINITIONS: LazyLock<Definitions> = LazyLock::new(|| {
    let bytes = miniz_oxide::inflate::decompress_to_vec(include_bytes!(concat!(
        env!("OUT_DIR"),
        "/compat-theme.json.deflate"
    )))
    .expect("falha ao descomprimir as definições do tema de compatibilidade");
    serde_json::from_slice(&bytes).expect("pinned compatibility theme definitions")
});

#[derive(Clone)]
enum Value {
    Null,
    String(String),
    Number(u8),
    Object(Properties),
    Array(Vec<Value>),
    Function,
}

type Properties = IndexMap<String, Value>;

impl Value {
    fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Object(map) => map.get(key),
            Self::Array(values) => key.parse::<usize>().ok().and_then(|i| values.get(i)),
            _ => None,
        }
    }
    fn options(&self, key: &str) -> u8 {
        match self.get(key) {
            Some(Self::Number(value)) => *value,
            _ => 0,
        }
    }
    fn js_string(&self) -> String {
        match self {
            Self::Null => "".into(),
            Self::String(value) => value.clone(),
            Self::Number(value) => value.to_string(),
            Self::Object(_) => "[object Object]".into(),
            Self::Array(values) => values
                .iter()
                .map(Self::js_string)
                .collect::<Vec<_>>()
                .join(","),
            Self::Function => "function".into(),
        }
    }
}

fn set(map: &mut IndexMap<String, Value>, key: &str, value: Value) {
    // Assigning a primitive or array to Object.prototype.__proto__ does not
    // create an own property. No other prototype keys are special here.
    if key != "__proto__" {
        map.insert(key.into(), value);
    }
}

fn camel_case(value: &str) -> String {
    let mut chars = value.chars().peekable();
    let mut output = String::new();
    while let Some(c) = chars.next() {
        if c == '-' && chars.peek().is_some_and(char::is_ascii_lowercase) {
            output.push(chars.next().unwrap().to_ascii_uppercase());
        } else {
            output.push(c);
        }
    }
    output
}

fn read_css(theme: &Theme, path: &[String]) -> Result<(Value, Value), String> {
    if let [key] = path
        && key.starts_with("--")
    {
        return Ok((
            theme
                .get(std::slice::from_ref(key))
                .map_or(Value::Null, |v| Value::String(v.into())),
            Value::Number(theme.get_options(key)),
        ));
    }
    let key = compat::key_path_to_css_property(path)?.unwrap_or_else(|| "null".into());
    let namespace = format!("--{key}");
    let entries = theme.namespace(&namespace);
    if entries.is_empty() {
        return Ok((Value::Null, Value::Number(0)));
    }
    let (mut values, mut options) = (IndexMap::new(), IndexMap::new());
    let mut nested: IndexMap<String, (Properties, Properties)> = IndexMap::new();
    for (key, value) in entries {
        let key = key.as_deref().unwrap_or("");
        if !key.is_empty()
            && let Some((main, child)) = key.split_once("--")
        {
            let (values, options) = nested.entry(main.into()).or_default();
            let child = camel_case(child);
            set(values, &child, Value::String(value));
            set(
                options,
                &child,
                Value::Number(theme.get_options(&format!("{namespace}{key}"))),
            );
        } else {
            values.insert(key.to_owned(), Value::String(value));
            options.insert(
                key.to_owned(),
                Value::Number(theme.get_options(&if key.is_empty() {
                    namespace.clone()
                } else {
                    format!("{namespace}-{key}")
                })),
            );
        }
    }
    for (key, (extra, extra_options)) in nested {
        if let Some(Value::String(value)) = values.get(&key) {
            values.insert(
                key.clone(),
                Value::Array(vec![Value::String(value.clone()), Value::Object(extra)]),
            );
            options.insert(
                key,
                Value::Array(vec![
                    Value::Number(theme.get_options(&namespace)),
                    Value::Object(extra_options),
                ]),
            );
        }
    }
    let (mut object, mut option_object) = (IndexMap::new(), IndexMap::new());
    for (key, value) in values {
        set(
            &mut object,
            if key.is_empty() { "DEFAULT" } else { &key },
            value,
        );
    }
    for (key, value) in options {
        set(
            &mut option_object,
            if key.is_empty() { "DEFAULT" } else { &key },
            value,
        );
    }
    if path.last().is_some_and(|s| s == "DEFAULT")
        || (path.len() > 1 && object.contains_key("DEFAULT") && object.len() == 1)
    {
        return Ok((
            object.shift_remove("DEFAULT").unwrap_or(Value::Null),
            option_object
                .shift_remove("DEFAULT")
                .unwrap_or(Value::Number(0)),
        ));
    }
    object.insert(
        "__CSS_VALUES__".into(),
        Value::Object(option_object.clone()),
    );
    Ok((Value::Object(object), Value::Object(option_object)))
}

struct Resolver<'a> {
    theme: &'a Theme,
    cache: IndexMap<String, Value>,
}

fn deep_merge(left: Value, right: Value) -> Value {
    match (left, right) {
        (Value::Object(mut left), Value::Object(right)) => {
            for (key, value) in right {
                let old = left.shift_remove(&key).unwrap_or(Value::Null);
                set(&mut left, &key, deep_merge(old, value));
            }
            Value::Object(left)
        }
        (left, Value::Null) => left,
        (_, right) => right,
    }
}

impl Resolver<'_> {
    fn evaluate(&mut self, expr: &Expression) -> Result<Value, String> {
        Ok(match expr {
            Expression::String { value } => Value::String(value.clone()),
            Expression::Array { values } => Value::Array(
                values
                    .iter()
                    .map(|v| self.evaluate(v))
                    .collect::<Result<_, _>>()?,
            ),
            Expression::Theme { path } => self.lookup(path)?,
            Expression::Bare => {
                Value::Object(IndexMap::from([("__BARE_VALUE__".into(), Value::Function)]))
            }
            Expression::Object { entries } => {
                let mut object = IndexMap::new();
                for property in entries {
                    let value = self.evaluate(&property.value)?;
                    if let Some(key) = &property.key {
                        set(&mut object, key, value);
                    } else if let Value::Object(values) = value {
                        for (key, value) in values {
                            set(&mut object, &key, value);
                        }
                    } else if let Value::String(value) = value {
                        for (index, c) in value.chars().enumerate() {
                            set(
                                &mut object,
                                &index.to_string(),
                                Value::String(c.to_string()),
                            );
                        }
                    }
                }
                Value::Object(object)
            }
        })
    }

    fn root(&mut self, key: &str) -> Result<Value, String> {
        if let Some(value) = self.cache.get(key) {
            return Ok(value.clone());
        }
        let Some(expr) = DEFINITIONS.definitions.get(key) else {
            return Ok(Value::Null);
        };
        let mut value = self.evaluate(expr)?;
        let extension = match key {
            "fontSize" => Some("text"),
            "boxShadow" => Some("shadow"),
            "animation" => Some("animate"),
            "aspectRatio" => Some("aspect"),
            "borderRadius" => Some("radius"),
            "screens" => Some("breakpoint"),
            "letterSpacing" => Some("tracking"),
            "lineHeight" => Some("leading"),
            "maxWidth" => Some("container"),
            _ => None,
        };
        if let Some(path) = extension {
            value = deep_merge(value, self.lookup(path)?);
        } else if matches!(key, "transitionDuration" | "transitionTimingFunction") {
            let variable = if key == "transitionDuration" {
                "--default-transition-duration"
            } else {
                "--default-transition-timing-function"
            };
            if let Value::Object(values) = &mut value {
                values.insert(
                    "DEFAULT".into(),
                    self.theme
                        .get(&[variable.into()])
                        .map_or(Value::Null, |value| Value::String(value.into())),
                );
            }
        }
        self.cache.insert(key.into(), value.clone());
        Ok(value)
    }

    fn config(&mut self, path: &[String]) -> Result<Value, String> {
        let mut value = Value::Object(IndexMap::new());
        let mut key = String::new();
        let mut at_root = true;
        for (i, part) in path.iter().enumerate() {
            if !key.is_empty() {
                key.push('-');
            }
            key.push_str(part);
            let next = if at_root {
                let value = self.root(&key)?;
                (!matches!(value, Value::Null)).then_some(value)
            } else {
                value.get(&key).cloned()
            };
            if let Some(next) = next {
                value = next;
                key.clear();
                at_root = false;
            } else if i + 1 == path.len() {
                return Ok(Value::Null);
            }
        }
        Ok(value)
    }

    fn lookup(&mut self, path: &str) -> Result<Value, String> {
        let (path, modifier) = path
            .rsplit_once('/')
            .map_or((path, None), |(path, modifier)| {
                (
                    path.trim_matches(is_js_whitespace),
                    Some(modifier.trim_matches(is_js_whitespace)),
                )
            });
        let keys = compat::to_key_path(path);
        let (mut css, options) = read_css(self.theme, &keys)?;
        let mut config = self.config(&keys)?;
        if let Value::String(value) = &mut config {
            *value = value.replacen("<alpha-value>", "1", 1);
        }
        let mut result = if let Value::String(_) = &css {
            if matches!(options,Value::Number(value) if value & DEFAULT != 0)
                && !matches!(config, Value::Null)
            {
                config
            } else {
                css
            }
        } else if let Value::Object(mut config) = config {
            if matches!(css, Value::Null)
                && let Some(Value::Object(meta)) = config.get("__CSS_VALUES__").cloned()
            {
                let mut local = IndexMap::new();
                for key in meta.keys() {
                    if let Some(value) = config.shift_remove(key) {
                        local.insert(key.clone(), value);
                    }
                }
                css = Value::Object(local);
            }
            if let Value::Object(css) = css {
                for (key, value) in css {
                    if key == "__CSS_VALUES__" {
                        continue;
                    }
                    if config
                        .get("__CSS_VALUES__")
                        .is_some_and(|meta| meta.options(&key) & DEFAULT != 0)
                        && get(
                            &Value::Object(config.clone()),
                            &key.split('-').map(str::to_owned).collect::<Vec<_>>(),
                        )
                        .is_some()
                    {
                        continue;
                    }
                    set(&mut config, &unescape(&key), value);
                }
            }
            Value::Object(config)
        } else if let (
            Value::Array(css_values),
            Value::Array(option_values),
            Value::Array(config_values),
        ) = (&css, &options, &config)
        {
            let mut values = css_values.clone();
            if matches!(option_values.first(),Some(Value::Number(value)) if value & DEFAULT != 0)
                && let Some(value) = config_values.first().filter(|v| !matches!(v, Value::Null))
            {
                values[0] = value.clone();
            }
            if let Some(Value::Object(extra)) = values.get_mut(1) {
                for (key, value) in extra.iter_mut() {
                    if option_values
                        .get(1)
                        .is_some_and(|v| v.options(key) & DEFAULT != 0)
                        && let Some(config) = config_values
                            .get(1)
                            .and_then(|v| v.get(key))
                            .filter(|v| !matches!(v, Value::Null))
                    {
                        *value = config.clone();
                    }
                }
            }
            Value::Array(values)
        } else if keys.len() > 1 && css.get("DEFAULT").is_some() {
            css.get("DEFAULT").unwrap().clone()
        } else if matches!(css, Value::Null) {
            config
        } else {
            css
        };
        if let (Value::String(value), Some(modifier)) =
            (&mut result, modifier.filter(|m| !m.is_empty()))
        {
            *value = with_alpha(value, modifier);
        }
        Ok(result)
    }
}

fn get(value: &Value, path: &[String]) -> Option<Value> {
    let mut value = value;
    let mut key = String::new();
    for (i, part) in path.iter().enumerate() {
        if !key.is_empty() {
            key.push('-');
        }
        key.push_str(part);
        if let Some(next) = value.get(&key) {
            value = next;
            key.clear();
        } else if i + 1 == path.len() {
            return None;
        }
    }
    Some(value.clone())
}

pub(crate) fn resolve(theme: &Theme, path: &str, inline: bool) -> Result<Option<String>, String> {
    if path.starts_with("--") {
        return Ok(crate::css_functions::resolve_theme_value(
            theme, path, inline,
        ));
    }
    let mut resolver = Resolver {
        theme,
        cache: IndexMap::new(),
    };
    let mut value = resolver.lookup(path)?;
    if let Some(default) = value.get("DEFAULT") {
        value = default.clone();
    }
    Ok(match value {
        Value::String(value) => Some(value),
        Value::Array(values) if values.len() == 2 => match &values[0] {
            Value::String(value) => Some(value.clone()),
            _ => None,
        },
        Value::Array(values) => Some(
            values
                .iter()
                .map(Value::js_string)
                .collect::<Vec<_>>()
                .join(", "),
        ),
        _ => None,
    })
}
