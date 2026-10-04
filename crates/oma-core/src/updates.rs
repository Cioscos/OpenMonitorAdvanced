//! Pure logic of the update check: version parsing, GitHub response
//! validation, persisted state and the check schedule. No I/O here.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Endpoint of the "latest release" REST call.
pub const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/Cioscos/OpenMonitorAdvanced/releases/latest";
/// Only release pages under this prefix are accepted (and ever opened).
pub const RELEASE_PAGE_PREFIX: &str = "https://github.com/Cioscos/OpenMonitorAdvanced/releases/";
/// Headers sent with every request, besides `User-Agent`.
pub const REQUEST_HEADERS: [(&str, &str); 2] = [
    ("Accept", "application/vnd.github+json"),
    ("X-GitHub-Api-Version", "2022-11-28"),
];

pub const FIRST_CHECK_DELAY_MS: u64 = 60_000;
pub const SUCCESS_INTERVAL_MS: u64 = 86_400_000;
pub const RETRY_INTERVAL_MS: u64 = 21_600_000;
pub const MAX_BODY_BYTES: usize = 262_144;

/// `User-Agent` header value for the given app version.
pub fn user_agent(version: &str) -> String {
    format!("OpenMonitorAdvanced/{version} (+https://github.com/Cioscos/OpenMonitorAdvanced)")
}

/// Canonical `X.Y.Z` version, each component 0..=65535 without leading zeros.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl Version {
    /// Parses `X.Y.Z` (no `v` prefix, no suffix).
    pub fn parse(text: &str) -> Option<Version> {
        let mut parts = text.split('.');
        let major = parse_component(parts.next()?)?;
        let minor = parse_component(parts.next()?)?;
        let patch = parse_component(parts.next()?)?;
        if parts.next().is_some() {
            return None;
        }
        Some(Version {
            major,
            minor,
            patch,
        })
    }

    /// Parses a tag of the form `vX.Y.Z` (the `v` is mandatory).
    pub fn parse_tag(tag: &str) -> Option<Version> {
        Version::parse(tag.strip_prefix('v')?)
    }
}

fn parse_component(s: &str) -> Option<u16> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) || (s.len() > 1 && s.starts_with('0'))
    {
        return None;
    }
    s.parse().ok()
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A validated published release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckError {
    Offline,
    Timeout,
    Tls,
    Http(u16),
    Invalid,
}

impl CheckError {
    /// Stable category used by the UI and the logs.
    pub fn category(&self) -> &'static str {
        match self {
            CheckError::Offline => "offline",
            CheckError::Timeout => "timeout",
            CheckError::Tls => "tls",
            CheckError::Http(_) => "http",
            CheckError::Invalid => "invalid",
        }
    }
}

#[derive(Deserialize)]
struct RawRelease {
    tag_name: String,
    html_url: String,
    draft: bool,
    prerelease: bool,
}

/// Validates the response of the "latest release" call.
pub fn parse_latest(status: u16, body: &[u8]) -> Result<Release, CheckError> {
    if status != 200 {
        return Err(CheckError::Http(status));
    }
    if body.is_empty() || body.len() > MAX_BODY_BYTES {
        return Err(CheckError::Invalid);
    }
    let raw: RawRelease = serde_json::from_slice(body).map_err(|_| CheckError::Invalid)?;
    if raw.draft || raw.prerelease || !raw.html_url.starts_with(RELEASE_PAGE_PREFIX) {
        return Err(CheckError::Invalid);
    }
    let version = Version::parse_tag(&raw.tag_name).ok_or(CheckError::Invalid)?;
    Ok(Release {
        version,
        url: raw.html_url,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredRelease {
    pub version: String,
    pub url: String,
}

/// Persisted state of the update check. Tolerates unknown fields.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdateState {
    pub last_attempt_ms: Option<u64>,
    pub last_success_ms: Option<u64>,
    pub latest: Option<StoredRelease>,
    pub notified_version: Option<String>,
}

impl UpdateState {
    pub fn record_success(&mut self, now_ms: u64, release: &Release) {
        self.last_attempt_ms = Some(now_ms);
        self.last_success_ms = Some(now_ms);
        self.latest = Some(StoredRelease {
            version: release.version.to_string(),
            url: release.url.clone(),
        });
    }

    /// Touches only the attempt time: the known `latest` is kept.
    pub fn record_failure(&mut self, now_ms: u64) {
        self.last_attempt_ms = Some(now_ms);
    }

    /// The stored release, revalidated (the file may have been edited by
    /// hand), if it is newer than `current`.
    pub fn available(&self, current: Version) -> Option<Release> {
        let stored = self.latest.as_ref()?;
        let version = Version::parse(&stored.version)?;
        if version <= current || !stored.url.starts_with(RELEASE_PAGE_PREFIX) {
            return None;
        }
        Some(Release {
            version,
            url: stored.url.clone(),
        })
    }
}

/// Absolute time (ms) when the next automatic check is due, or `None` when
/// automatic checks are off. The scheduler waits `min(due - now, 1 h)`.
pub fn next_check_ms(now_ms: u64, started_ms: u64, state: &UpdateState, auto: bool) -> Option<u64> {
    if !auto {
        return None;
    }
    let first = started_ms.saturating_add(FIRST_CHECK_DELAY_MS);
    let (attempt, success) = (state.last_attempt_ms, state.last_success_ms);
    // A stored time in the future means the clock went back: start over.
    if attempt.is_some_and(|t| t > now_ms) || success.is_some_and(|t| t > now_ms) {
        return Some(first);
    }
    let due = match (attempt, success) {
        (Some(a), s) if s.is_none_or(|s| a > s) => a.saturating_add(RETRY_INTERVAL_MS),
        (_, Some(s)) => s.saturating_add(SUCCESS_INTERVAL_MS),
        _ => first,
    };
    Some(due.max(first))
}

/// True when `release` is newer than `current` and was not announced yet.
pub fn should_notify(current: Version, state: &UpdateState, release: &Release) -> bool {
    release.version > current
        && state.notified_version.as_deref() != Some(release.version.to_string().as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../testdata/github/latest-release.json");
    const STARTED: u64 = 1_000_000_000; // larger than 30 h so "started - N h" does not underflow
    const H: u64 = 3_600_000;

    fn v(major: u16, minor: u16, patch: u16) -> Version {
        Version {
            major,
            minor,
            patch,
        }
    }

    fn release(version: Version) -> Release {
        Release {
            version,
            url: format!("{RELEASE_PAGE_PREFIX}tag/v{version}"),
        }
    }

    fn body(draft: bool, prerelease: bool, tag: &str, url: &str) -> Vec<u8> {
        serde_json::json!({
            "tag_name": tag, "html_url": url, "draft": draft, "prerelease": prerelease
        })
        .to_string()
        .into_bytes()
    }

    const GOOD_URL: &str = "https://github.com/Cioscos/OpenMonitorAdvanced/releases/tag/v1.0.0";

    #[test]
    fn version_parses_canonical_only() {
        assert_eq!(Version::parse("0.4.0"), Some(v(0, 4, 0)));
        assert_eq!(Version::parse_tag("v1.2.3"), Some(v(1, 2, 3)));
        for bad in [
            "1.2",
            "1.2.3.4",
            "01.2.3",
            "1.2.65536",
            "v1.2.3",
            "v1.2.3-rc1",
            "",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
        assert_eq!(Version::parse_tag("1.2.3"), None);
        assert_eq!(Version::parse_tag("v1.2.3-rc1"), None);
        assert_eq!(v(1, 2, 3).to_string(), "1.2.3");
    }

    #[test]
    fn version_orders_numerically() {
        assert!(v(0, 10, 0) > v(0, 9, 9));
        assert!(v(1, 0, 0) > v(0, 99, 99));
    }

    #[test]
    fn parses_real_release_response() {
        let r = parse_latest(200, FIXTURE).unwrap();
        assert_eq!(r.version, v(0, 3, 0));
        assert_eq!(
            r.url,
            "https://github.com/Cioscos/OpenMonitorAdvanced/releases/tag/v0.3.0"
        );
    }

    #[test]
    fn rejects_draft_prerelease_and_bad_fields() {
        let ok = parse_latest(200, &body(false, false, "v1.0.0", GOOD_URL));
        assert_eq!(ok.unwrap().version, v(1, 0, 0));
        let invalid = |b: &[u8]| assert_eq!(parse_latest(200, b), Err(CheckError::Invalid));
        invalid(&body(true, false, "v1.0.0", GOOD_URL));
        invalid(&body(false, true, "v1.0.0", GOOD_URL));
        invalid(&body(false, false, "release-3", GOOD_URL));
        for url in [
            "https://github.com/Other/Repo/releases/tag/v1.0.0",
            "http://github.com/Cioscos/OpenMonitorAdvanced/releases/tag/v1.0.0",
            "https://github.com/Cioscos/OpenMonitorAdvanced-evil/releases/tag/v1.0.0",
        ] {
            invalid(&body(false, false, "v1.0.0", url));
        }
        invalid(br#"{"tag_name":"v1.0.0"}"#);
        invalid(b"not json");
        invalid(b"");
        assert_eq!(parse_latest(403, b"{}"), Err(CheckError::Http(403)));
        let big = vec![b' '; MAX_BODY_BYTES + 1];
        assert_eq!(parse_latest(200, &big), Err(CheckError::Invalid));
    }

    #[test]
    fn error_categories() {
        assert_eq!(CheckError::Offline.category(), "offline");
        assert_eq!(CheckError::Timeout.category(), "timeout");
        assert_eq!(CheckError::Tls.category(), "tls");
        assert_eq!(CheckError::Http(500).category(), "http");
        assert_eq!(CheckError::Invalid.category(), "invalid");
    }

    #[test]
    fn user_agent_names_version_and_repo() {
        assert_eq!(
            user_agent("0.4.0"),
            "OpenMonitorAdvanced/0.4.0 (+https://github.com/Cioscos/OpenMonitorAdvanced)"
        );
        assert_eq!(
            REQUEST_HEADERS[0],
            ("Accept", "application/vnd.github+json")
        );
        assert_eq!(REQUEST_HEADERS[1], ("X-GitHub-Api-Version", "2022-11-28"));
    }

    #[test]
    fn next_check_schedule() {
        let st = |success: Option<u64>, attempt: Option<u64>| UpdateState {
            last_success_ms: success,
            last_attempt_ms: attempt,
            ..UpdateState::default()
        };
        let now = STARTED + 100 * H;
        let first = STARTED + FIRST_CHECK_DELAY_MS;
        let next = |s: &UpdateState| next_check_ms(now, STARTED, s, true);

        for s in [
            UpdateState::default(),
            st(Some(STARTED + H), Some(STARTED + H)),
        ] {
            assert_eq!(next_check_ms(now, STARTED, &s, false), None);
        }
        assert_eq!(next(&UpdateState::default()), Some(first));
        let t = STARTED + 5 * H;
        assert_eq!(next(&st(Some(t), Some(t))), Some(t + SUCCESS_INTERVAL_MS));
        assert_eq!(
            next(&st(Some(STARTED - 10 * H), Some(STARTED - 10 * H))),
            Some(STARTED + 14 * H)
        );
        let far = STARTED - 30 * H;
        assert_eq!(next(&st(Some(far), Some(far))), Some(first));
        // failed attempt after the last success
        assert_eq!(next(&st(Some(far), Some(t))), Some(t + RETRY_INTERVAL_MS));
        assert_eq!(next(&st(None, Some(t))), Some(t + RETRY_INTERVAL_MS));
        let early = STARTED - 7 * H;
        assert_eq!(next(&st(Some(far), Some(early))), Some(first));
        // clock went backwards
        assert_eq!(next(&st(Some(now + 1), Some(now + 1))), Some(first));
        assert_eq!(next(&st(Some(t), Some(now + 1))), Some(first));
    }

    #[test]
    fn notify_once_per_version() {
        let cur = v(0, 4, 0);
        let mut s = UpdateState::default();
        let r = release(v(0, 5, 0));
        assert!(should_notify(cur, &s, &r));
        s.notified_version = Some("0.5.0".into());
        assert!(!should_notify(cur, &s, &r));
        s.notified_version = Some("0.4.9".into());
        assert!(should_notify(cur, &s, &r));
        let s = UpdateState::default();
        assert!(!should_notify(cur, &s, &release(v(0, 4, 0))));
        assert!(!should_notify(cur, &s, &release(v(0, 3, 0))));
    }

    #[test]
    fn failed_check_keeps_known_latest() {
        let mut s = UpdateState::default();
        s.record_success(1000, &release(v(0, 5, 0)));
        s.record_failure(1001);
        assert_eq!(s.available(v(0, 4, 0)), Some(release(v(0, 5, 0))));
        assert_eq!(s.last_success_ms, Some(1000));
        assert_eq!(s.last_attempt_ms, Some(1001));
    }

    #[test]
    fn available_drops_stale_or_invalid_latest() {
        let mut s = UpdateState::default();
        s.record_success(1, &release(v(0, 4, 0)));
        assert_eq!(s.available(v(0, 4, 0)), None);
        assert_eq!(s.available(v(0, 5, 0)), None);
        s.latest = Some(StoredRelease {
            version: "0.9.0".into(),
            url: "https://evil.example/releases/".into(),
        });
        assert_eq!(s.available(v(0, 4, 0)), None);
        s.latest = Some(StoredRelease {
            version: "garbage".into(),
            url: format!("{RELEASE_PAGE_PREFIX}tag/v9.9.9"),
        });
        assert_eq!(s.available(v(0, 4, 0)), None);
    }

    #[test]
    fn state_round_trips_and_tolerates_garbage() {
        let mut s = UpdateState::default();
        s.record_success(42, &release(v(0, 5, 0)));
        s.notified_version = Some("0.5.0".into());
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("lastAttemptMs") && json.contains("notifiedVersion"));
        assert_eq!(serde_json::from_str::<UpdateState>(&json).unwrap(), s);
        let extra = r#"{"lastSuccessMs":7,"somethingNew":[1,2],"latest":null}"#;
        let p: UpdateState = serde_json::from_str(extra).unwrap();
        assert_eq!(p.last_success_ms, Some(7));
        assert_eq!(p.last_attempt_ms, None);
    }
}
