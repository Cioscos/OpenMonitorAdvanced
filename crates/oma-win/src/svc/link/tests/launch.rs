use super::*;

// ---- launch ----

#[test]
fn launch_starts_a_stopped_service_once() {
    let control = FakeControl::new(in_state(RunState::Stopped));
    let h = Harness::spawn(control, Script::with(vec![]), false);
    // No pipe: Starting during the grace period, then Unreachable.
    h.wait_for(is(disconnected()));
    cycles(5);
    assert_eq!(h.control.starts(), 1);
    assert!(h.script.connects() >= 3, "the link keeps reconnecting");
    assert_eq!(shows(&h.status()), shows(&disconnected()));
}

#[test]
fn not_installed_is_reported_and_never_started() {
    let control = FakeControl::new(ServiceQuery::NotInstalled);
    let h = Harness::spawn(control, Script::with(vec![]), false);
    h.wait_for(is(st(ServiceState::NotInstalled, None)));
    cycles(5);
    assert_eq!(h.control.starts(), 0);
    assert!(h.control.queries() >= 3, "the query repeats at every retry");
    assert_eq!(
        shows(&h.status()),
        shows(&st(ServiceState::NotInstalled, None))
    );
}

#[test]
fn access_denied_on_start_is_unreachable_with_detail() {
    let control = FakeControl::new(in_state(RunState::Stopped));
    control.with(|s| s.start_result = Err(ERROR_ACCESS_DENIED));
    let h = Harness::spawn(control, Script::with(vec![]), false);
    let denied = st(ServiceState::Unreachable, Some(ServiceDetail::AccessDenied));
    h.wait_for(is(denied.clone()));
    cycles(5);
    assert_eq!(h.control.starts(), 1);
    assert_eq!(
        shows(&h.status()),
        shows(&denied),
        "a missing pipe keeps the reason"
    );
}

#[test]
fn other_start_errors_are_start_failed() {
    let control = FakeControl::new(in_state(RunState::Stopped));
    control.with(|s| s.start_result = Err(1058));
    let h = Harness::spawn(control, Script::with(vec![]), false);
    h.wait_for(is(st(
        ServiceState::Unreachable,
        Some(ServiceDetail::StartFailed),
    )));
    assert_eq!(h.control.starts(), 1);
}

#[test]
fn reconnect_loop_never_starts_the_service() {
    let control = FakeControl::new(in_state(RunState::Stopped));
    let (conn, ctl) = streaming_conn(Some(PID));
    let h = Harness::spawn(control, Script::with(vec![conn]), false);
    h.wait_for(is(connected()));

    ctl.close();
    h.wait_for(is(disconnected()));
    let connects = h.script.connects();
    cycles(5);
    assert!(
        h.script.connects() >= connects + 3,
        "the link keeps retrying"
    );
    assert_eq!(h.control.starts(), 1, "only the launch started the service");
}

// ---- connection checks ----

#[test]
fn pid_mismatch_is_unreachable_not_connected() {
    let control = FakeControl::new(running());
    let (conn, ctl) = streaming_conn(Some(PID + 1));
    let h = Harness::spawn_with(control, Script::with(vec![conn]), false, slow_retry());
    h.wait_for(is(st(
        ServiceState::Unreachable,
        Some(ServiceDetail::PidMismatch),
    )));
    let view = h.feed.view();
    assert!(view.schema.is_none());
    assert!(view.snapshot.is_none());
    assert!(ctl.sent().is_empty(), "nothing is sent to an impostor");
    cycles(1);
    assert!(!ctl.is_alive(), "the connection is closed");
}

#[test]
fn missing_server_pid_is_rejected() {
    let control = FakeControl::new(running());
    let (conn, ctl) = streaming_conn(None);
    let h = Harness::spawn_with(control, Script::with(vec![conn]), false, slow_retry());
    h.wait_for(is(st(
        ServiceState::Unreachable,
        Some(ServiceDetail::PidMismatch),
    )));
    assert!(h.feed.view().schema.is_none());
    assert!(ctl.sent().is_empty());
}

#[test]
fn a_stopped_service_behind_the_pipe_is_a_pid_mismatch() {
    // Someone else serves the pipe while the service is stopped (pid 0).
    let control = FakeControl::new(in_state(RunState::StartPending));
    let (conn, _ctl) = streaming_conn(Some(0));
    let h = Harness::spawn_with(control, Script::with(vec![conn]), false, slow_retry());
    h.wait_for(is(st(
        ServiceState::Unreachable,
        Some(ServiceDetail::PidMismatch),
    )));
    assert!(h.feed.view().schema.is_none());
}

#[test]
fn protocol_version_mismatch_is_incompatible() {
    let control = FakeControl::new(running());
    let (conn, ctl) = fake_conn(Some(PID));
    ctl.push(hello(PROTOCOL_VERSION + 1));
    let h = Harness::spawn(control, Script::with(vec![conn]), false);
    let incompatible = st(ServiceState::Incompatible, None);
    h.wait_for(is(incompatible.clone()));
    assert!(ctl.sent().is_empty(), "no Subscribe after a foreign Hello");
    // The SCM is asked at every retry, but the service is not connected
    // again (each connection would reset its idle timer).
    let queries = h.control.queries();
    cycles(5);
    assert!(h.control.queries() >= queries + 3, "the SCM is still asked");
    assert_eq!(h.script.connects(), 1, "no reconnection by itself");
    assert_eq!(shows(&h.status()), shows(&incompatible));

    // "Avvia" tries again.
    h.send(LinkCommand::Start);
    cycles(2);
    assert!(h.script.connects() >= 2, "the command connects again");
    assert_eq!(h.control.starts(), 1, "started once, at the command");
}

#[test]
fn a_v1_service_hello_on_the_wire_is_incompatible_not_disconnected() {
    // The bytes a protocol v1 service sends (no `pawn_io`), decoded the way the pipe
    // reader does, must reach the version check.
    const V1_HELLO: [u8; 58] = [
        0x82, 0xa4, 0x74, 0x79, 0x70, 0x65, 0xa5, 0x68, 0x65, 0x6c, 0x6c, 0x6f, 0xa4, 0x62, 0x6f,
        0x64, 0x79, 0x82, 0xb0, 0x70, 0x72, 0x6f, 0x74, 0x6f, 0x63, 0x6f, 0x6c, 0x5f, 0x76, 0x65,
        0x72, 0x73, 0x69, 0x6f, 0x6e, 0x1, 0xaf, 0x73, 0x65, 0x72, 0x76, 0x69, 0x63, 0x65, 0x5f,
        0x76, 0x65, 0x72, 0x73, 0x69, 0x6f, 0x6e, 0xa5, 0x30, 0x2e, 0x31, 0x2e, 0x30,
    ];
    let old_hello = oma_ipc::decode_payload(&V1_HELLO).expect("a v1 hello decodes");
    let control = FakeControl::new(running());
    let (conn, ctl) = fake_conn(Some(PID));
    ctl.push(old_hello);
    let h = Harness::spawn(control, Script::with(vec![conn]), false);
    h.wait_for(is(st(ServiceState::Incompatible, None)));
    assert!(ctl.sent().is_empty(), "no Subscribe to a v1 service");
}

#[test]
fn incompatible_is_not_retried_until_start() {
    // Fake time: the rules alone, no clock.
    let t0 = Instant::now();
    let (mut m, _) = machine(false);
    m.decide(Event::Queried(running()), t0);
    m.decide(Event::Connected(Ok(Some(PID))), t0);
    m.decide(Event::Queried(running()), t0);
    assert_eq!(
        m.decide(Event::Message(hello(PROTOCOL_VERSION + 1)), t0),
        vec![Effect::Close, Effect::ClearFeed]
    );
    assert_eq!(m.status, st(ServiceState::Incompatible, None));

    // 20 s of retries with the same running service: the SCM is asked
    // each time, and no connection is made.
    let retry = m.settings.retry;
    let mut t = t0;
    while t < t0 + Duration::from_secs(20) {
        t += retry;
        assert_eq!(
            m.decide(Event::Timer, t),
            vec![Effect::Query],
            "at {:?}",
            t - t0
        );
        assert_eq!(m.decide(Event::Queried(running()), t), vec![]);
    }
    assert_eq!(m.status, st(ServiceState::Incompatible, None));

    // "Avvia": the service is started (a no-op if running) and connected once.
    assert_eq!(
        m.decide(Event::Command(LinkCommand::Start), t),
        vec![Effect::Start]
    );
    assert_eq!(m.decide(Event::Started(Ok(())), t), vec![Effect::Connect]);
}

#[test]
fn a_restarted_incompatible_service_is_connected_again() {
    let t0 = Instant::now();
    let (mut m, _) = machine(false);
    m.decide(Event::Queried(running()), t0);
    m.decide(Event::Connected(Ok(Some(PID))), t0);
    m.decide(Event::Queried(running()), t0);
    m.decide(Event::Message(hello(PROTOCOL_VERSION + 1)), t0);

    let retry = m.settings.retry;
    let t1 = t0 + retry;
    m.decide(Event::Timer, t1);
    assert_eq!(
        m.decide(Event::Queried(in_state(RunState::Stopped)), t1),
        vec![]
    );
    // Upgraded and started by someone else: a new process.
    let t2 = t1 + retry;
    assert_eq!(m.decide(Event::Timer, t2), vec![Effect::Query]);
    let upgraded = ServiceQuery::State {
        state: RunState::Running,
        pid: PID + 9,
    };
    assert_eq!(
        m.decide(Event::Queried(upgraded), t2),
        vec![Effect::Connect]
    );
}

#[test]
fn pid_mismatch_waits_for_an_scm_change() {
    let t0 = Instant::now();
    let (mut m, _) = machine(false);
    let pid1 = ServiceQuery::State {
        state: RunState::Running,
        pid: 1,
    };
    let pid2 = ServiceQuery::State {
        state: RunState::Running,
        pid: 2,
    };
    assert_eq!(m.decide(Event::Queried(pid1), t0), vec![Effect::Connect]);
    // Someone else serves the pipe.
    m.decide(Event::Connected(Ok(Some(99))), t0);
    assert_eq!(
        m.decide(Event::Queried(pid1), t0),
        vec![Effect::Close, Effect::ClearFeed]
    );
    assert_eq!(
        m.status,
        st(ServiceState::Unreachable, Some(ServiceDetail::PidMismatch))
    );

    let retry = m.settings.retry;
    let mut t = t0;
    let mut step = |m: &mut Machine, answer: ServiceQuery| {
        t += retry;
        assert_eq!(m.decide(Event::Timer, t), vec![Effect::Query]);
        m.decide(Event::Queried(answer), t)
    };
    // Unchanged, then stopped: still no connection.
    assert_eq!(step(&mut m, pid1), vec![]);
    assert_eq!(step(&mut m, pid1), vec![]);
    assert_eq!(step(&mut m, in_state(RunState::Stopped)), vec![]);
    assert_eq!(step(&mut m, in_state(RunState::Stopped)), vec![]);
    // A transient error says nothing.
    assert_eq!(step(&mut m, ServiceQuery::Error(1115)), vec![]);
    // Running again as a new process: one new connection.
    assert_eq!(step(&mut m, pid2), vec![Effect::Connect]);
    assert_eq!(m.phase, Phase::Connecting);
}

#[test]
fn a_pid_mismatch_is_retried_after_the_service_finishes_starting() {
    // The pipe answered while the SCM still said StartPending.
    let t0 = Instant::now();
    let (mut m, _) = machine(false);
    m.decide(Event::Queried(running()), t0);
    m.decide(Event::Connected(Ok(Some(PID))), t0);
    let pending = ServiceQuery::State {
        state: RunState::StartPending,
        pid: PID,
    };
    m.decide(Event::Queried(pending), t0);
    let t1 = t0 + m.settings.retry;
    m.decide(Event::Timer, t1);
    assert_eq!(
        m.decide(Event::Queried(running()), t1),
        vec![Effect::Connect]
    );
}

#[test]
fn a_threaded_pid_mismatch_reconnects_only_on_an_scm_change() {
    let control = FakeControl::new(ServiceQuery::State {
        state: RunState::Running,
        pid: 1,
    });
    let (impostor, _ctl) = streaming_conn(Some(77));
    let h = Harness::spawn(Arc::clone(&control), Script::with(vec![impostor]), false);
    h.wait_for(is(st(
        ServiceState::Unreachable,
        Some(ServiceDetail::PidMismatch),
    )));
    cycles(6);
    assert_eq!(h.script.connects(), 1, "no reconnection while unchanged");

    control.set_query(in_state(RunState::Stopped));
    cycles(4);
    assert_eq!(h.script.connects(), 1, "a stopped service is not connected");
    let (real, _ctl2) = streaming_conn(Some(2));
    h.script.add(real);
    control.set_query(ServiceQuery::State {
        state: RunState::Running,
        pid: 2,
    });
    h.wait_for(is(connected()));
    assert_eq!(h.script.connects(), 2);
}

#[test]
fn unsupported_version_error_is_incompatible() {
    let control = FakeControl::new(running());
    let (conn, ctl) = fake_conn(Some(PID));
    ctl.push(hello(PROTOCOL_VERSION));
    ctl.push(Message::Error(WireError {
        code: UNSUPPORTED_VERSION.to_owned(),
        message: "no".to_owned(),
    }));
    let h = Harness::spawn(control, Script::with(vec![conn]), false);
    h.wait_for(is(st(ServiceState::Incompatible, None)));
}
