use super::*;

// ---- anti-cheat mode ----

#[test]
fn anti_cheat_stops_and_keeps_the_service_stopped() {
    let control = FakeControl::new(running());
    let (conn, ctl) = streaming_conn(Some(PID));
    let h = Harness::spawn(control, Script::with(vec![conn]), false);
    h.wait_for(is(connected()));

    h.send(LinkCommand::SetAntiCheat(true));
    h.wait_for(is(st(ServiceState::AntiCheat, None)));
    assert!(h.feed.view().schema.is_none());
    let connects = h.script.connects();
    cycles(10);
    assert!(!ctl.is_alive(), "the connection is closed");
    assert_eq!(h.control.stops(), 1);
    assert_eq!(h.control.starts(), 0);
    assert_eq!(h.script.connects(), connects, "no connection attempts");
    assert_eq!(
        shows(&h.status()),
        shows(&st(ServiceState::AntiCheat, None))
    );
}

#[test]
fn anti_cheat_does_not_fight_other_clients() {
    let control = FakeControl::new(running());
    let h = Harness::spawn(Arc::clone(&control), Script::with(vec![]), true);
    h.wait_for(is(st(ServiceState::AntiCheat, None)));
    // Someone else starts the service again.
    control.set_query(running());
    cycles(10);
    assert_eq!(control.stops(), 1);
    assert_eq!(
        shows(&h.status()),
        shows(&st(ServiceState::AntiCheat, None))
    );
}

#[test]
fn anti_cheat_at_launch_stops_a_running_service() {
    let control = FakeControl::new(running());
    // Hold the stop in StopPending so the launch status can be observed.
    control.with(|s| s.after_stop = Some(in_state(RunState::StopPending)));
    let h = Harness::spawn(Arc::clone(&control), Script::with(vec![]), true);
    let stopping = st(ServiceState::AntiCheat, Some(ServiceDetail::Stopping));
    assert_eq!(
        shows(&h.status()),
        shows(&stopping),
        "set before the thread runs"
    );
    cycles(2);
    assert_eq!(shows(&h.status()), shows(&stopping));
    control.set_query(in_state(RunState::Stopped));
    h.wait_for(is(st(ServiceState::AntiCheat, None)));
    assert_eq!(control.stops(), 1);
    assert_eq!(control.starts(), 0);
    assert_eq!(h.script.connects(), 0);
}

#[test]
fn disabling_anti_cheat_starts_once() {
    let control = FakeControl::new(in_state(RunState::Stopped));
    let (conn, _ctl) = streaming_conn(Some(PID));
    let h = Harness::spawn(control, Script::with(vec![conn]), true);
    h.wait_for(is(st(ServiceState::AntiCheat, None)));
    assert_eq!(h.control.stops(), 0, "a stopped service needs no STOP");
    assert_eq!(h.control.starts(), 0);

    h.send(LinkCommand::SetAntiCheat(false));
    h.wait_for(is(connected()));
    h.send(LinkCommand::SetAntiCheat(false));
    cycles(3);
    assert_eq!(h.control.starts(), 1);
    assert_eq!(shows(&h.status()), shows(&connected()));
}

#[test]
fn start_command_is_ignored_in_anti_cheat_mode() {
    let control = FakeControl::new(in_state(RunState::Stopped));
    let h = Harness::spawn(control, Script::with(vec![]), true);
    h.wait_for(is(st(ServiceState::AntiCheat, None)));
    h.send(LinkCommand::Start);
    cycles(3);
    assert_eq!(h.control.starts(), 0);
    assert_eq!(h.script.connects(), 0);
    assert_eq!(
        shows(&h.status()),
        shows(&st(ServiceState::AntiCheat, None))
    );
}

#[test]
fn start_command_starts_once_and_connects() {
    let control = FakeControl::new(in_state(RunState::Stopped));
    control.with(|s| s.start_result = Err(1058));
    // No retry fires during the test: only the command can use the
    // scripted connection.
    let settings = LinkSettings {
        retry: Duration::from_secs(10),
        ..test_settings()
    };
    let h = Harness::spawn_with(Arc::clone(&control), Script::with(vec![]), false, settings);
    h.wait_for(is(st(
        ServiceState::Unreachable,
        Some(ServiceDetail::StartFailed),
    )));
    control.with(|s| s.start_result = Ok(()));
    let (conn, _ctl) = streaming_conn(Some(PID));
    h.script.add(conn);
    h.send(LinkCommand::Start);
    h.wait_for(is(connected()));
    h.send(LinkCommand::Start);
    cycles(2);
    assert_eq!(control.starts(), 2, "the launch start and the command");
}

#[test]
fn stop_is_confirmed_by_query() {
    let control = FakeControl::new(running());
    control.with(|s| s.after_stop = Some(running()));
    let h = Harness::spawn(Arc::clone(&control), Script::with(vec![]), true);
    cycles(3);
    // STOP was accepted but the service still runs: not confirmed yet.
    assert_eq!(control.stops(), 1);
    assert_eq!(
        h.status(),
        st(ServiceState::AntiCheat, Some(ServiceDetail::Stopping))
    );
    control.set_query(in_state(RunState::StopPending));
    cycles(2);
    assert_eq!(
        h.status(),
        st(ServiceState::AntiCheat, Some(ServiceDetail::Stopping))
    );
    control.set_query(in_state(RunState::Stopped));
    h.wait_for(is(st(ServiceState::AntiCheat, None)));
    assert_eq!(control.stops(), 1);
}

#[test]
fn start_pending_is_waited_before_stop() {
    let control = FakeControl::new(in_state(RunState::StartPending));
    let h = Harness::spawn(Arc::clone(&control), Script::with(vec![]), true);
    cycles(3);
    assert_eq!(control.stops(), 0, "no STOP while StartPending");
    assert_eq!(
        h.status(),
        st(ServiceState::AntiCheat, Some(ServiceDetail::Stopping))
    );
    control.set_query(running());
    h.wait_for(is(st(ServiceState::AntiCheat, None)));
    assert_eq!(control.stops(), 1);
}

#[test]
fn stop_timeout_keeps_preference_and_reports_failure() {
    let control = FakeControl::new(running());
    control.with(|s| s.after_stop = None); // accepted, never stops
    let settings = LinkSettings {
        stop_timeout: Duration::from_millis(100),
        ..test_settings()
    };
    let h = Harness::spawn_with(Arc::clone(&control), Script::with(vec![]), true, settings);
    let failed = st(ServiceState::AntiCheat, Some(ServiceDetail::StopFailed));
    h.wait_for(is(failed.clone()));
    assert_eq!(control.stops(), 1, "one STOP, not a loop");

    h.send(LinkCommand::Start);
    cycles(5);
    assert_eq!(control.starts(), 0);
    assert_eq!(h.script.connects(), 0);
    assert_eq!(control.stops(), 1);
    assert_eq!(shows(&h.status()), shows(&failed));
}

#[test]
fn a_refused_stop_is_stop_failed() {
    let control = FakeControl::new(running());
    control.with(|s| s.stop_result = Err(ERROR_ACCESS_DENIED));
    let h = Harness::spawn(Arc::clone(&control), Script::with(vec![]), true);
    h.wait_for(is(st(
        ServiceState::AntiCheat,
        Some(ServiceDetail::StopFailed),
    )));
    cycles(3);
    assert_eq!(control.stops(), 1);
}

#[test]
fn repeated_toggle_is_idempotent() {
    let control = FakeControl::new(running());
    let (conn, _ctl) = streaming_conn(Some(PID));
    let h = Harness::spawn(Arc::clone(&control), Script::with(vec![conn]), false);
    h.wait_for(is(connected()));

    h.send(LinkCommand::SetAntiCheat(true));
    h.send(LinkCommand::SetAntiCheat(true));
    h.wait_for(is(st(ServiceState::AntiCheat, None)));
    cycles(2);
    assert_eq!(control.stops(), 1);

    let (conn, _ctl) = streaming_conn(Some(PID));
    h.script.add(conn);
    h.send(LinkCommand::SetAntiCheat(false));
    h.send(LinkCommand::SetAntiCheat(false));
    h.wait_for(is(connected()));
    cycles(2);
    assert_eq!(control.starts(), 1);
    assert_eq!(control.stops(), 1);
}

// ---- fix round 1: launch rule R21 and fresh reasons ----

/// The service turns up stopped; the link reports it and never starts it.
fn assert_never_started(h: &Harness, expected: ServiceStatus) {
    h.control.set_query(in_state(RunState::Stopped));
    h.wait_for(is(expected.clone()));
    cycles(5);
    assert_eq!(h.control.starts(), 0);
    assert_eq!(shows(&h.status()), shows(&expected));
}

#[test]
fn a_service_being_stopped_at_launch_is_not_restarted() {
    let control = FakeControl::new(in_state(RunState::StopPending));
    let h = Harness::spawn(control, Script::with(vec![]), false);
    h.wait_for(is(disconnected()));
    assert_never_started(&h, disconnected());
}

#[test]
fn a_service_in_another_state_at_launch_is_not_started() {
    let control = FakeControl::new(in_state(RunState::Other(7)));
    let h = Harness::spawn(control, Script::with(vec![]), false);
    h.wait_for(is(disconnected()));
    assert_never_started(&h, disconnected());
}

#[test]
fn a_service_installed_after_launch_is_not_started_automatically() {
    let control = FakeControl::new(ServiceQuery::NotInstalled);
    let h = Harness::spawn(control, Script::with(vec![]), false);
    h.wait_for(is(st(ServiceState::NotInstalled, None)));
    assert_never_started(&h, disconnected());
}

#[test]
fn access_denied_at_launch_is_never_followed_by_a_start() {
    let control = FakeControl::new(ServiceQuery::AccessDenied);
    let h = Harness::spawn(control, Script::with(vec![]), false);
    let denied = st(ServiceState::Unreachable, Some(ServiceDetail::AccessDenied));
    h.wait_for(is(denied.clone()));
    assert_never_started(&h, denied.clone());
}

#[test]
fn transient_query_error_at_launch_is_retried_at_most_three_times() {
    let control = FakeControl::new(ServiceQuery::Error(1115));
    let h = Harness::spawn(control, Script::with(vec![]), false);
    h.wait_for(is(st(ServiceState::Unreachable, None)));
    assert!(h.control.queries() >= 4);
    assert_never_started(&h, disconnected());
}

#[test]
fn a_vanished_impostor_is_reported_as_disconnected() {
    let control = FakeControl::new(running());
    let (conn, _ctl) = streaming_conn(Some(PID + 1));
    let h = Harness::spawn_with(control, Script::with(vec![conn]), false, slow_retry());
    h.wait_for(is(st(
        ServiceState::Unreachable,
        Some(ServiceDetail::PidMismatch),
    )));
    // No automatic retry now: the user's "Avvia" tries again, and finds
    // no pipe (the SCM says why).
    cycles(1);
    h.send(LinkCommand::Start);
    wait_for(&h.status, is(disconnected()), Duration::from_secs(2));
}

#[test]
fn an_uninstalled_service_is_reported_as_not_installed() {
    let control = FakeControl::new(running());
    let (conn, ctl) = streaming_conn(Some(PID));
    let h = Harness::spawn(control, Script::with(vec![conn]), false);
    h.wait_for(is(connected()));
    h.control.set_query(ServiceQuery::NotInstalled);
    ctl.close();
    h.wait_for(is(st(ServiceState::NotInstalled, None)));
    assert_eq!(h.control.starts(), 0);
}

#[test]
fn disabling_anti_cheat_during_the_stop_waits_then_starts_once() {
    let control = FakeControl::new(running());
    control.with(|s| s.after_stop = Some(in_state(RunState::StopPending)));
    let (conn, _ctl) = streaming_conn(Some(PID));
    let h = Harness::spawn(Arc::clone(&control), Script::with(vec![conn]), true);
    let end = Instant::now() + WAIT;
    while control.stops() == 0 {
        assert!(Instant::now() < end, "STOP never sent");
        std::thread::sleep(Duration::from_millis(1));
    }

    h.send(LinkCommand::SetAntiCheat(false));
    h.wait_for(is(st(ServiceState::Starting, None)));
    cycles(3);
    assert_eq!(control.starts(), 0, "no start while the STOP is pending");
    assert_eq!(h.script.connects(), 0);

    control.set_query(in_state(RunState::Stopped));
    h.wait_for(is(connected()));
    assert_eq!(control.starts(), 1);
    assert_eq!(control.stops(), 1);
}
