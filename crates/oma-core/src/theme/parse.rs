//! Parser of user theme files (spec M10 §2): the trust boundary for
//! user-supplied CSS. A hand-written tokenizer (comments, strings, `{`, `}`,
//! `;`, `:`) reads one `:root { … }` block of known variables; everything
//! else becomes a warning and is never interpreted. The CSS text never
//! reaches the webview: only the normalized values do, through
//! `style.setProperty`, so our reading of the file is the only one that
//! counts (CSS escapes such as `\7b` are not decoded: they make a value
//! invalid instead).

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;

use super::variables::{variable, VarKind, VarSpec};

/// Largest accepted file, BOM included.
pub const MAX_THEME_BYTES: usize = 64 * 1024;
/// Longest name and author, in characters after the trim.
pub const MAX_META_CHARS: usize = 64;
/// Version of the contract this parser knows.
pub const THEME_VERSION: u32 = 1;
const MAX_FONTS: usize = 8;
const MAX_FONT_CHARS: usize = 64;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedTheme {
    pub meta: ThemeMeta,
    /// Accepted variables with their normalized values.
    pub vars: BTreeMap<String, String>,
    /// Sorted by line.
    pub warnings: Vec<ThemeWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeMeta {
    pub name: Option<String>,
    pub author: Option<String>,
    pub version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeWarning {
    /// 1-based; only `\n` counts.
    pub line: u32,
    pub code: WarningCode,
    /// The variable, property or selector involved (cleaned, at most 64
    /// characters): plain text for the UI.
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WarningCode {
    UnknownVariable,
    InvalidValue,
    Duplicate,
    Important,
    NotAVariable,
    Selector,
    AtRule,
    FutureVersion,
    LowContrast,
}

impl WarningCode {
    pub fn key(self) -> &'static str {
        match self {
            Self::UnknownVariable => "themes.warn.unknownVariable",
            Self::InvalidValue => "themes.warn.invalidValue",
            Self::Duplicate => "themes.warn.duplicate",
            Self::Important => "themes.warn.important",
            Self::NotAVariable => "themes.warn.notAVariable",
            Self::Selector => "themes.warn.selector",
            Self::AtRule => "themes.warn.atRule",
            Self::FutureVersion => "themes.warn.futureVersion",
            Self::LowContrast => "themes.warn.lowContrast",
        }
    }
}

/// Errors that reject the whole file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
pub enum ThemeError {
    #[error("theme file larger than 64 KiB")]
    TooLarge,
    #[error("theme file is not UTF-8")]
    NotUtf8,
    #[error("theme file has no valid variable")]
    Empty,
    #[error("unterminated comment, string or block in the theme file")]
    Unterminated,
}

impl ThemeError {
    pub fn key(self) -> &'static str {
        match self {
            Self::TooLarge => "themes.error.tooLarge",
            Self::NotUtf8 => "themes.error.notUtf8",
            Self::Empty => "themes.error.empty",
            Self::Unterminated => "themes.error.unterminated",
        }
    }
}

/// Parses and validates a theme file.
pub fn parse_theme(bytes: &[u8]) -> Result<ParsedTheme, ThemeError> {
    if bytes.len() > MAX_THEME_BYTES {
        return Err(ThemeError::TooLarge);
    }
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).map_err(|_| ThemeError::NotUtf8)?;
    let toks = tokenize(text)?;

    let mut p = Parser {
        toks: &toks,
        pos: 0,
        vars: BTreeMap::new(),
        lines: HashMap::new(),
        warnings: Vec::new(),
    };
    while p.kind() == Some(Tok::Ws) {
        p.pos += 1;
    }
    let mut meta = ThemeMeta {
        name: None,
        author: None,
        version: THEME_VERSION,
    };
    if p.kind() == Some(Tok::Comment) {
        let t = &toks[p.pos];
        meta = p.header(&t.text[2..t.text.len() - 2], t.line);
        p.pos += 1;
    }
    p.top_level()?;

    if p.vars.is_empty() {
        return Err(ThemeError::Empty);
    }
    p.warnings.sort_by_key(|w| w.line);
    Ok(ParsedTheme {
        meta,
        vars: p.vars,
        warnings: p.warnings,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    Ws,
    Comment,
    Str,
    Open,
    Close,
    Semi,
    Colon,
    Other,
}

struct Token<'a> {
    kind: Tok,
    text: &'a str,
    /// Line where the token starts.
    line: u32,
}

fn starts_comment(b: &[u8], i: usize) -> bool {
    b[i] == b'/' && b.get(i + 1) == Some(&b'*')
}

fn tokenize(s: &str) -> Result<Vec<Token<'_>>, ThemeError> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let (mut i, mut line) = (0, 1u32);
    while i < b.len() {
        let start = i;
        let kind = match b[i] {
            _ if starts_comment(b, i) => {
                let close = s[i + 2..].find("*/").ok_or(ThemeError::Unterminated)?;
                i += 2 + close + 2;
                Tok::Comment
            }
            q @ (b'"' | b'\'') => {
                i += 1;
                // Ends after an ASCII quote, so `i` stays on a char boundary
                // even when a backslash skips into a multi-byte character.
                loop {
                    match b.get(i) {
                        None | Some(b'\n') => return Err(ThemeError::Unterminated),
                        Some(b'\\') => i += 2,
                        Some(&c) if c == q => break i += 1,
                        Some(_) => i += 1,
                    }
                }
                Tok::Str
            }
            b'{' => {
                i += 1;
                Tok::Open
            }
            b'}' => {
                i += 1;
                Tok::Close
            }
            b';' => {
                i += 1;
                Tok::Semi
            }
            b':' => {
                i += 1;
                Tok::Colon
            }
            c if c.is_ascii_whitespace() => {
                while i < b.len() && b[i].is_ascii_whitespace() {
                    i += 1;
                }
                Tok::Ws
            }
            _ => {
                i += 1;
                while i < b.len()
                    && !matches!(b[i], b'{' | b'}' | b';' | b':' | b'"' | b'\'')
                    && !b[i].is_ascii_whitespace()
                    && !starts_comment(b, i)
                {
                    i += 1;
                }
                Tok::Other
            }
        };
        let text = &s[start..i];
        out.push(Token { kind, text, line });
        line += text.bytes().filter(|&c| c == b'\n').count() as u32;
    }
    Ok(out)
}

/// Token text with comments and whitespace as one space, trimmed.
fn join(toks: &[Token]) -> String {
    let mut out = String::new();
    for t in toks {
        match t.kind {
            Tok::Ws | Tok::Comment => out.push(' '),
            _ => out.push_str(t.text),
        }
    }
    out.trim().to_owned()
}

/// Line of the first meaningful token, or `fallback`.
fn first_line(toks: &[Token], fallback: u32) -> u32 {
    toks.iter()
        .find(|t| !matches!(t.kind, Tok::Ws | Tok::Comment))
        .map_or(fallback, |t| t.line)
}

struct Parser<'t, 'a> {
    toks: &'t [Token<'a>],
    pos: usize,
    vars: BTreeMap<String, String>,
    /// Line of the declaration that set each variable, for `Duplicate`.
    lines: HashMap<String, u32>,
    warnings: Vec<ThemeWarning>,
}

impl Parser<'_, '_> {
    fn kind(&self) -> Option<Tok> {
        self.toks.get(self.pos).map(|t| t.kind)
    }

    fn warn(&mut self, line: u32, code: WarningCode, detail: &str) {
        self.warnings.push(ThemeWarning {
            line,
            code,
            detail: clean_text(detail).unwrap_or_default(),
        });
    }

    /// Advances to the first token in `stops` (or the end); returns where it
    /// started.
    fn until(&mut self, stops: &[Tok]) -> usize {
        let start = self.pos;
        while self.kind().is_some_and(|k| !stops.contains(&k)) {
            self.pos += 1;
        }
        start
    }

    fn header(&mut self, body: &str, first: u32) -> ThemeMeta {
        let (mut name, mut author, mut version) = (None, None, None);
        for (n, raw) in body.split('\n').enumerate() {
            let l = raw.trim().trim_start_matches('*').trim_start();
            if let Some(v) = directive(l, "@oma-theme") {
                version.get_or_insert((v.trim(), first + n as u32));
            } else if let Some(v) = directive(l, "@name") {
                name.get_or_insert(v);
            } else if let Some(v) = directive(l, "@author") {
                author.get_or_insert(v);
            }
        }
        let mut meta = ThemeMeta {
            name: name.and_then(clean_text),
            author: author.and_then(clean_text),
            version: THEME_VERSION,
        };
        // Not a number: ignored, the contract version stays.
        let parsed = version.and_then(|(v, line)| Some((v.parse::<u32>().ok()?, v, line)));
        if let Some((n, v, line)) = parsed.filter(|(n, ..)| *n >= 1) {
            meta.version = n;
            if n > THEME_VERSION {
                self.warn(line, WarningCode::FutureVersion, v);
            }
        }
        meta
    }

    fn top_level(&mut self) -> Result<(), ThemeError> {
        let mut root_seen = false;
        while self.pos < self.toks.len() {
            let start = self.until(&[Tok::Open, Tok::Close, Tok::Semi]);
            let prelude = &self.toks[start..self.pos];
            let text = join(prelude);
            let Some(end) = self.toks.get(self.pos) else {
                if !text.is_empty() {
                    self.warn(first_line(prelude, 1), rule_code(&text), &text);
                }
                break;
            };
            let line = first_line(prelude, end.line);
            self.pos += 1;
            match end.kind {
                Tok::Open if !root_seen && text.eq_ignore_ascii_case(":root") => {
                    root_seen = true;
                    self.root_block()?;
                }
                Tok::Open => {
                    self.warn(line, rule_code(&text), &text);
                    self.skip_block()?;
                }
                Tok::Semi if text.is_empty() => {}
                Tok::Semi => self.warn(line, rule_code(&text), &text),
                // A stray `}`.
                _ => {
                    let detail = if text.is_empty() { "}" } else { &text };
                    self.warn(line, WarningCode::Selector, detail);
                }
            }
        }
        Ok(())
    }

    /// Skips to the `}` that closes a block whose `{` was just consumed.
    fn skip_block(&mut self) -> Result<(), ThemeError> {
        let mut depth = 1;
        while let Some(k) = self.kind() {
            self.pos += 1;
            match k {
                Tok::Open => depth += 1,
                Tok::Close if depth == 1 => return Ok(()),
                Tok::Close => depth -= 1,
                _ => {}
            }
        }
        Err(ThemeError::Unterminated)
    }

    /// Declarations of the `:root` block, whose `{` was just consumed.
    fn root_block(&mut self) -> Result<(), ThemeError> {
        loop {
            while matches!(self.kind(), Some(Tok::Ws | Tok::Comment | Tok::Semi)) {
                self.pos += 1;
            }
            let Some(first) = self.toks.get(self.pos) else {
                return Err(ThemeError::Unterminated);
            };
            let line = first.line;
            let start = self.until(&[Tok::Colon, Tok::Semi, Tok::Close, Tok::Open]);
            let name = join(&self.toks[start..self.pos]);
            match self.kind() {
                None => return Err(ThemeError::Unterminated),
                Some(Tok::Close) if name.is_empty() => {
                    self.pos += 1;
                    return Ok(());
                }
                Some(Tok::Colon) => {
                    self.pos += 1;
                    let vstart = self.until(&[Tok::Semi, Tok::Close, Tok::Open]);
                    match self.kind() {
                        None => return Err(ThemeError::Unterminated),
                        Some(Tok::Open) => {
                            // A nested rule such as `a:hover { … }`.
                            self.pos += 1;
                            self.warn(line, WarningCode::Selector, &name);
                            self.skip_block()?;
                        }
                        // `;` is skipped and `}` ends the block at the top of
                        // the loop.
                        _ => {
                            let value = join(&self.toks[vstart..self.pos]);
                            self.declare(name, &value, line);
                        }
                    }
                }
                Some(Tok::Open) => {
                    self.pos += 1;
                    let detail = if name.is_empty() { "{" } else { &name };
                    self.warn(line, WarningCode::Selector, detail);
                    self.skip_block()?;
                }
                // `stray;` or `stray }`.
                _ => self.warn(line, WarningCode::NotAVariable, &name),
            }
        }
    }

    fn declare(&mut self, name: String, value: &str, line: u32) {
        if !name.starts_with("--") {
            return self.warn(line, WarningCode::NotAVariable, &name);
        }
        let Some(spec) = variable(&name) else {
            return self.warn(line, WarningCode::UnknownVariable, &name);
        };
        let mut value = value;
        if let Some(rest) = strip_important(value) {
            self.warn(line, WarningCode::Important, &name);
            value = rest;
        }
        let Some(normalized) = normalize(spec, value) else {
            return self.warn(line, WarningCode::InvalidValue, &name);
        };
        if let Some(previous) = self.lines.insert(name.clone(), line) {
            self.warn(previous, WarningCode::Duplicate, &name);
        }
        self.vars.insert(name, normalized);
    }
}

fn rule_code(prelude: &str) -> WarningCode {
    if prelude.starts_with('@') {
        WarningCode::AtRule
    } else {
        WarningCode::Selector
    }
}

/// The rest of `line` after `word`, if the line starts with that directive.
fn directive<'a>(line: &'a str, word: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(word)?;
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then_some(rest)
}

/// Text for the UI: without control and bidi override characters, trimmed,
/// at most 64 characters; `None` when nothing is left.
fn clean_text(s: &str) -> Option<String> {
    let kept: String = s
        .chars()
        .filter(|&c| {
            !c.is_control() && !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .collect();
    let capped: String = kept.trim().chars().take(MAX_META_CHARS).collect();
    let capped = capped.trim_end();
    (!capped.is_empty()).then(|| capped.to_owned())
}

/// The value without a trailing `!important` (any case, spaces allowed after
/// the `!`), or `None` when there is none.
fn strip_important(value: &str) -> Option<&str> {
    let t = value.trim_end();
    let cut = t.len().checked_sub("important".len())?;
    if !t.get(cut..)?.eq_ignore_ascii_case("important") {
        return None;
    }
    Some(t[..cut].trim_end().strip_suffix('!')?.trim())
}

fn normalize(spec: &VarSpec, value: &str) -> Option<String> {
    let v = value.trim();
    match spec.kind {
        VarKind::Scheme => ["light", "dark"]
            .into_iter()
            .find(|s| v.eq_ignore_ascii_case(s))
            .map(str::to_owned),
        VarKind::Color => color(v),
        VarKind::Length => length(v, spec.min.unwrap_or(0.0), spec.max.unwrap_or(f64::MAX)),
        VarKind::FontList => font_list(v),
    }
}

/// `12`, `.5`, `0.25`: no sign, no exponent.
fn is_number(s: &str) -> bool {
    let (int, frac) = s.split_once('.').unwrap_or((s, "0"));
    !frac.is_empty()
        && int.bytes().all(|c| c.is_ascii_digit())
        && frac.bytes().all(|c| c.is_ascii_digit())
}

/// A number with the optional `suffix` (any case), lowercased.
fn component(s: &str, suffix: &str) -> Option<String> {
    let s = s.to_ascii_lowercase();
    is_number(s.strip_suffix(suffix).unwrap_or(&s)).then_some(s)
}

fn color(v: &str) -> Option<String> {
    if v.eq_ignore_ascii_case("transparent") {
        return Some("transparent".into());
    }
    if let Some(hex) = v.strip_prefix('#') {
        if !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let hex = hex.to_ascii_lowercase();
        return match hex.len() {
            3 | 4 => Some(
                std::iter::once('#')
                    .chain(hex.chars().flat_map(|c| [c, c]))
                    .collect(),
            ),
            6 | 8 => Some(format!("#{hex}")),
            _ => None,
        };
    }
    let open = v.find('(')?;
    let name = v[..open].to_ascii_lowercase();
    let hsl = match name.as_str() {
        "rgb" | "rgba" => false,
        "hsl" | "hsla" => true,
        _ => return None,
    };
    let inner = v[open + 1..].strip_suffix(')')?;
    if inner.contains(['(', ')']) {
        return None;
    }
    let comma = inner.contains(',');
    let (channels, alpha): (Vec<&str>, Option<&str>) = if comma {
        let p: Vec<&str> = inner.split(',').map(str::trim).collect();
        match p.len() {
            3 => (p, None),
            4 => (p[..3].to_vec(), Some(p[3])),
            _ => return None,
        }
    } else {
        let (channels, alpha) = match inner.split_once('/') {
            Some((c, a)) => (c, Some(a.trim())),
            None => (inner, None),
        };
        let p: Vec<&str> = channels.split_whitespace().collect();
        if p.len() != 3 {
            return None;
        }
        (p, alpha)
    };
    let mut out = Vec::with_capacity(3);
    for (i, c) in channels.into_iter().enumerate() {
        // Hue: a number or degrees; the other channels: a number or a percentage.
        out.push(component(c, if hsl && i == 0 { "deg" } else { "%" })?);
    }
    let alpha = match alpha {
        Some(a) => Some(component(a, "%")?),
        None => None,
    };
    let sep = if comma { ", " } else { " " };
    let mut s = format!("{name}({}", out.join(sep));
    if let Some(a) = alpha {
        s.push_str(if comma { ", " } else { " / " });
        s.push_str(&a);
    }
    s.push(')');
    Some(s)
}

fn length(v: &str, min: f64, max: f64) -> Option<String> {
    let cut = v.len().checked_sub(2)?;
    if !v.get(cut..)?.eq_ignore_ascii_case("px") || !is_number(&v[..cut]) {
        return None;
    }
    let n: f64 = v[..cut].parse().ok()?;
    (min..=max).contains(&n).then(|| format!("{n}px"))
}

/// Quoted names (letters, digits, spaces and `-_.+&`) or identifiers
/// separated by commas, normalized to `"Quoted", unquoted ident`.
fn font_list(v: &str) -> Option<String> {
    const KEYWORDS: [&str; 6] = [
        "inherit",
        "initial",
        "unset",
        "revert",
        "revert-layer",
        "default",
    ];
    let mut out = Vec::new();
    let mut rest = v;
    loop {
        if out.len() == MAX_FONTS {
            return None;
        }
        let item = if let Some(q) = rest.chars().next().filter(|c| matches!(c, '"' | '\'')) {
            let (name, after) = rest[1..].split_once(q)?;
            let allowed = name
                .chars()
                .all(|c| c.is_alphanumeric() || " -_.+&".contains(c));
            if !allowed || name.trim().is_empty() || name.chars().count() > MAX_FONT_CHARS {
                return None;
            }
            rest = after;
            format!("\"{name}\"")
        } else {
            let end = rest.find(',').unwrap_or(rest.len());
            let words: Vec<&str> = rest[..end].split_whitespace().collect();
            let allowed = words.iter().all(|w| {
                !w.starts_with(|c: char| c.is_ascii_digit())
                    && w.chars()
                        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            });
            let name = words.join(" ");
            if !allowed
                || name.is_empty()
                || name.chars().count() > MAX_FONT_CHARS
                || KEYWORDS.iter().any(|k| name.eq_ignore_ascii_case(k))
            {
                return None;
            }
            rest = &rest[end..];
            name
        };
        out.push(item);
        rest = rest.trim_start();
        if rest.is_empty() {
            return Some(out.join(", "));
        }
        rest = rest.strip_prefix(',')?.trim_start();
        if rest.is_empty() {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(css: &str) -> ParsedTheme {
        parse_theme(css.as_bytes()).expect("theme should parse")
    }

    fn codes(t: &ParsedTheme) -> Vec<(u32, WarningCode)> {
        t.warnings.iter().map(|w| (w.line, w.code)).collect()
    }

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// The value `parse_theme` keeps for `name: value`, or `None` when it is
    /// discarded.
    fn value_of(name: &str, value: &str) -> Option<String> {
        // `--crit` keeps the theme non-empty when the value is discarded.
        let t = ok(&format!(":root {{ --crit: #000; {name}: {value}; }}"));
        t.vars.get(name).cloned()
    }

    #[test]
    fn minimal_theme_keeps_its_three_variables() {
        let t = parse_theme(include_bytes!("../../../../testdata/themes/minimal.css")).unwrap();
        assert_eq!(t.vars.len(), 3);
        assert!(t.warnings.is_empty(), "{:?}", t.warnings);
        assert_eq!(t.meta.version, 1);
        assert_eq!(t.meta.name, None);
        assert_eq!(t.vars["--accent"], "#f2aa4c");
    }

    #[test]
    fn full_and_light_fixtures_are_clean() {
        let full =
            parse_theme(include_bytes!("../../../../testdata/themes/full-dark.css")).unwrap();
        assert!(full.warnings.is_empty(), "{:?}", full.warnings);
        let names: Vec<&str> = crate::theme::variables()
            .iter()
            .map(|v| v.name.as_str())
            .collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(
            full.vars.keys().map(String::as_str).collect::<Vec<_>>(),
            sorted
        );
        assert_eq!(full.vars["--radius"], "12px");
        assert_eq!(
            full.vars["--font"],
            "\"Segoe UI Variable\", \"Segoe UI\", system-ui, sans-serif"
        );
        assert_eq!(full.meta.author.as_deref(), Some("OpenMonitor Advanced"));

        let light = parse_theme(include_bytes!("../../../../testdata/themes/light.css")).unwrap();
        assert!(light.warnings.is_empty(), "{:?}", light.warnings);
        assert_eq!(light.vars.len(), 6);
        assert_eq!(light.vars["--color-scheme"], "light");
    }

    #[test]
    fn header_gives_name_author_and_version() {
        let t =
            ok("/* @oma-theme 1\n @name  Nord Notte \n @author Mario */\n:root { --bg: #2e3440; }");
        assert_eq!(t.meta.name.as_deref(), Some("Nord Notte"));
        assert_eq!(t.meta.author.as_deref(), Some("Mario"));
        assert_eq!(t.meta.version, 1);
        // Only the first comment before any token is the header.
        let t = ok(":root { --bg: #000; }\n/* @name Late */");
        assert_eq!(t.meta.name, None);
        // Doc-comment style stars are fine, and `@namex` is not `@name`.
        let t = ok("/*\n * @namex X\n * @name Star\n */ :root { --bg: #000; }");
        assert_eq!(t.meta.name.as_deref(), Some("Star"));
    }

    #[test]
    fn hostile_theme_keeps_only_valid_values() {
        let t = parse_theme(include_bytes!("../../../../testdata/themes/hostile.css")).unwrap();
        assert_eq!(t.vars, map(&[("--ok", "#ffffff"), ("--bg", "#222222")]));
        use WarningCode::*;
        assert_eq!(
            codes(&t),
            vec![
                (3, Selector),
                (4, AtRule),
                (5, AtRule),
                (7, InvalidValue),
                (8, InvalidValue),
                (9, InvalidValue),
                (10, InvalidValue),
                (11, Important),
                (12, UnknownVariable),
                (13, NotAVariable),
                (14, Duplicate),
            ]
        );
        assert_eq!(t.warnings[10].detail, "--bg");
        assert_eq!(t.warnings[1].detail, "@import url(x)");
    }

    #[test]
    fn rejects_values_that_escape_the_declaration() {
        let t = ok(":root{--bg:#fff; } body { display:none }");
        assert_eq!(t.vars, map(&[("--bg", "#ffffff")]));
        assert_eq!(codes(&t), vec![(1, WarningCode::Selector)]);
    }

    #[test]
    fn nested_blocks_and_rules_outside_root_are_skipped() {
        let css = "@media print { :root { --bg: #000; } }\n\
                   :root {\n\
                   --bg: #010101;\n\
                   a:hover { --ok: #fff; }\n\
                   { --warn: #fff; }\n\
                   stray;\n\
                   }\n\
                   :root { --accent: #fff; }\n\
                   --text: #fff;\n\
                   }\n";
        let t = ok(css);
        assert_eq!(t.vars, map(&[("--bg", "#010101")]));
        use WarningCode::*;
        assert_eq!(
            codes(&t),
            vec![
                (1, AtRule),
                (4, Selector),
                (5, Selector),
                (6, NotAVariable),
                (8, Selector),
                (9, Selector),
                (10, Selector),
            ]
        );
    }

    #[test]
    fn strings_and_comments_hide_braces() {
        let t = parse_theme(include_bytes!("../../../../testdata/themes/comments.css")).unwrap();
        assert_eq!(t.vars["--font"], "\"Fira Sans\", sans-serif");
        assert_eq!(t.vars["--accent"], "#abcdef");
        assert_eq!(t.meta.name.as_deref(), Some("Comments } ;"));
        // The first --font has a name with `;` and `}`: discarded, not a duplicate.
        assert_eq!(codes(&t), vec![(5, WarningCode::InvalidValue)]);
    }

    #[test]
    fn crlf_and_bom_keep_line_numbers() {
        let t = parse_theme(include_bytes!("../../../../testdata/themes/crlf-bom.css")).unwrap();
        assert_eq!(codes(&t), vec![(4, WarningCode::UnknownVariable)]);
        assert_eq!(t.warnings[0].detail, "--nope");
        assert_eq!(t.meta.name.as_deref(), Some("CRLF"));
        assert_eq!(t.vars, map(&[("--bg", "#101010")]));
    }

    #[test]
    fn colors_are_normalized() {
        let c = |v: &str| value_of("--bg", v);
        assert_eq!(c("#ABC").as_deref(), Some("#aabbcc"));
        assert_eq!(c("#abcd").as_deref(), Some("#aabbccdd"));
        assert_eq!(c("#AABBCC80").as_deref(), Some("#aabbcc80"));
        assert_eq!(c("rgb(1 2 3 / 50%)").as_deref(), Some("rgb(1 2 3 / 50%)"));
        assert_eq!(
            c("RGB(  1   2 3/50% )").as_deref(),
            Some("rgb(1 2 3 / 50%)")
        );
        assert_eq!(c("rgba(1,2,3,.5)").as_deref(), Some("rgba(1, 2, 3, .5)"));
        assert_eq!(
            c("hsl(300, 100%, 50%)").as_deref(),
            Some("hsl(300, 100%, 50%)")
        );
        assert_eq!(
            c("hsl(300DEG 100% 50%)").as_deref(),
            Some("hsl(300deg 100% 50%)")
        );
        assert_eq!(c("transparent").as_deref(), Some("transparent"));
        assert_eq!(c("TRANSPARENT").as_deref(), Some("transparent"));
        for bad in [
            "red",
            "#abcde",
            "#ggg",
            "#",
            "var(--accent)",
            "calc(1px)",
            "color-mix(in srgb, red, blue)",
            "rgb(1 2)",
            "rgb(1, 2, 3, 4, 5)",
            "rgb(1, 2 3)",
            "rgb(1 2 3 / 4 / 5)",
            "rgb(1, 2, 3 / 4)",
            "rgb(-1 2 3)",
            "rgb(1. 2 3)",
            "rgb(1deg 2 3)",
            "hsl(300% 1% 1%)",
            "rgb((1 2 3))",
            "rgb(1 2 3) red",
            "rgb (1 2 3)",
            "url(x)",
            "\\23 fff",
            "",
        ] {
            assert_eq!(c(bad), None, "{bad:?} should be rejected");
        }
    }

    #[test]
    fn scheme_accepts_light_and_dark_only() {
        assert_eq!(
            value_of("--color-scheme", "LIGHT").as_deref(),
            Some("light")
        );
        assert_eq!(value_of("--color-scheme", "dark").as_deref(), Some("dark"));
        assert_eq!(value_of("--color-scheme", "light dark"), None);
        assert_eq!(value_of("--color-scheme", "inherit"), None);
    }

    #[test]
    fn length_and_font_limits() {
        let r = |v: &str| value_of("--radius", v);
        assert_eq!(r("24px").as_deref(), Some("24px"));
        assert_eq!(r("0px").as_deref(), Some("0px"));
        assert_eq!(r("2.50PX").as_deref(), Some("2.5px"));
        for bad in [
            "25px",
            "4em",
            "-1px",
            "px",
            "12",
            "1e1px",
            "12 px",
            "calc(1px)",
        ] {
            assert_eq!(r(bad), None, "{bad:?} should be rejected");
        }

        let f = |v: &str| value_of("--font", v);
        assert_eq!(
            f("'Segoe UI',   Segoe   UI , monospace").as_deref(),
            Some("\"Segoe UI\", Segoe UI, monospace")
        );
        let eight = ["a"; 8].join(", ");
        assert_eq!(f(&eight).as_deref(), Some(eight.as_str()));
        assert_eq!(f(&["a"; 9].join(", ")), None);
        let long = "x".repeat(64);
        assert_eq!(f(&format!("\"{long}\"")), Some(format!("\"{long}\"")));
        assert_eq!(f(&format!("\"{long}x\"")), None);
        for bad in [
            "url(x.woff)",
            "\"url(x)\"",
            "a,",
            ",a",
            "a,,b",
            "\"\"",
            "\"a\" b",
            "\"a\\\"b\"",
            "inherit",
            "1font",
            "a\\62",
            "\"a<b\"",
        ] {
            assert_eq!(f(bad), None, "{bad:?} should be rejected");
        }
    }

    #[test]
    fn important_is_stripped_with_a_warning() {
        let t = ok(":root { --bg: #000 ! IMPORTANT; --ok: red !important; }");
        assert_eq!(t.vars, map(&[("--bg", "#000000")]));
        use WarningCode::*;
        assert_eq!(
            codes(&t),
            vec![(1, Important), (1, Important), (1, InvalidValue)]
        );
    }

    #[test]
    fn future_version_is_a_warning() {
        let t = ok("/* @oma-theme 2\n */\n:root { --bg: #000; }");
        assert_eq!(t.meta.version, 2);
        assert_eq!(codes(&t), vec![(1, WarningCode::FutureVersion)]);
        assert_eq!(t.vars, map(&[("--bg", "#000000")]));
        // Not a number: ignored, version 1.
        let t = ok("/* @oma-theme x */ :root { --bg: #000; }");
        assert_eq!(t.meta.version, 1);
        assert!(t.warnings.is_empty());
    }

    #[test]
    fn hard_errors() {
        assert_eq!(
            parse_theme(&vec![b' '; 65_537]).unwrap_err(),
            ThemeError::TooLarge
        );
        let mut at_limit = b":root { --bg: #000; }".to_vec();
        at_limit.resize(65_536, b' ');
        assert!(parse_theme(&at_limit).is_ok());
        assert_eq!(parse_theme(&[0xff, 0xfe]).unwrap_err(), ThemeError::NotUtf8);
        assert_eq!(
            parse_theme(include_bytes!("../../../../testdata/themes/empty.css")).unwrap_err(),
            ThemeError::Empty
        );
        assert_eq!(
            parse_theme(include_bytes!(
                "../../../../testdata/themes/unterminated.css"
            ))
            .unwrap_err(),
            ThemeError::Unterminated
        );
        assert_eq!(parse_theme(b"").unwrap_err(), ThemeError::Empty);
        assert_eq!(parse_theme(b":root { --bg: #fff; }  ").map(|_| ()), Ok(()));
        for unterminated in [
            ":root { --bg: #000;",
            ":root { --bg: #000",
            ":root { --font: \"abc; }",
            ":root { --font: \"a\nb\"; }",
            "body { :root { --bg: #000; }",
            "@media x { a { } ",
        ] {
            assert_eq!(
                parse_theme(unterminated.as_bytes()).unwrap_err(),
                ThemeError::Unterminated,
                "{unterminated:?}"
            );
        }
        // Only invalid values: nothing to apply.
        assert_eq!(
            parse_theme(b":root { --bg: red; }").unwrap_err(),
            ThemeError::Empty
        );
    }

    #[test]
    fn name_is_trimmed_capped_and_cleaned() {
        let long = "n".repeat(70);
        let t = ok(&format!(
            "/* @name {long}\n @author  A\u{7}B\u{202e}C */ :root {{ --bg: #000; }}"
        ));
        assert_eq!(t.meta.name.as_deref(), Some("n".repeat(64).as_str()));
        assert_eq!(t.meta.author.as_deref(), Some("ABC"));
        let t = ok("/* @name   \u{7} \n @author */ :root { --bg: #000; }");
        assert_eq!(t.meta.name, None);
        assert_eq!(t.meta.author, None);
        // 64 characters, not bytes.
        let t = ok(&format!(
            "/* @name {} */ :root {{ --bg: #000; }}",
            "è".repeat(70)
        ));
        assert_eq!(t.meta.name.map(|n| n.chars().count()), Some(64));
    }

    #[test]
    fn never_panics_on_noise() {
        // Deterministic xorshift over the characters the parser cares about,
        // plus a multi-byte one to probe the slicing.
        let alphabet: Vec<char> = ":root{};\"'\\/*-#a(),!@ \n\u{e8}px%".chars().collect();
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..20_000 {
            let mut s = String::new();
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            for _ in 0..(seed % 48) {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                s.push(alphabet[(seed % alphabet.len() as u64) as usize]);
            }
            let _ = parse_theme(s.as_bytes());
            let _ = parse_theme(format!(":root {{ --font: {s}; --bg: {s}; }}").as_bytes());
        }
    }

    #[test]
    fn keys_and_serialization_are_camel_case() {
        assert_eq!(
            WarningCode::UnknownVariable.key(),
            "themes.warn.unknownVariable"
        );
        assert_eq!(WarningCode::LowContrast.key(), "themes.warn.lowContrast");
        assert_eq!(ThemeError::TooLarge.key(), "themes.error.tooLarge");
        assert_eq!(ThemeError::NotUtf8.key(), "themes.error.notUtf8");
        let t = ok(":root { --nope: #000; --bg: #000; }");
        let json = serde_json::to_value(&t).unwrap();
        assert_eq!(json["warnings"][0]["code"], "unknownVariable");
        assert_eq!(json["meta"]["version"], 1);
        assert_eq!(
            serde_json::to_value(ThemeError::Unterminated).unwrap(),
            "unterminated"
        );
    }
}
