//! Leaderboard rules shared with the Worker (scores-worker/src/rules.ts and table.ts): model
//! normalisation, submission validation, the reference table and the plausibility check. The
//! JavaScript behaviours (whitespace, UTF-16 lengths, `Number.isInteger`) are imitated by hand
//! and pinned by the fixtures in testdata/scores/. Pure; no Windows code.

use super::{DISK_SCORE_VERSION, GPU_SCORE_VERSION, SCORE_VERSION};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;

// OMA: the Worker's host, as in scores-worker/wrangler.toml routes
pub const TABLE_URL: &str = "https://scores.cischi.dev/v1/reference-scores.json";
pub const SUBMIT_URL: &str = "https://scores.cischi.dev/v1/submit";

pub const MAX_SUBMIT_BYTES: usize = 16384;
pub const VALUE_CAP: f64 = 100000.0;
pub const PLAUSIBLE_MIN: f64 = 0.2;
pub const PLAUSIBLE_MAX: f64 = 5.0;
pub const MODEL_MAX: usize = 128;
pub const MAX_TABLE_BYTES: usize = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Board {
    CpuSingle,
    CpuMulti,
    GpuCompute,
    GpuGraphics,
    Disk,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Author,
    Community,
}

/// The server's error codes the app also checks for itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadJson,
    BadSchema,
    BadFormat,
    UnknownVersion,
    NotValid,
    BadValue,
    Implausible,
    BodyTooLarge,
    RateLimited,
    DailyCap,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BadJson => "bad_json",
            Self::BadSchema => "bad_schema",
            Self::BadFormat => "bad_format",
            Self::UnknownVersion => "unknown_version",
            Self::NotValid => "not_valid",
            Self::BadValue => "bad_value",
            Self::Implausible => "implausible",
            Self::BodyTooLarge => "body_too_large",
            Self::RateLimited => "rate_limited",
            Self::DailyCap => "daily_cap",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Model {
    pub display: String,
    pub key: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Submission {
    pub category: String,
    pub score_version: String,
    pub overclock: bool,
    pub app_version: String,
    pub os_build: String,
    pub ram_gb: u32,
    pub flags: Vec<String>,
    pub model: Model,
    pub values: Vec<(Board, f64)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableRow {
    pub category: Board,
    pub score_version: String,
    pub model: String,
    pub value: f64,
    pub n: u32,
    pub source: Source,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub generated_at: Option<String>,
    pub rows: Vec<TableRow>,
}

/// The ranges of Unicode `Cf` (format characters), as in testdata/scores/format-chars.json.
const CF_RANGES: &[(u32, u32)] = &[
    (173, 173),
    (1536, 1541),
    (1564, 1564),
    (1757, 1757),
    (1807, 1807),
    (2192, 2193),
    (2274, 2274),
    (6158, 6158),
    (8203, 8207),
    (8234, 8238),
    (8288, 8292),
    (8294, 8303),
    (65279, 65279),
    (65529, 65531),
    (69821, 69821),
    (69837, 69837),
    (78896, 78911),
    (113824, 113827),
    (119155, 119162),
    (917505, 917505),
    (917536, 917631),
];

/// JavaScript's `\s` (and `trim`): not `char::is_whitespace`, which has U+0085 but not U+FEFF.
fn is_js_space(c: char) -> bool {
    matches!(c as u32,
        0x09..=0x0D | 0x20 | 0xA0 | 0x1680 | 0x2000..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F
        | 0x3000 | 0xFEFF)
}

fn forbidden_in_raw_model(c: char) -> bool {
    let u = c as u32;
    u <= 0x1F || (0x7F..=0x9F).contains(&u) || CF_RANGES.iter().any(|&(a, b)| (a..=b).contains(&u))
}

/// Like `raw.replace(/\((?:R|TM)\)/gi, "")`: one left-to-right pass, ASCII case-insensitive.
fn strip_marks(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '(' {
            let rest = &chars[i + 1..];
            let n = if rest.len() >= 2 && rest[0].eq_ignore_ascii_case(&'r') && rest[1] == ')' {
                Some(3)
            } else if rest.len() >= 3
                && rest[0].eq_ignore_ascii_case(&'t')
                && rest[1].eq_ignore_ascii_case(&'m')
                && rest[2] == ')'
            {
                Some(4)
            } else {
                None
            };
            if let Some(n) = n {
                i += n;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Applied once, as the server does: not idempotent (`((R)R)` becomes `(R)`).
pub fn normalize_model(raw: &str) -> Model {
    let mut display = String::with_capacity(raw.len());
    let mut pending_space = false;
    for c in strip_marks(raw).chars().filter(|&c| c != '®' && c != '™') {
        if is_js_space(c) {
            pending_space = true;
        } else {
            if pending_space && !display.is_empty() {
                display.push(' ');
            }
            pending_space = false;
            display.push(c);
        }
    }
    let key = display.to_lowercase();
    Model { display, key }
}

fn is_ascii_digits(s: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())
}

fn app_version_ok(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    s.len() <= 16 && parts.len() == 3 && parts.iter().all(|p| is_ascii_digits(p, 1, 16))
}

fn os_build_ok(s: &str) -> bool {
    match s.split_once('.') {
        None => is_ascii_digits(s, 4, 6),
        Some((a, b)) => is_ascii_digits(a, 4, 6) && is_ascii_digits(b, 1, 6),
    }
}

fn flag_ok(s: &str) -> bool {
    (1..=32).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

pub fn known_version(category: &str) -> Option<&'static str> {
    match category {
        "cpu" => Some(SCORE_VERSION),
        "gpu" => Some(GPU_SCORE_VERSION),
        "disk" => Some(DISK_SCORE_VERSION),
        _ => None,
    }
}

fn score_keys(category: &str) -> &'static [(&'static str, Board)] {
    match category {
        "cpu" => &[("single", Board::CpuSingle), ("multi", Board::CpuMulti)],
        "gpu" => &[
            ("compute", Board::GpuCompute),
            ("graphics", Board::GpuGraphics),
        ],
        _ => &[("points", Board::Disk)],
    }
}

/// Same checks, same order as `validateSubmission` in the Worker.
pub fn validate_submission(body: &Value) -> Result<Submission, ErrorCode> {
    use ErrorCode::*;
    let obj = body.as_object().ok_or(BadSchema)?;
    let app_version = obj
        .get("appVersion")
        .and_then(Value::as_str)
        .filter(|s| app_version_ok(s))
        .ok_or(BadSchema)?;
    let category = obj
        .get("category")
        .and_then(Value::as_str)
        .filter(|c| matches!(*c, "cpu" | "gpu" | "disk"))
        .ok_or(BadSchema)?;
    let score_version = obj
        .get("scoreVersion")
        .and_then(Value::as_str)
        .ok_or(BadSchema)?;
    let valid = obj.get("valid").and_then(Value::as_bool).ok_or(BadSchema)?;
    let overclock = obj
        .get("overclock")
        .and_then(Value::as_bool)
        .ok_or(BadSchema)?;
    let scores = obj
        .get("scores")
        .and_then(Value::as_object)
        .ok_or(BadSchema)?;
    let mut values = Vec::new();
    for (key, board) in score_keys(category) {
        let v = scores
            .get(*key)
            .filter(|v| v.is_number())
            .ok_or(BadSchema)?;
        values.push((*board, v.as_f64().ok_or(BadSchema)?));
    }
    match obj.get("kernels") {
        Some(Value::Array(k)) if k.len() <= 32 => {}
        _ => return Err(BadSchema),
    }
    let hw = obj
        .get("hardware")
        .and_then(Value::as_object)
        .ok_or(BadSchema)?;
    let raw_model = hw
        .get("model")
        .and_then(Value::as_str)
        .filter(|m| !m.chars().any(forbidden_in_raw_model))
        .ok_or(BadSchema)?;
    let model = normalize_model(raw_model);
    let model_len = model.display.encode_utf16().count();
    if !(1..=MODEL_MAX).contains(&model_len) {
        return Err(BadSchema);
    }
    let ram = hw
        .get("ramGB")
        .filter(|v| v.is_number())
        .and_then(Value::as_f64)
        .filter(|r| r.fract() == 0.0 && (1.0..=4096.0).contains(r))
        .ok_or(BadSchema)?;
    let os_build = hw
        .get("osBuild")
        .and_then(Value::as_str)
        .filter(|s| os_build_ok(s))
        .ok_or(BadSchema)?;
    let flags = match obj.get("flags") {
        Some(Value::Array(f)) if f.len() <= 16 => f
            .iter()
            .map(|v| v.as_str().filter(|s| flag_ok(s)).map(str::to_owned))
            .collect::<Option<Vec<_>>>()
            .ok_or(BadSchema)?,
        _ => return Err(BadSchema),
    };

    if obj.get("format").and_then(Value::as_f64) != Some(1.0) {
        return Err(BadFormat);
    }
    if known_version(category) != Some(score_version) {
        return Err(UnknownVersion);
    }
    if !valid {
        return Err(NotValid);
    }
    if values
        .iter()
        .any(|&(_, v)| !v.is_finite() || v <= 0.0 || v > VALUE_CAP)
    {
        return Err(BadValue);
    }
    Ok(Submission {
        category: category.to_owned(),
        score_version: score_version.to_owned(),
        overclock,
        app_version: app_version.to_owned(),
        os_build: os_build.to_owned(),
        ram_gb: ram as u32,
        flags,
        model,
        values,
    })
}

pub fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut s = values.to_vec();
    s.sort_by(f64::total_cmp);
    let mid = s.len() / 2;
    if s.len() % 2 == 1 {
        s[mid]
    } else {
        (s[mid - 1] + s[mid]) / 2.0
    }
}

/// Like `parseTable` of the Worker, plus: `n` must be an integer >= 0 and the file at most
/// `MAX_TABLE_BYTES`. Bad rows are skipped; a bad file gives `None`.
pub fn parse_table(bytes: &[u8]) -> Option<Table> {
    if bytes.len() > MAX_TABLE_BYTES {
        return None;
    }
    let t: Value = serde_json::from_slice(bytes).ok()?;
    let obj = t.as_object()?;
    if obj.get("format").and_then(Value::as_f64) != Some(1.0) {
        return None;
    }
    let rows = obj
        .get("rows")?
        .as_array()?
        .iter()
        .filter(|r| {
            let value = r.get("value").and_then(Value::as_f64);
            let n = r.get("n").and_then(Value::as_f64);
            value.is_some_and(|v| v.is_finite() && v > 0.0)
                && n.is_some_and(|n| n.fract() == 0.0 && (0.0..=u32::MAX as f64).contains(&n))
        })
        .filter_map(|r| serde_json::from_value::<TableRow>(r.clone()).ok())
        .collect();
    let generated_at = obj
        .get("generatedAt")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Some(Table { generated_at, rows })
}

pub fn plausible(board: Board, version: &str, key: &str, value: f64, rows: &[TableRow]) -> bool {
    let same: Vec<&TableRow> = rows
        .iter()
        .filter(|r| r.category == board && r.score_version == version)
        .collect();
    if same.is_empty() {
        return true;
    }
    let own: Vec<f64> = same
        .iter()
        .filter(|r| normalize_model(&r.model).key == key)
        .map(|r| r.value)
        .collect();
    if !own.is_empty() {
        let reference = median(&own);
        return PLAUSIBLE_MIN * reference <= value && value <= PLAUSIBLE_MAX * reference;
    }
    // Unknown model: the band of the whole category, so slow hardware still passes.
    let min = same.iter().map(|r| r.value).fold(f64::INFINITY, f64::min);
    let max = same
        .iter()
        .map(|r| r.value)
        .fold(f64::NEG_INFINITY, f64::max);
    PLAUSIBLE_MIN * min <= value && value <= PLAUSIBLE_MAX * max
}

/// The author's rows shipped with the app (`reference-scores.json`), read once.
pub fn author_rows() -> &'static [TableRow] {
    static ROWS: OnceLock<Vec<TableRow>> = OnceLock::new();
    ROWS.get_or_init(|| {
        parse_table(include_bytes!("reference-scores.json"))
            .map(|t| t.rows)
            .unwrap_or_default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    const NORMALIZE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../testdata/scores/normalize.json"
    ));
    const SUBMISSIONS: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../testdata/scores/submissions.json"
    ));
    const FORMAT_CHARS: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../testdata/scores/format-chars.json"
    ));

    fn fixture(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    fn row(board: Board, version: &str, model: &str, value: f64) -> TableRow {
        TableRow {
            category: board,
            score_version: version.into(),
            model: model.into(),
            value,
            n: 1,
            source: Source::Community,
        }
    }

    #[test]
    fn normalize_matches_the_fixture() {
        for c in fixture(NORMALIZE).as_array().unwrap() {
            let input = c["input"].as_str().unwrap();
            let m = normalize_model(input);
            assert_eq!(m.display, c["display"].as_str().unwrap(), "{input:?}");
            assert_eq!(m.key, c["key"].as_str().unwrap(), "{input:?}");
        }
    }

    #[test]
    fn valid_submissions_pass() {
        let f = fixture(SUBMISSIONS);
        for c in f["valid"].as_array().unwrap() {
            let name = c["name"].as_str().unwrap();
            let s = validate_submission(&c["body"]).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            let boards: Vec<Board> = s.values.iter().map(|v| v.0).collect();
            let want: &[Board] = match s.category.as_str() {
                "cpu" => &[Board::CpuSingle, Board::CpuMulti],
                "gpu" => &[Board::GpuCompute, Board::GpuGraphics],
                _ => &[Board::Disk],
            };
            assert_eq!(boards, want, "{name}");
        }
        let disk = f["valid"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "disk_with_throughput")
            .unwrap();
        let s = validate_submission(&disk["body"]).unwrap();
        assert_eq!(s.values, vec![(Board::Disk, 900.0)]);
        let float = f["valid"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "ram_integral_float")
            .unwrap();
        assert_eq!(validate_submission(&float["body"]).unwrap().ram_gb, 32);
    }

    #[test]
    fn invalid_submissions_give_their_code() {
        for c in fixture(SUBMISSIONS)["invalid"].as_array().unwrap() {
            let name = c["name"].as_str().unwrap();
            let err = validate_submission(&c["body"]).unwrap_err();
            assert_eq!(err.as_str(), c["error"].as_str().unwrap(), "{name}");
        }
    }

    #[test]
    fn cf_table_matches_the_fixture() {
        let want: Vec<(u32, u32)> = fixture(FORMAT_CHARS)["cf"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (r[0].as_u64().unwrap() as u32, r[1].as_u64().unwrap() as u32))
            .collect();
        assert_eq!(CF_RANGES, want.as_slice());
    }

    #[test]
    fn rejects_non_object_bodies() {
        for b in [json!([]), json!(null), json!(1), json!("x")] {
            assert_eq!(validate_submission(&b).unwrap_err(), ErrorCode::BadSchema);
        }
    }

    #[test]
    fn too_many_kernels_is_bad_schema() {
        let mut body = fixture(SUBMISSIONS)["valid"][0]["body"].clone();
        body["kernels"] = json!(vec![json!({}); 33]);
        assert_eq!(
            validate_submission(&body).unwrap_err(),
            ErrorCode::BadSchema
        );
    }

    #[test]
    fn parse_table_skips_bad_rows_and_rejects_bad_files() {
        let good = |v: &str, n: &str| {
            format!(
                r#"{{"category":"disk","scoreVersion":"disk-1","model":"X","value":{v},"n":{n},"source":"community"}}"#
            )
        };
        let file = |rows: Vec<String>| {
            format!(
                r#"{{"format":1,"generatedAt":"2026-10-09T00:00:00Z","rows":[{}]}}"#,
                rows.join(",")
            )
        };
        let t =
            parse_table(file(vec![good("5", "3"), good("-1", "3"), good("5", "1.5")]).as_bytes())
                .unwrap();
        assert_eq!(t.rows.len(), 1);
        assert_eq!(t.rows[0].value, 5.0);
        assert_eq!(t.generated_at.as_deref(), Some("2026-10-09T00:00:00Z"));
        assert!(parse_table(br#"{"format":2,"rows":[]}"#).is_none());
        assert!(parse_table(b"not json").is_none());
        let mut big = file(vec![good("5", "3")]).into_bytes();
        big.resize(MAX_TABLE_BYTES + 1, b' ');
        assert!(parse_table(&big).is_none());
    }

    #[test]
    fn plausibility_matches_the_normalized_model() {
        let rows = author_rows();
        let key = normalize_model("AMD Radeon(TM) Graphics").key;
        assert!(!plausible(Board::GpuCompute, "gpu-1", &key, 140.0, rows));
        assert!(plausible(Board::GpuCompute, "gpu-1", &key, 100.0, rows));
    }

    #[test]
    fn unknown_model_uses_the_category_band() {
        let rows = vec![
            row(Board::Disk, "disk-1", "A", 10.0),
            row(Board::Disk, "disk-1", "B", 1000.0),
        ];
        for v in [2.0, 5000.0] {
            assert!(plausible(Board::Disk, "disk-1", "zzz", v, &rows), "{v}");
        }
        for v in [1.9, 5001.0] {
            assert!(!plausible(Board::Disk, "disk-1", "zzz", v, &rows), "{v}");
        }
    }

    #[test]
    fn empty_category_is_plausible() {
        let rows = vec![row(Board::Disk, "disk-1", "A", 10.0)];
        assert!(plausible(Board::CpuSingle, "cpu-1", "x", 1e9, &rows));
        assert!(plausible(Board::Disk, "disk-2", "x", 1e9, &rows));
    }

    #[test]
    fn median_of_odd_and_even() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), 2.5);
    }

    #[test]
    fn author_rows_are_all_author() {
        assert!(!author_rows().is_empty());
        assert!(author_rows().iter().all(|r| r.source == Source::Author));
    }

    #[test]
    fn known_versions() {
        assert_eq!(known_version("cpu"), Some("cpu-1"));
        assert_eq!(known_version("gpu"), Some("gpu-1"));
        assert_eq!(known_version("disk"), Some("disk-1"));
        assert_eq!(known_version("x"), None);
    }
}
