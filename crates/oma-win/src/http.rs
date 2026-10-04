//! HTTPS GET over WinHTTP (spec M6c §2.1), used only by the update check on a
//! background thread. Synchronous; TLS 1.2/1.3 only (TLS 1.2 alone where
//! WinHTTP does not know TLS 1.3), no cookies, redirects only from HTTPS to
//! HTTPS. WinHTTP validates the server certificate chain and name against the
//! Windows store, with no option to ignore errors. Revocation checking
//! (`WINHTTP_ENABLE_SSL_REVOCATION`) is deliberately not enabled: behind
//! proxies an unreachable CRL/OCSP server would fail every check.

use std::ffi::c_void;
use std::time::{Duration, Instant};

use oma_core::updates::CheckError;
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::GetLastError;
use windows::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpCrackUrl, WinHttpOpen, WinHttpOpenRequest,
    WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts, ERROR_WINHTTP_CANNOT_CONNECT,
    ERROR_WINHTTP_CLIENT_AUTH_CERT_NEEDED, ERROR_WINHTTP_CONNECTION_ERROR,
    ERROR_WINHTTP_NAME_NOT_RESOLVED, ERROR_WINHTTP_SECURE_CERT_CN_INVALID,
    ERROR_WINHTTP_SECURE_CERT_DATE_INVALID, ERROR_WINHTTP_SECURE_CERT_REV_FAILED,
    ERROR_WINHTTP_SECURE_CERT_WRONG_USAGE, ERROR_WINHTTP_SECURE_CHANNEL_ERROR,
    ERROR_WINHTTP_SECURE_FAILURE, ERROR_WINHTTP_SECURE_INVALID_CA,
    ERROR_WINHTTP_SECURE_INVALID_CERT, ERROR_WINHTTP_TIMEOUT, URL_COMPONENTS,
    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_DISABLE_COOKIES, WINHTTP_FLAG_SECURE,
    WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2, WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3,
    WINHTTP_INTERNET_SCHEME_HTTPS, WINHTTP_OPTION_DISABLE_FEATURE, WINHTTP_OPTION_REDIRECT_POLICY,
    WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP, WINHTTP_OPTION_SECURE_PROTOCOLS,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
};

// `URL_COMPONENTS` comes from the `windows` crate; pin its layout anyway,
// since `crack_url` fills it in place.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<URL_COMPONENTS>() == 104);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Maps a WinHTTP error code (`GetLastError`) to a check error category.
pub(crate) fn classify(code: u32) -> CheckError {
    match code {
        ERROR_WINHTTP_NAME_NOT_RESOLVED
        | ERROR_WINHTTP_CANNOT_CONNECT
        | ERROR_WINHTTP_CONNECTION_ERROR => CheckError::Offline,
        ERROR_WINHTTP_TIMEOUT => CheckError::Timeout,
        ERROR_WINHTTP_SECURE_FAILURE
        | ERROR_WINHTTP_SECURE_CERT_DATE_INVALID
        | ERROR_WINHTTP_SECURE_CERT_CN_INVALID
        | ERROR_WINHTTP_CLIENT_AUTH_CERT_NEEDED
        | ERROR_WINHTTP_SECURE_INVALID_CA
        | ERROR_WINHTTP_SECURE_CERT_REV_FAILED
        | ERROR_WINHTTP_SECURE_CHANNEL_ERROR
        | ERROR_WINHTTP_SECURE_INVALID_CERT
        | ERROR_WINHTTP_SECURE_CERT_WRONG_USAGE => CheckError::Tls,
        _ => {
            // The category alone would hide the cause; keep the raw code.
            tracing::warn!(code, "update check: unclassified WinHTTP error");
            CheckError::Invalid
        }
    }
}

/// Classifies the calling thread's last error, for the WinHTTP calls that
/// signal failure with a null handle.
fn last_error() -> CheckError {
    // SAFETY: `GetLastError` only reads the calling thread's error slot.
    classify(unsafe { GetLastError() }.0)
}

/// Classifies an error from a `windows` wrapper; those build the error from
/// `GetLastError` as `HRESULT_FROM_WIN32(code)`, so the low word is the code.
fn win_error(error: windows::core::Error) -> CheckError {
    let hresult = error.code().0 as u32;
    let code = if hresult & 0xFFFF_0000 == 0x8007_0000 {
        hresult & 0xFFFF
    } else {
        hresult
    };
    classify(code)
}

/// A WinHTTP handle (session, connection or request), closed on drop.
struct Handle(*mut c_void);

impl Handle {
    fn new(raw: *mut c_void) -> Result<Handle, CheckError> {
        if raw.is_null() {
            Err(last_error())
        } else {
            Ok(Handle(raw))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a non-null handle returned by WinHTTP, owned by
        // this wrapper and closed exactly once. Locals drop in reverse order,
        // so a request closes before its connection and session.
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}

/// Overall deadline of one request. It is checked between WinHTTP calls and
/// bounds each phase through `WinHttpSetTimeouts`; it is not a hard bound on
/// the whole request: redirects repeat phases, and WPAD proxy discovery
/// (`WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY`) is not bounded by these timeouts.
struct Deadline(Instant);

impl Deadline {
    /// Time left in milliseconds for WinHTTP's timeouts (at least 1 ms);
    /// `Timeout` once the deadline has passed.
    fn remaining_ms(&self) -> Result<i32, CheckError> {
        let left = self.0.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(CheckError::Timeout);
        }
        Ok(i32::try_from(left.as_millis()).unwrap_or(i32::MAX).max(1))
    }

    /// Before a call that may resolve, connect, send and receive (session
    /// setup, `WinHttpSendRequest`, `WinHttpReceiveResponse` which may follow
    /// a redirect): shares the time left across the four phase timeouts.
    fn arm_phases(&self, handle: &Handle) -> Result<(), CheckError> {
        let [resolve, connect, send, receive] = split_timeouts(self.remaining_ms()?);
        set_timeouts(handle, resolve, connect, send, receive)
    }

    /// Before a body read, where only the receive timeout applies: all four
    /// timeouts get the time left.
    fn arm_receive(&self, handle: &Handle) -> Result<(), CheckError> {
        let ms = self.remaining_ms()?;
        set_timeouts(handle, ms, ms, ms, ms)
    }
}

fn set_timeouts(
    handle: &Handle,
    resolve: i32,
    connect: i32,
    send: i32,
    receive: i32,
) -> Result<(), CheckError> {
    // SAFETY: `handle` is a live WinHTTP session or request handle.
    unsafe { WinHttpSetTimeouts(handle.0, resolve, connect, send, receive) }.map_err(win_error)
}

/// Lowest per-phase timeout, unless less time than that is left.
const MIN_PHASE_MS: i32 = 250;

/// Per-phase timeouts (resolve, connect, send, receive) for the time left:
/// a quarter each, at least `MIN_PHASE_MS` but never more than the time left.
fn split_timeouts(remaining_ms: i32) -> [i32; 4] {
    let ms = (remaining_ms / 4)
        .max(MIN_PHASE_MS.min(remaining_ms))
        .max(1);
    [ms; 4]
}

/// Sets `WINHTTP_OPTION_SECURE_PROTOCOLS` through `set`: TLS 1.2 and 1.3,
/// or TLS 1.2 alone when that fails (older WinHTTP rejects the TLS 1.3 flag
/// with `ERROR_INVALID_PARAMETER`).
fn with_tls_fallback(mut set: impl FnMut(u32) -> Result<(), CheckError>) -> Result<(), CheckError> {
    set(WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2 | WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3)
        .or_else(|_| set(WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2))
}

/// The parts of an `https` URL needed to connect.
struct Target {
    host: Vec<u16>,
    port: u16,
    object: Vec<u16>,
}

/// Splits `url` with `WinHttpCrackUrl`; anything but a valid `https` URL
/// with a host is `Invalid`.
fn crack_url(url: &str) -> Result<Target, CheckError> {
    if url.is_empty() || url.contains('\0') {
        return Err(CheckError::Invalid);
    }
    let wide: Vec<u16> = url.encode_utf16().collect();
    let mut parts = URL_COMPONENTS {
        dwStructSize: std::mem::size_of::<URL_COMPONENTS>() as u32,
        // Non-zero lengths with null pointers: WinHTTP points each part into
        // `wide` instead of copying it.
        dwSchemeLength: u32::MAX,
        dwHostNameLength: u32::MAX,
        dwUserNameLength: u32::MAX,
        dwPasswordLength: u32::MAX,
        dwUrlPathLength: u32::MAX,
        dwExtraInfoLength: u32::MAX,
        ..Default::default()
    };
    // SAFETY: `wide` is a non-empty buffer passed with its length; `parts`
    // is an initialised `URL_COMPONENTS` with its size set.
    unsafe { WinHttpCrackUrl(&wide, 0, &mut parts) }.map_err(|_| CheckError::Invalid)?;
    // Credentials in the URL are never sent.
    if parts.nScheme != WINHTTP_INTERNET_SCHEME_HTTPS
        || parts.dwUserNameLength != 0
        || parts.dwPasswordLength != 0
    {
        return Err(CheckError::Invalid);
    }
    let base = wide.as_ptr_range();
    let part = |ptr: *mut u16, len: u32| -> Result<Vec<u16>, CheckError> {
        if ptr.is_null() || len == 0 {
            return Ok(Vec::new());
        }
        let end = ptr.wrapping_add(len as usize);
        if !(base.start..=base.end).contains(&(ptr as *const u16))
            || !(base.start..=base.end).contains(&(end as *const u16))
        {
            return Err(CheckError::Invalid);
        }
        // SAFETY: `ptr..ptr + len` was just checked to lie inside `wide`,
        // which is alive and not mutated while borrowed here.
        Ok(unsafe { std::slice::from_raw_parts(ptr, len as usize) }.to_vec())
    };
    let mut host = part(parts.lpszHostName.0, parts.dwHostNameLength)?;
    if host.is_empty() {
        return Err(CheckError::Invalid);
    }
    let mut object = part(parts.lpszUrlPath.0, parts.dwUrlPathLength)?;
    object.extend(part(parts.lpszExtraInfo.0, parts.dwExtraInfoLength)?);
    // The fragment belongs to the client, never to the request line.
    if let Some(hash) = object.iter().position(|&c| c == u16::from(b'#')) {
        object.truncate(hash);
    }
    if object.is_empty() {
        object.push(u16::from(b'/'));
    }
    host.push(0);
    object.push(0);
    Ok(Target {
        host,
        port: parts.nPort,
        object,
    })
}

/// `Name: value\r\n` lines for `WinHttpSendRequest`; a line break or NUL in
/// a name or value (or a colon in a name) is `Invalid`.
fn header_block(headers: &[(&str, &str)]) -> Result<Vec<u16>, CheckError> {
    let bad = |s: &str| s.contains(['\r', '\n', '\0']);
    let mut block = String::new();
    for (name, value) in headers {
        if name.is_empty() || bad(name) || name.contains(':') || bad(value) {
            return Err(CheckError::Invalid);
        }
        block.push_str(name);
        block.push_str(": ");
        block.push_str(value);
        block.push_str("\r\n");
    }
    Ok(block.encode_utf16().collect())
}

fn set_u32_option(handle: &Handle, option: u32, value: u32) -> Result<(), CheckError> {
    let bytes = value.to_ne_bytes();
    // SAFETY: `handle` is a live WinHTTP handle; the buffer is a DWORD, as
    // every option set here expects.
    unsafe { WinHttpSetOption(Some(handle.0), option, Some(&bytes)) }.map_err(win_error)
}

/// Sends `GET url` over HTTPS and reads the response. `deadline` is checked
/// between WinHTTP calls (`Timeout` once it has passed) and bounds each
/// phase through the WinHTTP timeouts; it is not a hard bound on the whole
/// request, since redirects repeat phases and WPAD proxy discovery is not
/// bounded. Any HTTP status is returned as is (the caller judges it); a body
/// larger than `max_body` is `Invalid`.
pub fn get(
    url: &str,
    user_agent: &str,
    headers: &[(&str, &str)],
    deadline: Duration,
    max_body: usize,
) -> Result<HttpResponse, CheckError> {
    let deadline = Deadline(Instant::now() + deadline);
    let target = crack_url(url)?;
    let header_block = header_block(headers)?;
    if user_agent.contains(['\r', '\n', '\0']) {
        return Err(CheckError::Invalid);
    }
    let agent = HSTRING::from(user_agent);

    // SAFETY: `agent` is a NUL-terminated wide string alive for the call; no
    // named proxy, so both proxy strings are null.
    let session = Handle::new(unsafe {
        WinHttpOpen(
            &agent,
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        )
    })?;
    with_tls_fallback(|flags| set_u32_option(&session, WINHTTP_OPTION_SECURE_PROTOCOLS, flags))?;
    deadline.arm_phases(&session)?;

    // SAFETY: `session` is live; `target.host` is NUL-terminated and alive.
    let connection = Handle::new(unsafe {
        WinHttpConnect(session.0, PCWSTR(target.host.as_ptr()), target.port, 0)
    })?;

    // SAFETY: `connection` is live; verb and object name are NUL-terminated
    // and alive; default version, no referrer, no accept types.
    let request = Handle::new(unsafe {
        WinHttpOpenRequest(
            connection.0,
            w!("GET"),
            PCWSTR(target.object.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        )
    })?;
    set_u32_option(
        &request,
        WINHTTP_OPTION_DISABLE_FEATURE,
        WINHTTP_DISABLE_COOKIES,
    )?;
    set_u32_option(
        &request,
        WINHTTP_OPTION_REDIRECT_POLICY,
        WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP,
    )?;

    deadline.arm_phases(&request)?;
    let extra = (!header_block.is_empty()).then_some(header_block.as_slice());
    // SAFETY: `request` is live; the header block is passed with its length
    // and outlives the call; no request body.
    unsafe { WinHttpSendRequest(request.0, extra, None, 0, 0, 0) }.map_err(win_error)?;

    deadline.arm_phases(&request)?;
    // SAFETY: `request` is live and was sent; the reserved pointer is null.
    unsafe { WinHttpReceiveResponse(request.0, std::ptr::null_mut()) }.map_err(win_error)?;

    let mut status: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: `request` has a response; the buffer is a DWORD of `size`
    // bytes, as `WINHTTP_QUERY_FLAG_NUMBER` requires; no header index.
    unsafe {
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut c_void),
            &mut size,
            std::ptr::null_mut(),
        )
    }
    .map_err(win_error)?;
    let status = u16::try_from(status).map_err(|_| CheckError::Invalid)?;

    let mut body = Vec::new();
    loop {
        deadline.arm_receive(&request)?;
        let mut available: u32 = 0;
        // SAFETY: `request` is live with a received response; valid out-pointer.
        unsafe { WinHttpQueryDataAvailable(request.0, &mut available) }.map_err(win_error)?;
        if available == 0 {
            break;
        }
        let available = available as usize;
        if available > max_body - body.len() {
            tracing::warn!(
                read = body.len(),
                available,
                max_body,
                "update check: response body too large"
            );
            return Err(CheckError::Invalid);
        }
        let start = body.len();
        body.resize(start + available, 0);
        deadline.arm_receive(&request)?;
        let mut read: u32 = 0;
        // SAFETY: `body[start..]` is `available` writable bytes owned by
        // `body`; `read` is a valid out-pointer.
        unsafe {
            WinHttpReadData(
                request.0,
                body[start..].as_mut_ptr().cast(),
                available as u32,
                &mut read,
            )
        }
        .map_err(win_error)?;
        body.truncate(start + (read as usize).min(available));
        if read == 0 {
            break;
        }
    }
    Ok(HttpResponse { status, body })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oma_core::updates::{
        parse_latest, user_agent, LATEST_RELEASE_URL, MAX_BODY_BYTES, REQUEST_HEADERS,
    };

    #[test]
    fn classify_maps_winhttp_errors() {
        assert_eq!(classify(12007), CheckError::Offline);
        assert_eq!(classify(12029), CheckError::Offline);
        assert_eq!(classify(12002), CheckError::Timeout);
        assert_eq!(classify(12030), CheckError::Offline);
        for code in [
            12175, 12037, 12038, 12044, 12045, 12057, 12157, 12169, 12179,
        ] {
            assert_eq!(classify(code), CheckError::Tls, "code {code}");
        }
        assert_eq!(classify(12152), CheckError::Invalid);
        assert_eq!(classify(5), CheckError::Invalid);
    }

    #[test]
    fn non_https_urls_are_rejected_without_connecting() {
        for url in ["http://example.com/", "ftp://example.com/", "not a url", ""] {
            let result = get(url, "test", &[], Duration::from_secs(10), 1024);
            assert_eq!(result, Err(CheckError::Invalid), "url {url:?}");
        }
    }

    #[test]
    fn header_line_breaks_are_rejected_without_connecting() {
        let headers = [("X-Test", "a\r\nInjected: b")];
        let result = get(
            "https://example.invalid/",
            "test",
            &headers,
            Duration::from_secs(10),
            1024,
        );
        assert_eq!(result, Err(CheckError::Invalid));
    }

    fn text(wide: &[u16]) -> String {
        String::from_utf16(wide.strip_suffix(&[0]).expect("NUL-terminated")).unwrap()
    }

    #[test]
    fn crack_url_splits_an_https_url() {
        let target = crack_url("https://api.example.com/repos/x/releases/latest").unwrap();
        assert_eq!(text(&target.host), "api.example.com");
        assert_eq!(target.port, 443);
        assert_eq!(text(&target.object), "/repos/x/releases/latest");
    }

    #[test]
    fn crack_url_keeps_port_and_query_and_drops_the_fragment() {
        let target = crack_url("https://example.com:8443/a/b?x=1&y=2#frag").unwrap();
        assert_eq!(text(&target.host), "example.com");
        assert_eq!(target.port, 8443);
        assert_eq!(text(&target.object), "/a/b?x=1&y=2");
    }

    #[test]
    fn crack_url_uses_a_slash_for_an_empty_path() {
        let target = crack_url("https://example.com").unwrap();
        assert_eq!(text(&target.object), "/");
    }

    #[test]
    fn crack_url_rejects_credentials() {
        for url in [
            "https://user@example.com/",
            "https://user:secret@example.com/",
        ] {
            assert_eq!(
                crack_url(url).err(),
                Some(CheckError::Invalid),
                "url {url:?}"
            );
        }
    }

    #[test]
    fn user_agent_control_characters_are_rejected_without_connecting() {
        for agent in ["a\rb", "a\nb", "a\0b"] {
            let result = get(
                "https://example.invalid/",
                agent,
                &[],
                Duration::from_secs(10),
                1024,
            );
            assert_eq!(result, Err(CheckError::Invalid), "agent {agent:?}");
        }
    }

    #[test]
    fn split_timeouts_shares_the_time_left_across_phases() {
        assert_eq!(split_timeouts(10_000), [2_500; 4]);
        // A floor keeps each phase usable, never above the time left.
        assert_eq!(split_timeouts(400), [250; 4]);
        assert_eq!(split_timeouts(100), [100; 4]);
        assert_eq!(split_timeouts(1), [1; 4]);
    }

    #[test]
    fn tls_fallback_retries_with_tls_1_2_alone() {
        let both = WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2 | WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3;
        let mut calls = Vec::new();
        let result = with_tls_fallback(|flags| {
            calls.push(flags);
            if flags == both {
                Err(CheckError::Invalid)
            } else {
                Ok(())
            }
        });
        assert_eq!(result, Ok(()));
        assert_eq!(calls, [both, WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2]);
    }

    #[test]
    fn tls_fallback_stops_when_both_succeed() {
        let mut calls = 0;
        assert_eq!(
            with_tls_fallback(|_| {
                calls += 1;
                Ok(())
            }),
            Ok(())
        );
        assert_eq!(calls, 1);
    }

    #[test]
    fn tls_fallback_fails_when_tls_1_2_fails_too() {
        let mut calls = 0;
        let result = with_tls_fallback(|_| {
            calls += 1;
            Err(CheckError::Invalid)
        });
        assert_eq!(result, Err(CheckError::Invalid));
        assert_eq!(calls, 2);
    }

    #[test]
    #[ignore = "requires network"]
    fn an_unresolvable_host_is_offline() {
        let result = get(
            "https://nonexistent.invalid/",
            "test",
            &[],
            Duration::from_secs(10),
            1024,
        );
        assert_eq!(result, Err(CheckError::Offline));
    }

    #[test]
    #[ignore = "requires network"]
    fn fetches_the_latest_release() {
        let response = get(
            LATEST_RELEASE_URL,
            &user_agent("0.0.0"),
            &REQUEST_HEADERS,
            Duration::from_secs(10),
            MAX_BODY_BYTES,
        )
        .expect("request");
        assert_eq!(response.status, 200);
        let release = parse_latest(response.status, &response.body).expect("valid release");
        println!("latest release: {} at {}", release.version, release.url);
    }
}
