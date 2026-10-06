//! The private pipe between the app and `oma-load.exe` (M8a1): the generic
//! [`crate::private_pipe`] with the load protocol and its name prefix.

use std::io;

use oma_ipc::load::{LoadMessage, LOAD_PIPE_PREFIX};

use crate::overlay_pipe::random_uuid_v4;
use crate::private_pipe::{connect_private_client, PrivateConnection, PrivatePipeServer};

/// The server end of the load pipe, owned by the app. Create it with
/// [`create_load_server`].
pub type LoadPipeServer = PrivatePipeServer;

/// One connected end of the load pipe (app or `oma-load`).
pub type LoadConnection = PrivateConnection<LoadMessage>;

/// A fresh load pipe name: [`LOAD_PIPE_PREFIX`] followed by a UUID v4.
pub fn random_load_pipe_name() -> io::Result<String> {
    Ok(format!("{LOAD_PIPE_PREFIX}{}", random_uuid_v4()?))
}

/// Creates the only instance of the load pipe `name`; see
/// [`PrivatePipeServer::create`].
pub fn create_load_server(name: &str) -> io::Result<LoadPipeServer> {
    PrivatePipeServer::create(LOAD_PIPE_PREFIX, name)
}

/// Opens the load pipe `name` (used by `oma-load`).
pub fn connect_load_client(name: &str) -> io::Result<LoadConnection> {
    connect_private_client(LOAD_PIPE_PREFIX, name)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;

    use oma_ipc::load::{LoadHello, LOAD_PROTOCOL_VERSION};

    use super::*;
    use crate::private_pipe::PipeEvent;

    fn hello() -> LoadMessage {
        LoadMessage::Hello(LoadHello {
            protocol_version: LOAD_PROTOCOL_VERSION,
            version: "test".to_owned(),
            isa: vec![],
        })
    }

    #[test]
    fn load_pipe_round_trip_and_pid() {
        let name = random_load_pipe_name().unwrap();
        let server = create_load_server(&name).expect("create the server");
        let client = connect_load_client(&name).expect("connect the client");
        assert_eq!(
            server.accept(Duration::from_secs(5)).expect("accept"),
            std::process::id()
        );
        let server: LoadConnection = server.into_connection();
        let (tx, rx) = sync_channel(4);
        let _reader = server.start_reader(move |e| tx.try_send(e).is_ok());
        client.send(&hello()).expect("send hello");
        match rx.recv_timeout(Duration::from_secs(5)).expect("event") {
            PipeEvent::Message(m) => assert_eq!(m, hello()),
            _ => panic!("expected a message"),
        }
    }

    #[test]
    fn second_load_server_with_same_name_fails() {
        let name = random_load_pipe_name().unwrap();
        let _first = create_load_server(&name).expect("first server");
        assert!(create_load_server(&name).is_err());
    }

    #[test]
    fn load_name_requires_the_prefix() {
        let err = create_load_server(r"\\.\pipe\OpenMonitorAdvanced-Overlay-x")
            .err()
            .expect("overlay name");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        let err = create_load_server(&format!("{LOAD_PIPE_PREFIX}a\\b"))
            .err()
            .expect("nested name");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        let err = connect_load_client(LOAD_PIPE_PREFIX)
            .err()
            .expect("empty suffix");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
