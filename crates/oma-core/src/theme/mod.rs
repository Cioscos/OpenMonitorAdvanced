//! User themes: the variable contract and the parser that turns an untrusted
//! `.css` file into a map of validated values (spec M10 §2). Pure, no I/O.

mod builtin;
mod contrast;
mod parse;
mod resolve;
mod variables;

pub use builtin::{builtin, builtins, Builtin, DEFAULT_THEME};
pub use contrast::{contrast_ratio, contrast_warnings, parse_color};
pub use resolve::{resolve, scheme_of, to_css, Scheme};

pub use parse::{parse_theme, ParsedTheme, ThemeError, ThemeMeta, ThemeWarning, WarningCode};
pub use variables::{variable, variables, VarKind, VarSpec};
