//! Windows toasts for the rule alerts (spec §3.5), with the WinRT
//! `ToastNotificationManager` and no COM activator: a click reaches the app
//! through `Activated` only while it is running.

use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::core::{IInspectable, Interface, HSTRING};
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::{DateTime, IReference, PropertyValue, TypedEventHandler};
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows::UI::Notifications::{
    ToastActivatedEventArgs, ToastDismissalReason, ToastDismissedEventArgs, ToastFailedEventArgs,
    ToastNotification, ToastNotificationManager, ToastNotifier,
};

/// The identity the toasts are shown under: the AppUserModelID of the Start
/// menu shortcut the installer creates.
pub const AUMID: &str = "io.github.openmonitoradvanced";

/// Toasts waiting for the toast thread; more are dropped (and logged).
const QUEUE: usize = 16;
/// How long a toast stays in the notification center, clickable: its
/// handlers are kept that long at most.
const LIFETIME: Duration = Duration::from_secs(4 * 60 * 60);
/// Toasts whose handlers are kept at once; the oldest goes first.
const MAX_LIVE: usize = 32;
/// Seconds from 1601-01-01 (the WinRT `DateTime` epoch) to 1970-01-01.
const EPOCH_1601_S: i64 = 11_644_473_600;

/// Callback of a clicked toast, with its launch string.
type OnActivated = Arc<dyn Fn(String) + Send + Sync>;

enum Command {
    Show {
        title: String,
        body: String,
        launch: String,
    },
    /// A toast was activated, dismissed or failed: its handlers can go.
    Finished(u64),
}

/// The toast XML (`ToastGeneric`): a title line, a body line and the launch
/// string handed back on activation, all escaped for XML.
pub fn toast_xml(title: &str, body: &str, launch: &str) -> String {
    format!(
        "<toast launch=\"{}\"><visual><binding template=\"ToastGeneric\">\
         <text>{}</text><text>{}</text></binding></visual></toast>",
        escape(launch),
        escape(title),
        escape(body)
    )
}

/// XML-escapes `text`; control characters XML 1.0 cannot hold become spaces.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if c < ' ' || c == '\u{fffe}' || c == '\u{ffff}' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// `unix` (time since 1970) as a WinRT `DateTime`: 100 ns ticks since 1601.
fn winrt_ticks(unix: Duration) -> i64 {
    // Far beyond any real date: saturate instead of overflowing.
    let seconds = i64::try_from(unix.as_secs()).unwrap_or(i64::MAX / 20_000_000);
    (seconds + EPOCH_1601_S) * 10_000_000 + i64::from(unix.subsec_nanos() / 100)
}

/// Shows toasts from a thread of its own (`oma-toast`), so a caller never
/// waits on WinRT. The thread lives as long as the process: the handlers of
/// the toasts it shows hold a way back to it.
pub struct Toaster {
    tx: SyncSender<Command>,
}

impl Toaster {
    /// Starts the toast thread; `on_activated` runs on a WinRT thread with the
    /// launch string of a clicked toast.
    pub fn spawn(on_activated: Box<dyn Fn(String) + Send + Sync>) -> Self {
        let on_activated: OnActivated = Arc::from(on_activated);
        Self::start(QUEUE, move |rx, wake| run(&rx, &wake, &on_activated))
    }

    fn start(
        capacity: usize,
        worker: impl FnOnce(Receiver<Command>, SyncSender<Command>) + Send + 'static,
    ) -> Self {
        let (tx, rx) = sync_channel(capacity);
        let wake = tx.clone();
        // A thread that cannot start drops the receiver: `show` then logs.
        if let Err(err) = std::thread::Builder::new()
            .name("oma-toast".into())
            .spawn(move || worker(rx, wake))
        {
            tracing::error!(%err, "cannot start the toast thread");
        }
        Self { tx }
    }

    /// Queues a toast and returns at once; a full queue or a failure is only
    /// logged.
    pub fn show(&self, title: String, body: String, launch: String) {
        match self.tx.try_send(Command::Show {
            title,
            body,
            launch,
        }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => tracing::warn!("toast queue full: toast dropped"),
            Err(TrySendError::Disconnected(_)) => {
                tracing::warn!("toast thread not running: toast dropped");
            }
        }
    }
}

/// A shown toast and its handler tokens, kept until it is clicked, dismissed,
/// failed or expired.
struct Live {
    id: u64,
    toast: ToastNotification,
    activated: i64,
    dismissed: i64,
    failed: i64,
    expires: Instant,
}

impl Live {
    /// Removes the handlers; the toast object goes with `self`.
    fn release(self) {
        let _ = self.toast.RemoveActivated(self.activated);
        let _ = self.toast.RemoveDismissed(self.dismissed);
        let _ = self.toast.RemoveFailed(self.failed);
    }
}

fn run(rx: &Receiver<Command>, wake: &SyncSender<Command>, on_activated: &OnActivated) {
    // SAFETY: RoInitialize takes no pointers; this thread initializes WinRT
    // once, for itself, before any WinRT call, and keeps it for its lifetime.
    if let Err(err) = unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
        tracing::error!(%err, "WinRT unavailable: no toasts");
        return;
    }
    let mut notifier: Option<ToastNotifier> = None;
    let mut live: Vec<Live> = Vec::new();
    let mut next_id = 0u64;
    loop {
        // Idle without toasts to expire: no wake-ups.
        let command = match live.iter().map(|l| l.expires).min() {
            Some(expires) => rx.recv_timeout(expires.saturating_duration_since(Instant::now())),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match command {
            Ok(Command::Show {
                title,
                body,
                launch,
            }) => {
                next_id += 1;
                let xml = toast_xml(&title, &body, &launch);
                match show(&mut notifier, &xml, next_id, wake, on_activated) {
                    Ok(shown) => {
                        live.push(shown);
                        if live.len() > MAX_LIVE {
                            live.remove(0).release();
                        }
                    }
                    Err(err) => tracing::warn!(%err, "cannot show a toast"),
                }
            }
            Ok(Command::Finished(id)) => {
                if let Some(index) = live.iter().position(|l| l.id == id) {
                    live.remove(index).release();
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        let now = Instant::now();
        while let Some(index) = live.iter().position(|l| l.expires <= now) {
            live.remove(index).release();
        }
    }
    live.into_iter().for_each(Live::release);
}

fn show(
    notifier: &mut Option<ToastNotifier>,
    xml: &str,
    id: u64,
    wake: &SyncSender<Command>,
    on_activated: &OnActivated,
) -> windows::core::Result<Live> {
    // A notifier that could not be made is tried again with the next toast.
    let notifier = match notifier {
        Some(notifier) => notifier,
        None => notifier.insert(ToastNotificationManager::CreateToastNotifierWithId(
            &HSTRING::from(AUMID),
        )?),
    };
    let doc = XmlDocument::new()?;
    doc.LoadXml(&HSTRING::from(xml))?;
    let toast = ToastNotification::CreateToastNotification(&doc)?;
    if let Err(err) = expiration(LIFETIME).and_then(|at| toast.SetExpirationTime(&at)) {
        tracing::debug!(%err, "toast without an expiration time");
    }

    // A full queue loses the message: the toast is then released at expiry.
    let finished = move |wake: &SyncSender<Command>| {
        let _ = wake.try_send(Command::Finished(id));
    };
    let (click, done) = (on_activated.clone(), wake.clone());
    let activated = toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
        move |_, args| {
            let launch = args
                .as_ref()
                .and_then(|args| args.cast::<ToastActivatedEventArgs>().ok())
                .and_then(|args| args.Arguments().ok())
                .map(|launch| launch.to_string())
                .unwrap_or_default();
            click(launch);
            finished(&done);
            Ok(())
        },
    ))?;
    let done = wake.clone();
    let dismissed = toast.Dismissed(&TypedEventHandler::<
        ToastNotification,
        ToastDismissedEventArgs,
    >::new(move |_, args| {
        // A timed-out toast moved to the notification center: still clickable.
        let reason = args.as_ref().and_then(|args| args.Reason().ok());
        if reason != Some(ToastDismissalReason::TimedOut) {
            finished(&done);
        }
        Ok(())
    }))?;
    let done = wake.clone();
    let failed = toast.Failed(
        &TypedEventHandler::<ToastNotification, ToastFailedEventArgs>::new(move |_, args| {
            let code = args.as_ref().and_then(|args| args.ErrorCode().ok());
            tracing::warn!(?code, "a toast failed");
            finished(&done);
            Ok(())
        }),
    )?;
    let shown = Live {
        id,
        toast,
        activated,
        dismissed,
        failed,
        expires: Instant::now() + LIFETIME,
    };
    match notifier.Show(&shown.toast) {
        Ok(()) => Ok(shown),
        Err(err) => {
            shown.release();
            Err(err)
        }
    }
}

/// The WinRT time `lifetime` from now, boxed for `SetExpirationTime`.
fn expiration(lifetime: Duration) -> windows::core::Result<IReference<DateTime>> {
    let unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    PropertyValue::CreateDateTime(DateTime {
        UniversalTime: winrt_ticks(unix + lifetime),
    })?
    .cast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_xml_escapes_text() {
        let xml = toast_xml("CPU <hot> & \"loud\"", "it's 95 °C", "{\"device\":\"a&b\"}");
        assert_eq!(
            xml,
            "<toast launch=\"{&quot;device&quot;:&quot;a&amp;b&quot;}\">\
             <visual><binding template=\"ToastGeneric\">\
             <text>CPU &lt;hot&gt; &amp; &quot;loud&quot;</text>\
             <text>it&apos;s 95 °C</text>\
             </binding></visual></toast>"
        );
    }

    #[test]
    fn toast_xml_drops_characters_xml_cannot_hold() {
        let xml = toast_xml("a\u{1}b", "tab\there\nline", "x");
        assert!(xml.contains("<text>a b</text>"), "{xml}");
        assert!(xml.contains("<text>tab\there\nline</text>"), "{xml}");
    }

    /// The XML parser Windows uses for the toast gives back every text and the
    /// launch string unchanged (no toast is shown).
    #[test]
    fn toast_xml_loads_with_the_launch_intact() {
        let launch = "{\"device\":\"storage/<\\\"d&'>/é\"}";
        let doc = XmlDocument::new().unwrap();
        doc.LoadXml(&HSTRING::from(toast_xml("T & <t>", "b \"q\"", launch)))
            .unwrap();
        let root = doc.DocumentElement().unwrap();
        assert_eq!(
            root.GetAttribute(&HSTRING::from("launch")).unwrap(),
            HSTRING::from(launch)
        );
        let texts = doc.GetElementsByTagName(&HSTRING::from("text")).unwrap();
        assert_eq!(texts.Length().unwrap(), 2);
        assert_eq!(texts.Item(0).unwrap().InnerText().unwrap(), "T & <t>");
        assert_eq!(texts.Item(1).unwrap().InnerText().unwrap(), "b \"q\"");
    }

    #[test]
    fn winrt_ticks_count_from_1601() {
        assert_eq!(winrt_ticks(Duration::ZERO), EPOCH_1601_S * 10_000_000);
        assert_eq!(
            winrt_ticks(Duration::new(1, 250)),
            (EPOCH_1601_S + 1) * 10_000_000 + 2
        );
    }

    #[test]
    fn toast_queue_saturation_does_not_block_sampling() {
        // A toast thread stuck in WinRT: it takes nothing from the queue.
        let (release, stuck) = std::sync::mpsc::channel::<()>();
        let toaster = Toaster::start(4, move |rx, _wake| {
            let _ = stuck.recv();
            drop(rx);
        });
        let started = Instant::now();
        for i in 0..1_000 {
            toaster.show(format!("t{i}"), "b".into(), "{}".into());
        }
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "show waited on the toast thread: {:?}",
            started.elapsed()
        );
        let _ = release.send(());
    }

    #[test]
    fn a_failed_toast_thread_is_only_logged() {
        // The thread ended (e.g. WinRT could not start): show still returns.
        let toaster = Toaster::start(4, |rx, wake| drop((rx, wake)));
        std::thread::sleep(Duration::from_millis(50));
        toaster.show("t".into(), "b".into(), "{}".into());
    }

    #[test]
    #[ignore = "shows a real toast"]
    fn shows_a_toast() {
        let (tx, rx) = std::sync::mpsc::channel();
        let toaster = Toaster::spawn(Box::new(move |launch| {
            let _ = tx.send(launch);
        }));
        toaster.show(
            "OpenMonitor Advanced".into(),
            "Test toast: click it within 20 s".into(),
            "{\"device\":\"cpu/0\"}".into(),
        );
        let clicked = rx.recv_timeout(Duration::from_secs(20));
        eprintln!("activation: {clicked:?}");
    }
}
