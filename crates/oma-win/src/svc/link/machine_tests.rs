use super::*;
use crate::svc::fake_server::FakeServer;
use crate::svc::link::tests::{
    connected, disconnected, hello, in_state, machine, running, schema, shows, snapshot, st,
    subscribe_request, subscribed, wire_schema, wire_snapshot, PID,
};
use crate::svc::link::{pipe_connector, Input, LinkSink};
use std::sync::mpsc;

#[test]
fn decide_launch_status_follows_the_preference() {
    let (m, _) = machine(false);
    assert_eq!(m.status, st(ServiceState::Starting, None));
    let (m, _) = machine(true);
    assert_eq!(
        m.status,
        st(ServiceState::AntiCheat, Some(ServiceDetail::Stopping))
    );
}

#[test]
fn decide_a_missing_pipe_is_starting_only_within_the_grace() {
    let (mut m, t0) = machine(false);
    assert_eq!(
        m.decide(Event::Queried(in_state(RunState::Stopped)), t0),
        vec![Effect::Start]
    );
    assert_eq!(m.decide(Event::Started(Ok(())), t0), vec![Effect::Connect]);
    let not_found = || Event::Connected(Err(ConnectError::NotFound));
    assert_eq!(m.decide(not_found(), t0), vec![]);
    assert_eq!(m.status, st(ServiceState::Starting, None));
    assert_eq!(m.deadline, Some(t0 + Duration::from_millis(20)));

    let t1 = t0 + Duration::from_millis(20);
    assert_eq!(m.decide(Event::Timer, t1), vec![Effect::Connect]);
    assert_eq!(m.decide(not_found(), t1), vec![]);
    assert_eq!(m.status, st(ServiceState::Starting, None));

    let t2 = t0 + Duration::from_millis(60);
    assert_eq!(m.decide(Event::Timer, t2), vec![Effect::Connect]);
    // Past the grace the reason comes from the SCM.
    assert_eq!(m.decide(not_found(), t2), vec![Effect::Query]);
    assert_eq!(m.decide(Event::Queried(running()), t2), vec![]);
    assert_eq!(m.status, disconnected());
}

#[test]
fn decide_start_pending_opens_the_grace() {
    let (mut m, t0) = machine(false);
    assert_eq!(
        m.decide(Event::Queried(in_state(RunState::StartPending)), t0),
        vec![Effect::Connect]
    );
    m.decide(Event::Connected(Err(ConnectError::NotFound)), t0);
    assert_eq!(m.status, st(ServiceState::Starting, None));
}

#[test]
fn decide_a_running_service_without_pipe_is_unreachable_at_once() {
    let (mut m, t0) = machine(false);
    assert_eq!(
        m.decide(Event::Queried(running()), t0),
        vec![Effect::Connect]
    );
    assert_eq!(
        m.decide(Event::Connected(Err(ConnectError::NotFound)), t0),
        vec![Effect::Query]
    );
    m.decide(Event::Queried(running()), t0);
    assert_eq!(m.status, disconnected());
}

#[test]
fn decide_pid_check_uses_the_fresh_query() {
    let (mut m, t0) = machine(false);
    m.decide(Event::Queried(running()), t0);
    m.decide(Event::Connected(Ok(Some(PID))), t0);
    // The service restarted between the launch query and this one.
    let restarted = ServiceQuery::State {
        state: RunState::Running,
        pid: PID + 7,
    };
    assert_eq!(
        m.decide(Event::Queried(restarted), t0),
        vec![Effect::Close, Effect::ClearFeed]
    );
    assert_eq!(
        m.status,
        st(ServiceState::Unreachable, Some(ServiceDetail::PidMismatch))
    );
    // Not a plain retry: the SCM must show a change before the next try.
    assert_eq!(
        m.phase,
        Phase::Held {
            baseline: Some(restarted)
        }
    );
}

#[test]
fn decide_snapshot_length_must_match_the_latest_schema() {
    let t0 = Instant::now();
    let mut m = subscribed(t0);
    assert_eq!(
        m.decide(Event::Message(schema(2)), t0),
        vec![Effect::SetSchema(wire_schema(2))]
    );
    assert_eq!(
        m.decide(Event::Message(snapshot(1, 2)), t0),
        vec![Effect::SetSnapshot(wire_snapshot(1, 2))]
    );
    assert_eq!(shows(&m.status), shows(&connected()));
    assert_eq!(m.deadline, Some(t0 + Duration::from_millis(3000)));

    m.decide(Event::Message(schema(3)), t0);
    // A snapshot sized for the old schema is a protocol violation.
    assert_eq!(
        m.decide(Event::Message(snapshot(2, 2)), t0),
        vec![Effect::Close, Effect::ClearFeed]
    );
    assert_eq!(m.status, disconnected());
}

#[test]
fn decide_first_sample_and_silence_deadlines() {
    let t0 = Instant::now();
    let mut m = subscribed(t0);
    assert_eq!(m.deadline, Some(t0 + Duration::from_secs(30)));
    assert_eq!(
        m.decide(Event::Timer, t0 + Duration::from_secs(30)),
        vec![Effect::Close, Effect::ClearFeed]
    );
    assert_eq!(m.status, disconnected());

    let mut m = subscribed(t0);
    m.decide(Event::Message(schema(1)), t0);
    m.decide(Event::Message(snapshot(1, 1)), t0);
    let t1 = t0 + Duration::from_millis(2500);
    m.decide(Event::Message(snapshot(2, 1)), t1);
    assert_eq!(m.deadline, Some(t1 + Duration::from_millis(3000)));
}

#[test]
fn decide_hello_timeout_and_unexpected_messages_close() {
    let (mut m, t0) = machine(false);
    m.decide(Event::Queried(running()), t0);
    m.decide(Event::Connected(Ok(Some(PID))), t0);
    m.decide(Event::Queried(running()), t0);
    assert_eq!(m.deadline, Some(t0 + Duration::from_secs(2)));
    assert_eq!(
        m.decide(Event::Timer, t0 + Duration::from_secs(2)),
        vec![Effect::Close, Effect::ClearFeed]
    );

    let mut m = subscribed(t0);
    assert_eq!(
        m.decide(Event::Message(hello(PROTOCOL_VERSION)), t0),
        vec![Effect::Close, Effect::ClearFeed]
    );
    assert_eq!(m.status, disconnected());
}

#[test]
fn decide_incompatible_is_kept_across_an_explicit_start() {
    let (mut m, t0) = machine(false);
    m.decide(Event::Queried(running()), t0);
    m.decide(Event::Connected(Ok(Some(PID))), t0);
    m.decide(Event::Queried(running()), t0);
    m.decide(Event::Message(hello(PROTOCOL_VERSION + 1)), t0);
    assert_eq!(m.status, st(ServiceState::Incompatible, None));

    let t1 = t0 + Duration::from_millis(20);
    assert_eq!(
        m.decide(Event::Command(LinkCommand::Start), t1),
        vec![Effect::Start]
    );
    assert_eq!(m.decide(Event::Started(Ok(())), t1), vec![Effect::Connect]);
    assert_eq!(
        m.decide(Event::Connected(Ok(Some(PID))), t1),
        vec![Effect::Query]
    );
    m.decide(Event::Queried(running()), t1);
    // Still the same service: the state is reported again, and held again.
    assert_eq!(
        m.decide(Event::Message(hello(PROTOCOL_VERSION + 1)), t1),
        vec![Effect::Close, Effect::ClearFeed]
    );
    assert_eq!(m.status, st(ServiceState::Incompatible, None));
    assert!(matches!(m.phase, Phase::Held { .. }));
}

#[test]
fn decide_stop_waits_for_a_stoppable_state() {
    let (mut m, t0) = machine(true);
    assert_eq!(
        m.decide(Event::Queried(in_state(RunState::StartPending)), t0),
        vec![]
    );
    assert_eq!(m.deadline, Some(t0 + Duration::from_millis(5)));
    let t1 = t0 + Duration::from_millis(5);
    assert_eq!(m.decide(Event::Timer, t1), vec![Effect::Query]);
    assert_eq!(m.decide(Event::Queried(running()), t1), vec![Effect::Stop]);
    // 1061: the service cannot take the control yet; query and retry.
    assert_eq!(
        m.decide(Event::StopSent(Err(ERROR_SERVICE_CANNOT_ACCEPT_CTRL)), t1),
        vec![]
    );
    let t2 = t1 + Duration::from_millis(5);
    assert_eq!(m.decide(Event::Timer, t2), vec![Effect::Query]);
    assert_eq!(m.decide(Event::Queried(running()), t2), vec![Effect::Stop]);
    assert_eq!(m.decide(Event::StopSent(Ok(())), t2), vec![]);
    let t3 = t2 + Duration::from_millis(5);
    assert_eq!(m.decide(Event::Timer, t3), vec![Effect::Query]);
    // Accepted: no second STOP while it is still running.
    assert_eq!(m.decide(Event::Queried(running()), t3), vec![]);
    assert_eq!(
        m.status,
        st(ServiceState::AntiCheat, Some(ServiceDetail::Stopping))
    );
    m.decide(Event::Timer, t3 + Duration::from_millis(5));
    m.decide(
        Event::Queried(in_state(RunState::Stopped)),
        t3 + Duration::from_millis(5),
    );
    assert_eq!(m.status, st(ServiceState::AntiCheat, None));
    assert_eq!(m.deadline, None);
}

#[test]
fn decide_stop_gives_up_after_the_timeout() {
    let (mut m, t0) = machine(true);
    m.decide(Event::Queried(in_state(RunState::StopPending)), t0);
    let late = t0 + m.settings.stop_timeout;
    assert_eq!(m.decide(Event::Timer, late), vec![]);
    assert_eq!(
        m.status,
        st(ServiceState::AntiCheat, Some(ServiceDetail::StopFailed))
    );
    assert_eq!(m.phase, Phase::AntiCheatIdle);
    assert_eq!(m.decide(Event::Command(LinkCommand::Start), late), vec![]);
}

#[test]
fn decide_start_is_ignored_while_connected() {
    let t0 = Instant::now();
    let mut m = subscribed(t0);
    assert_eq!(m.decide(Event::Command(LinkCommand::Start), t0), vec![]);
    assert_eq!(
        m.decide(Event::Command(LinkCommand::SetAntiCheat(false)), t0),
        vec![]
    );
    assert_eq!(
        m.decide(Event::Command(LinkCommand::SetAntiCheat(true)), t0),
        vec![Effect::Close, Effect::ClearFeed, Effect::Query]
    );
    assert_eq!(
        m.decide(Event::Command(LinkCommand::SetAntiCheat(true)), t0),
        vec![]
    );
}

#[test]
fn decide_start_errors() {
    for (code, status) in [
        (
            ERROR_ACCESS_DENIED,
            st(ServiceState::Unreachable, Some(ServiceDetail::AccessDenied)),
        ),
        (
            ERROR_SERVICE_DOES_NOT_EXIST,
            st(ServiceState::NotInstalled, None),
        ),
        (
            1058,
            st(ServiceState::Unreachable, Some(ServiceDetail::StartFailed)),
        ),
    ] {
        let (mut m, t0) = machine(false);
        m.decide(Event::Queried(in_state(RunState::Stopped)), t0);
        assert_eq!(m.decide(Event::Started(Err(code)), t0), vec![]);
        assert_eq!(m.status, status, "start error {code}");
    }
}

#[test]
fn decide_launch_starts_only_when_the_first_answer_is_stopped() {
    let denied = st(ServiceState::Unreachable, Some(ServiceDetail::AccessDenied));
    for (first, status, later) in [
        (
            ServiceQuery::NotInstalled,
            st(ServiceState::NotInstalled, None),
            disconnected(),
        ),
        (ServiceQuery::AccessDenied, denied.clone(), denied.clone()),
        (
            in_state(RunState::StopPending),
            disconnected(),
            disconnected(),
        ),
        (in_state(RunState::Other(7)), disconnected(), disconnected()),
    ] {
        let (mut m, t0) = machine(false);
        assert_eq!(m.decide(Event::Queried(first), t0), vec![], "{first:?}");
        assert_eq!(m.status, status, "{first:?}");
        // Later a stopped service is reported, never started.
        let t1 = t0 + Duration::from_millis(20);
        assert_eq!(m.decide(Event::Timer, t1), vec![Effect::Connect]);
        assert_eq!(
            m.decide(Event::Connected(Err(ConnectError::NotFound)), t1),
            vec![Effect::Query]
        );
        assert_eq!(
            m.decide(Event::Queried(in_state(RunState::Stopped)), t1),
            vec![]
        );
        assert_eq!(m.status, later, "{first:?} then Stopped");
    }
}

#[test]
fn decide_launch_error_is_retried_three_times_then_conclusive() {
    let (mut m, t0) = machine(false);
    let error = || Event::Queried(ServiceQuery::Error(1115));
    let mut t = t0;
    for _ in 0..3 {
        assert_eq!(m.decide(error(), t), vec![]);
        assert_eq!(m.status, st(ServiceState::Starting, None));
        t += Duration::from_millis(20);
        assert_eq!(m.decide(Event::Timer, t), vec![Effect::Query]);
    }
    assert_eq!(m.decide(error(), t), vec![]);
    assert_eq!(m.status, st(ServiceState::Unreachable, None));
    t += Duration::from_millis(20);
    // The connect loop now, not the probe: a stopped service stays stopped.
    assert_eq!(m.decide(Event::Timer, t), vec![Effect::Connect]);
    assert_eq!(
        m.decide(Event::Connected(Err(ConnectError::NotFound)), t),
        vec![Effect::Query]
    );
    assert_eq!(
        m.decide(Event::Queried(in_state(RunState::Stopped)), t),
        vec![]
    );
    assert_eq!(m.status, disconnected());
}

#[test]
fn decide_stopped_after_transient_errors_is_the_first_answer() {
    let (mut m, t0) = machine(false);
    m.decide(Event::Queried(ServiceQuery::Error(1115)), t0);
    let t1 = t0 + Duration::from_millis(20);
    assert_eq!(m.decide(Event::Timer, t1), vec![Effect::Query]);
    assert_eq!(
        m.decide(Event::Queried(in_state(RunState::Stopped)), t1),
        vec![Effect::Start]
    );
}

#[test]
fn decide_explicit_starts_never_reenter_the_launch_probe() {
    // A Start command on a service that turns out not installed.
    let (mut m, t0) = machine(false);
    m.decide(Event::Queried(ServiceQuery::NotInstalled), t0);
    let t1 = t0 + Duration::from_millis(5);
    assert_eq!(
        m.decide(Event::Command(LinkCommand::Start), t1),
        vec![Effect::Start]
    );
    assert_eq!(
        m.decide(Event::Started(Err(ERROR_SERVICE_DOES_NOT_EXIST)), t1),
        vec![]
    );
    assert_eq!(m.status, st(ServiceState::NotInstalled, None));
    let t2 = t1 + Duration::from_millis(20);
    assert_eq!(m.decide(Event::Timer, t2), vec![Effect::Connect]);
    assert_eq!(
        m.decide(Event::Connected(Err(ConnectError::NotFound)), t2),
        vec![Effect::Query]
    );
    assert_eq!(
        m.decide(Event::Queried(in_state(RunState::Stopped)), t2),
        vec![]
    );
}

#[test]
fn decide_connect_loop_refreshes_the_reason_from_the_scm() {
    let (mut m, t0) = machine(false);
    m.decide(Event::Queried(running()), t0);
    m.decide(Event::Connected(Err(ConnectError::NotFound)), t0);
    m.decide(Event::Queried(running()), t0);
    let denied = st(ServiceState::Unreachable, Some(ServiceDetail::AccessDenied));
    assert_eq!(m.status, disconnected());
    let mut t = t0;
    for (answer, expected) in [
        (in_state(RunState::Stopped), disconnected()),
        (
            ServiceQuery::NotInstalled,
            st(ServiceState::NotInstalled, None),
        ),
        (ServiceQuery::AccessDenied, denied.clone()),
        // A transient error says nothing new.
        (ServiceQuery::Error(1115), denied.clone()),
        (running(), disconnected()),
        (in_state(RunState::StopPending), disconnected()),
        (
            in_state(RunState::StartPending),
            st(ServiceState::Starting, None),
        ),
    ] {
        t += Duration::from_millis(20);
        assert_eq!(m.decide(Event::Timer, t), vec![Effect::Connect]);
        assert_eq!(
            m.decide(Event::Connected(Err(ConnectError::NotFound)), t),
            vec![Effect::Query]
        );
        assert_eq!(m.decide(Event::Queried(answer), t), vec![], "{answer:?}");
        assert_eq!(m.status, expected, "{answer:?}");
    }
    // StartPending opened a grace: no query while it lasts.
    t += Duration::from_millis(20);
    m.decide(Event::Timer, t);
    assert_eq!(
        m.decide(Event::Connected(Err(ConnectError::NotFound)), t),
        vec![]
    );
    assert_eq!(m.status, st(ServiceState::Starting, None));
}

#[test]
fn decide_a_refused_pipe_on_a_running_service_is_access_denied() {
    let (mut m, t0) = machine(false);
    m.decide(Event::Queried(running()), t0);
    assert_eq!(
        m.decide(Event::Connected(Err(ConnectError::AccessDenied)), t0),
        vec![Effect::Query]
    );
    m.decide(Event::Queried(running()), t0);
    assert_eq!(
        m.status,
        st(ServiceState::Unreachable, Some(ServiceDetail::AccessDenied))
    );
}

#[test]
fn decide_grace_ends_when_a_connection_is_accepted_or_closed() {
    let (mut m, t0) = machine(false);
    m.decide(Event::Queried(in_state(RunState::Stopped)), t0);
    m.decide(Event::Started(Ok(())), t0);
    assert!(m.grace_until.is_some());
    m.decide(Event::Connected(Ok(Some(PID))), t0);
    m.decide(Event::Queried(running()), t0);
    assert_eq!(m.grace_until, None, "accepted");
    m.decide(Event::Closed(CloseReason::Disconnected), t0);
    // Well within the old grace, a missing pipe is no longer "starting".
    let t1 = t0 + Duration::from_millis(20);
    m.decide(Event::Timer, t1);
    assert_eq!(
        m.decide(Event::Connected(Err(ConnectError::NotFound)), t1),
        vec![Effect::Query]
    );

    // Closed before being accepted: an impostor right after our start().
    let (mut m, t0) = machine(false);
    m.decide(Event::Queried(in_state(RunState::Stopped)), t0);
    m.decide(Event::Started(Ok(())), t0);
    m.decide(Event::Connected(Ok(Some(PID + 1))), t0);
    m.decide(Event::Queried(running()), t0);
    assert_eq!(m.grace_until, None, "closed");
}

#[test]
fn decide_query_error_during_verification_is_disconnected() {
    let (mut m, t0) = machine(false);
    m.decide(Event::Queried(running()), t0);
    m.decide(Event::Connected(Ok(Some(PID))), t0);
    assert_eq!(
        m.decide(Event::Queried(ServiceQuery::Error(1115)), t0),
        vec![Effect::Close, Effect::ClearFeed]
    );
    assert_eq!(m.status, disconnected());
}

#[test]
fn decide_a_service_gone_during_verification_is_not_installed() {
    let (mut m, t0) = machine(false);
    m.decide(Event::Queried(running()), t0);
    m.decide(Event::Connected(Ok(Some(PID))), t0);
    assert_eq!(
        m.decide(Event::Queried(ServiceQuery::NotInstalled), t0),
        vec![Effect::Close, Effect::ClearFeed]
    );
    assert_eq!(m.status, st(ServiceState::NotInstalled, None));
}

#[test]
fn decide_disabling_anti_cheat_waits_for_a_pending_stop() {
    let (mut m, t0) = machine(true);
    assert_eq!(m.decide(Event::Queried(running()), t0), vec![Effect::Stop]);
    assert_eq!(m.decide(Event::StopSent(Ok(())), t0), vec![]);
    assert_eq!(
        m.decide(Event::Command(LinkCommand::SetAntiCheat(false)), t0),
        vec![]
    );
    assert_eq!(m.status, st(ServiceState::Starting, None));
    // A Start command changes nothing: the start is already due.
    assert_eq!(m.decide(Event::Command(LinkCommand::Start), t0), vec![]);
    let t1 = t0 + Duration::from_millis(5);
    assert_eq!(m.decide(Event::Timer, t1), vec![Effect::Query]);
    assert_eq!(
        m.decide(Event::Queried(in_state(RunState::StopPending)), t1),
        vec![]
    );
    let t2 = t1 + Duration::from_millis(5);
    assert_eq!(m.decide(Event::Timer, t2), vec![Effect::Query]);
    assert_eq!(
        m.decide(Event::Queried(in_state(RunState::Stopped)), t2),
        vec![Effect::Start]
    );
    assert_eq!(m.decide(Event::Started(Ok(())), t2), vec![Effect::Connect]);
}

#[test]
fn decide_a_stop_timeout_after_disabling_still_starts_once() {
    let (mut m, t0) = machine(true);
    m.decide(Event::Queried(running()), t0);
    m.decide(Event::StopSent(Ok(())), t0);
    m.decide(Event::Command(LinkCommand::SetAntiCheat(false)), t0);
    let late = t0 + m.settings.stop_timeout;
    assert_eq!(m.decide(Event::Timer, late), vec![Effect::Start]);
}

#[test]
fn decide_disabling_anti_cheat_before_any_stop_starts_at_once() {
    let (mut m, t0) = machine(true);
    // Still StartPending: no STOP has gone out, nothing to wait for.
    m.decide(Event::Queried(in_state(RunState::StartPending)), t0);
    assert_eq!(
        m.decide(Event::Command(LinkCommand::SetAntiCheat(false)), t0),
        vec![Effect::Start]
    );
}

#[test]
fn decide_reenabling_anti_cheat_cancels_the_pending_start() {
    let (mut m, t0) = machine(true);
    m.decide(Event::Queried(running()), t0);
    m.decide(Event::StopSent(Ok(())), t0);
    m.decide(Event::Command(LinkCommand::SetAntiCheat(false)), t0);
    assert_eq!(
        m.decide(Event::Command(LinkCommand::SetAntiCheat(true)), t0),
        vec![]
    );
    assert_eq!(
        m.status,
        st(ServiceState::AntiCheat, Some(ServiceDetail::Stopping))
    );
    let t1 = t0 + Duration::from_millis(5);
    assert_eq!(m.decide(Event::Timer, t1), vec![Effect::Query]);
    assert_eq!(
        m.decide(Event::Queried(in_state(RunState::Stopped)), t1),
        vec![]
    );
    assert_eq!(m.status, st(ServiceState::AntiCheat, None));
}

// ---- the real pipe ----

#[test]
fn pipe_connector_reads_and_writes_the_real_pipe() {
    let server = FakeServer::new();
    let connect = pipe_connector();
    let (tx, inbox) = mpsc::channel();
    let mut conn = connect(&server.name, LinkSink::new(3, tx)).expect("connect");
    server.accept();
    assert_eq!(conn.server_pid(), Some(std::process::id()));

    server.send(&hello(PROTOCOL_VERSION));
    match inbox.recv_timeout(Duration::from_secs(5)).expect("open") {
        Input::Message(3, msg) => assert_eq!(msg, hello(PROTOCOL_VERSION)),
        _ => panic!("expected the Hello of connection 3"),
    }
    assert!(inbox.recv_timeout(Duration::from_millis(10)).is_err());

    let subscribe = Message::Subscribe(subscribe_request(1000));
    conn.send(&subscribe).expect("send");
    assert_eq!(server.recv(), subscribe);

    server.disconnect();
    match inbox
        .recv_timeout(Duration::from_secs(5))
        .expect("the close")
    {
        Input::Closed(3, _) => {}
        _ => panic!("expected the close of connection 3"),
    }
}
