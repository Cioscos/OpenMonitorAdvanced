//! Client side of the `oma-service` sensor service: the overlapped named-pipe
//! client ([`pipe`]), the service control wrapper over the SCM ([`scm`]), the
//! link thread that drives both ([`link`]), and what it publishes: the
//! service status ([`status`]) and the latest schema and snapshot ([`feed`]).
//! `drives` holds the rules that tell a service disk and a core disk apart.

pub(crate) mod drives;
#[cfg(test)]
mod fake_server;
pub mod feed;
pub mod link;
pub mod pipe;
pub mod provider;
pub mod scm;
pub mod status;

pub use feed::{FeedView, SourceRequest, SvcFeed};
pub use link::{
    pipe_connector, validate_schema, Connection, Connector, LinkBusy, LinkCommand, LinkSettings,
    ServiceLink, LINK_QUEUE_CAPACITY,
};
pub use pipe::{CloseReason, ConnectError, PipeClient, PipeEvent, PipeReader};
pub use provider::SvcProvider;
pub use scm::{RunState, ServiceControl, ServiceQuery, WindowsScm, SERVICE_NAME};
pub use status::{ServiceDetail, ServiceState, ServiceStatus, ServiceStatusTable};

/// The Win32 error code carried by a `windows` crate error (the HRESULT
/// itself when it is not a wrapped Win32 code).
pub(crate) fn win32_code(e: &windows::core::Error) -> u32 {
    windows::Win32::Foundation::WIN32_ERROR::from_error(e).map_or(e.code().0 as u32, |w| w.0)
}
