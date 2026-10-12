//! User themes: the variable contract and the parser that turns an untrusted
//! `.css` file into a map of validated values (spec M10 §2). Pure, no I/O.

mod parse;
mod variables;

pub use parse::{parse_theme, ParsedTheme, ThemeError, ThemeMeta, ThemeWarning, WarningCode};
pub use variables::{variable, variables, VarKind, VarSpec};
