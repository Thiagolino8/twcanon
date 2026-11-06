#![forbid(unsafe_code)]

pub mod apply;
pub mod ast;
mod ast_arena;
pub mod breakpoints;
pub mod candidate;
pub mod canonicalize;
pub mod compare;
pub mod compile;
pub mod completions;
pub mod configuration;
pub mod css_functions;
pub mod custom_utilities;
pub mod data_type;
pub mod declaration;
mod id_hasher;
pub mod imports;
mod js_regex;
pub mod js_sort;
mod legacy_utilities;
pub mod nesting;
pub mod numeric;
pub mod optimize;
pub mod print;
pub mod signature;
pub mod syntax;
pub mod theme;
pub mod utilities;
mod variant_cache;
pub mod variants;

pub mod compat;
mod compat_theme;
