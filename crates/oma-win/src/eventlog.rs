//! WHEA, bugcheck and Kernel-Power events from the System log (`EvtQuery`).

use std::io;

use oma_core::load::WheaEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventProvider {
    Whea,
    BugCheck,
    KernelPower,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemEvent {
    pub record_id: u64,
    pub provider: EventProvider,
    pub event_id: u32,
    pub time_utc: String,
    pub apic_id: Option<u32>,
}

impl SystemEvent {
    pub fn whea(&self) -> Option<WheaEvent> {
        (self.provider == EventProvider::Whea).then(|| WheaEvent {
            record_id: self.record_id,
            event_id: self.event_id,
            apic_id: self.apic_id,
            time_utc: self.time_utc.clone(),
        })
    }
}

const WHEA_PROVIDER: &str = "Microsoft-Windows-WHEA-Logger";
const BUGCHECK_PROVIDER: &str = "Microsoft-Windows-WER-SystemErrorReporting";
const KERNEL_POWER_PROVIDER: &str = "Microsoft-Windows-Kernel-Power";

fn provider_from_name(name: &str) -> Option<EventProvider> {
    match name {
        WHEA_PROVIDER => Some(EventProvider::Whea),
        BUGCHECK_PROVIDER => Some(EventProvider::BugCheck),
        KERNEL_POWER_PROVIDER => Some(EventProvider::KernelPower),
        _ => None,
    }
}

/// Start tags `<name ...>` of `xml`: the text between the name and the closing `>`
/// (quotes may hide a `>`), plus the rest of the document after that `>`.
fn start_tags<'a>(xml: &'a str, name: &str) -> impl Iterator<Item = (&'a str, &'a str)> {
    let open = format!("<{name}");
    let mut pos = 0;
    std::iter::from_fn(move || loop {
        let at = pos + xml.get(pos..)?.find(&open)?;
        let from = at + open.len();
        pos = from;
        let rest = &xml[from..];
        if !rest.starts_with(|c: char| c.is_whitespace() || c == '/' || c == '>') {
            continue;
        }
        let mut quote = None;
        let end = rest.char_indices().find(|&(_, c)| match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
                false
            }
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                false
            }
            None => c == '>',
        })?;
        return Some((&rest[..end.0], &rest[end.0 + 1..]));
    })
}

/// Value of attribute `name` in a start tag's text, with either quote style.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut pos = 0;
    while let Some(i) = tag.get(pos..)?.find(name) {
        let at = pos + i;
        pos = at + name.len();
        if !tag[..at].ends_with(char::is_whitespace) {
            continue;
        }
        let rest = tag[pos..].trim_start().strip_prefix('=')?.trim_start();
        let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let body = &rest[1..];
        return body.find(quote).map(|e| &body[..e]);
    }
    None
}

/// Text of the first `<name>` element, up to the next tag.
fn element_text<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let (tag, rest) = start_tags(xml, name).next()?;
    if tag.ends_with('/') {
        return None;
    }
    Some(rest[..rest.find('<')?].trim())
}

fn parse_u32(s: &str) -> Option<u32> {
    let s = s.trim();
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => s.parse().ok(),
    }
}

/// Reads one event rendered by `EvtRender(EvtRenderEventXml)`. `None` when the text is not an
/// event of one of the three providers, or lacks its id or record id.
pub fn parse_event_xml(xml: &str) -> Option<SystemEvent> {
    let (provider_tag, _) = start_tags(xml, "Provider").next()?;
    let provider = provider_from_name(attr(provider_tag, "Name")?)?;
    let event_id = parse_u32(element_text(xml, "EventID")?)?;
    let record_id = element_text(xml, "EventRecordID")?.parse().ok()?;
    let time_utc = start_tags(xml, "TimeCreated")
        .next()
        .and_then(|(t, _)| attr(t, "SystemTime"))
        .unwrap_or_default()
        .to_string();
    let apic_id = (provider == EventProvider::Whea && event_id == 19)
        .then(|| {
            start_tags(xml, "Data")
                .find(|(t, _)| attr(t, "Name") == Some("ApicId"))
                .and_then(|(t, rest)| {
                    if t.ends_with('/') {
                        return None;
                    }
                    parse_u32(&rest[..rest.find('<')?])
                })
        })
        .flatten();
    Some(SystemEvent {
        record_id,
        provider,
        event_id,
        time_utc,
        apic_id,
    })
}

const MAX_WHEA_EVENTS: usize = 1000;
const MAX_CRASH_EVENTS: usize = 100;

/// The newest WHEA record of the System log, if any.
pub fn latest_record_id() -> io::Result<Option<u64>> {
    let xpath = format!("*[System[Provider[@Name='{WHEA_PROVIDER}']]]");
    Ok(run_query(&xpath, true, 1)?.first().map(|e| e.record_id))
}

/// WHEA events newer than `record` (all of them when `None`), oldest first, at most 1000.
pub fn whea_after(record: Option<u64>) -> io::Result<Vec<SystemEvent>> {
    let cond = record
        .map(|n| format!(" and EventRecordID > {n}"))
        .unwrap_or_default();
    let xpath = format!("*[System[Provider[@Name='{WHEA_PROVIDER}']{cond}]]");
    run_query(&xpath, false, MAX_WHEA_EVENTS)
}

/// WHEA 17/18/19, bugcheck 1001 and Kernel-Power 41 since `since_utc` (an ISO 8601 UTC
/// timestamp), oldest first, at most 100.
pub fn crash_evidence(since_utc: &str) -> io::Result<Vec<SystemEvent>> {
    if since_utc.is_empty()
        || !since_utc
            .chars()
            .all(|c| c.is_ascii_digit() || "-:.TZ".contains(c))
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "bad timestamp"));
    }
    let xpath = format!(
        "*[System[((Provider[@Name='{WHEA_PROVIDER}'] and (EventID=17 or EventID=18 or EventID=19)) \
         or (Provider[@Name='{BUGCHECK_PROVIDER}'] and EventID=1001) \
         or (Provider[@Name='{KERNEL_POWER_PROVIDER}'] and EventID=41)) \
         and TimeCreated[@SystemTime>='{since_utc}']]]"
    );
    run_query(&xpath, false, MAX_CRASH_EVENTS)
}

use windows::core::HSTRING;
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_NO_MORE_ITEMS};
use windows::Win32::System::EventLog::{
    EvtClose, EvtNext, EvtQuery, EvtQueryChannelPath, EvtQueryForwardDirection,
    EvtQueryReverseDirection, EvtRender, EvtRenderEventXml, EVT_HANDLE,
};

use crate::private_pipe::os_error;

/// Closes the event-log handle on drop.
struct EvtGuard(EVT_HANDLE);

impl Drop for EvtGuard {
    fn drop(&mut self) {
        // SAFETY: the handle came from EvtQuery or EvtNext, is owned by this guard and is closed once.
        unsafe {
            let _ = EvtClose(self.0);
        }
    }
}

fn run_query(xpath: &str, newest_first: bool, max: usize) -> io::Result<Vec<SystemEvent>> {
    let direction = if newest_first {
        EvtQueryReverseDirection
    } else {
        EvtQueryForwardDirection
    };
    // SAFETY: both strings are valid, NUL-terminated HSTRINGs that outlive the call.
    let query = unsafe {
        EvtQuery(
            None,
            &HSTRING::from("System"),
            &HSTRING::from(xpath),
            EvtQueryChannelPath.0 | direction.0,
        )
    }
    .map_err(|e| os_error(&e))?;
    let query = EvtGuard(query);
    let mut out = Vec::new();
    while out.len() < max {
        let mut handles = [0isize; 32];
        let mut returned = 0u32;
        // SAFETY: `handles` has room for the 32 handles requested and `returned` is a valid out pointer.
        let next = unsafe { EvtNext(query.0, &mut handles, u32::MAX, 0, &mut returned) };
        if let Err(e) = next {
            if e.code() == ERROR_NO_MORE_ITEMS.to_hresult() {
                break;
            }
            return Err(os_error(&e));
        }
        // Guards first, so every returned handle is closed even if a render fails.
        let events: Vec<EvtGuard> = handles[..returned as usize]
            .iter()
            .map(|h| EvtGuard(EVT_HANDLE(*h)))
            .collect();
        for event in &events {
            if out.len() < max {
                if let Some(e) = render(event)?.as_deref().and_then(parse_event_xml) {
                    out.push(e);
                }
            }
        }
    }
    Ok(out)
}

/// The event as XML text; `None` when it renders to nothing usable.
fn render(event: &EvtGuard) -> io::Result<Option<String>> {
    let mut used = 0u32;
    let mut props = 0u32;
    // SAFETY: a zero-size call with no buffer only reports the size needed in `used`.
    let first = unsafe {
        EvtRender(
            None,
            event.0,
            EvtRenderEventXml.0,
            0,
            None,
            &mut used,
            &mut props,
        )
    };
    match first {
        Err(e) if e.code() == ERROR_INSUFFICIENT_BUFFER.to_hresult() => {}
        Err(e) => return Err(os_error(&e)),
        Ok(()) => return Ok(None),
    }
    let mut buf = vec![0u16; (used as usize).div_ceil(2)];
    // SAFETY: `buf` holds at least `used` bytes, and the size passed is its real size in bytes.
    unsafe {
        EvtRender(
            None,
            event.0,
            EvtRenderEventXml.0,
            (buf.len() * 2) as u32,
            Some(buf.as_mut_ptr().cast()),
            &mut used,
            &mut props,
        )
    }
    .map_err(|e| os_error(&e))?;
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Ok(Some(String::from_utf16_lossy(&buf[..len])))
}
#[cfg(test)]
mod tests {
    use super::*;

    const WHEA19: &str = r#"<Event xmlns="http://schemas.microsoft.com/win/2004/08/events/event"><System><Provider Name="Microsoft-Windows-WHEA-Logger" Guid="{c26c4f3c-3f17-4560-a3f8-dd5b19b8ae5a}"/><EventID>19</EventID><Version>0</Version><TimeCreated SystemTime="2026-10-06T10:11:12.1234567Z"/><EventRecordID>4242</EventRecordID></System><EventData><Data Name="ErrorSource">0</Data><Data Name="ApicId">0x1A</Data></EventData></Event>"#;
    const WHEA18: &str = r#"<Event><System><Provider Name='Microsoft-Windows-WHEA-Logger'/><EventID Qualifiers='0'>18</EventID><TimeCreated SystemTime='2026-10-06T10:00:00.0Z'/><EventRecordID>7</EventRecordID></System><EventData><Data Name='ErrorSource'>1</Data></EventData></Event>"#;
    const BUGCHECK: &str = r#"<Event><System><Provider Name="Microsoft-Windows-WER-SystemErrorReporting" Guid="{x}" EventSourceName="BugCheck"/><EventID Qualifiers="16384">1001</EventID><TimeCreated SystemTime="2026-10-06T09:00:00.0Z"/><EventRecordID>100</EventRecordID></System></Event>"#;
    const KPOWER: &str = r#"<Event><System><Provider Name="Microsoft-Windows-Kernel-Power" Guid="{y}"/><EventID>41</EventID><TimeCreated SystemTime="2026-10-06T08:00:00.0Z"/><EventRecordID>99</EventRecordID></System></Event>"#;

    #[test]
    fn parses_whea_19_with_apic_id() {
        let e = parse_event_xml(WHEA19).unwrap();
        assert_eq!(e.provider, EventProvider::Whea);
        assert_eq!((e.event_id, e.record_id, e.apic_id), (19, 4242, Some(26)));
        assert_eq!(e.time_utc, "2026-10-06T10:11:12.1234567Z");
        let w = e.whea().unwrap();
        assert_eq!((w.record_id, w.apic_id), (4242, Some(26)));
        let dec = WHEA19.replace("0x1A", "12");
        assert_eq!(parse_event_xml(&dec).unwrap().apic_id, Some(12));
    }

    #[test]
    fn parses_whea_18_without_apic() {
        let e = parse_event_xml(WHEA18).unwrap();
        assert_eq!((e.event_id, e.record_id, e.apic_id), (18, 7, None));
    }

    #[test]
    fn parses_bugcheck_1001_and_kernel_power_41() {
        let b = parse_event_xml(BUGCHECK).unwrap();
        assert_eq!(
            (b.provider, b.event_id, b.record_id),
            (EventProvider::BugCheck, 1001, 100)
        );
        assert!(b.whea().is_none());
        let k = parse_event_xml(KPOWER).unwrap();
        assert_eq!((k.provider, k.event_id), (EventProvider::KernelPower, 41));
    }

    #[test]
    fn single_and_double_quotes() {
        let spaced = WHEA19.replace("Name=\"ApicId\"", "Name = 'ApicId'");
        assert_eq!(parse_event_xml(&spaced).unwrap().apic_id, Some(26));
        let reordered = r#"<Event><System><Provider Guid="{g}" Name="Microsoft-Windows-Kernel-Power"/><EventID>41</EventID><TimeCreated SystemTime='t'/><EventRecordID>1</EventRecordID></System></Event>"#;
        assert_eq!(
            parse_event_xml(reordered).unwrap().provider,
            EventProvider::KernelPower
        );
    }

    #[test]
    fn garbage_is_none() {
        for g in ["", "<", "<Event", "<Provider Name=", "<Provider Name=\"", "not xml \u{1F600}", "<Event><System></System></Event>",
            "<Event><System><Provider Name=\"Other\"/><EventID>1</EventID><EventRecordID>1</EventRecordID></System></Event>",
            "<Provider Name=\"Microsoft-Windows-Kernel-Power\"/><EventID>x</EventID><EventRecordID>1</EventRecordID>"] {
            assert!(parse_event_xml(g).is_none(), "{g:?}");
        }
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn real_system_log_is_readable() {
        latest_record_id().unwrap();
    }
}
