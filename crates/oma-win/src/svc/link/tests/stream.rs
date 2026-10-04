use super::*;

// ---- the stream ----

#[test]
fn schema_and_snapshots_reach_the_feed_and_status_is_connected() {
    let control = FakeControl::new(running());
    let (conn, ctl) = streaming_conn(Some(PID));
    let h = Harness::spawn(control, Script::with(vec![conn]), false);
    h.wait_for(is(connected()));

    let view = h.feed.view();
    assert_eq!(view.schema.as_deref(), Some(&wire_schema(2)));
    assert_eq!(view.snapshot.map(|(_, s)| s), Some(wire_snapshot(1, 2)));
    assert_eq!(view.interval, Duration::from_millis(1000));
    assert_eq!(
        ctl.sent(),
        vec![Message::Subscribe(subscribe_request(1000))]
    );
    assert_eq!(h.control.starts(), 0, "a running service is not started");

    ctl.push(snapshot(2, 2));
    let end = Instant::now() + WAIT;
    while h.feed.view().snapshot.map(|(_, s)| s.seq) != Some(2) {
        assert!(Instant::now() < end, "the second snapshot never arrived");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Waits until the connection has received `count` messages.
pub(super) fn wait_for_sent(ctl: &ConnCtl, count: usize) -> Vec<Message> {
    let end = Instant::now() + WAIT;
    loop {
        let sent = ctl.sent();
        if sent.len() >= count {
            return sent;
        }
        assert!(Instant::now() < end, "only {} messages sent", sent.len());
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn set_interval_resubscribes_when_connected() {
    let control = FakeControl::new(running());
    let (conn, ctl) = streaming_conn(Some(PID));
    let h = Harness::spawn(control, Script::with(vec![conn]), false);
    h.wait_for(is(connected()));

    h.send(LinkCommand::SetInterval(2000));
    let sent = wait_for_sent(&ctl, 2);
    assert_eq!(
        sent,
        vec![
            Message::Subscribe(subscribe_request(1000)),
            Message::Subscribe(subscribe_request(2000)),
        ]
    );
    let end = Instant::now() + WAIT;
    while h.feed.view().interval != Duration::from_millis(2000) {
        assert!(Instant::now() < end, "the feed interval never changed");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        shows(&h.status()),
        shows(&connected()),
        "the connection is kept"
    );
    assert_eq!(h.script.connects(), 1);

    // The same interval again is not sent twice.
    h.send(LinkCommand::SetInterval(2000));
    cycles(2);
    assert_eq!(ctl.sent().len(), 2);
}

#[test]
fn a_resubscribe_does_not_make_the_provider_rediscover() {
    let control = FakeControl::new(running());
    let (conn, ctl) = streaming_conn(Some(PID));
    let h = Harness::spawn(control, Script::with(vec![conn]), false);
    h.wait_for(is(connected()));
    let generation = h.feed.view().generation;

    h.send(LinkCommand::SetInterval(2000));
    wait_for_sent(&ctl, 2);
    // The service answers with its schema again, then a snapshot.
    ctl.push(schema(2));
    ctl.push(snapshot(2, 2));
    let end = Instant::now() + WAIT;
    while h.feed.view().snapshot.map(|(_, s)| s.seq) != Some(2) {
        assert!(Instant::now() < end, "the second snapshot never arrived");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        h.feed.view().generation,
        generation,
        "the same schema must not invalidate what the provider bound"
    );
}

#[test]
fn set_interval_while_disconnected_is_used_on_connect() {
    let control = FakeControl::new(running());
    let h = Harness::spawn(control, Script::with(vec![]), false);
    // No pipe yet: the link keeps retrying.
    h.wait_for(is(disconnected()));
    h.send(LinkCommand::SetInterval(2000));
    cycles(2);
    let (conn, ctl) = streaming_conn(Some(PID));
    h.script.add(conn);
    h.wait_for(is(connected()));
    assert_eq!(
        ctl.sent(),
        vec![Message::Subscribe(subscribe_request(2000))]
    );
    assert_eq!(h.feed.view().interval, Duration::from_millis(2000));
}

#[test]
fn the_silence_limit_follows_the_new_interval() {
    let control = FakeControl::new(running());
    let (conn, _ctl) = streaming_conn(Some(PID));
    let h = Harness::spawn(control, Script::with(vec![conn]), false);
    h.wait_for(is(connected()));
    // With 1000 ms the link would wait 3 s (much longer than WAIT); with
    // 40 ms it gives up after 120 ms.
    h.send(LinkCommand::SetInterval(40));
    h.wait_for(is(disconnected()));
}

#[test]
fn set_interval_moves_the_silence_deadline_of_a_streaming_link() {
    let now = Instant::now();
    let mut machine = Machine::new(LinkSettings::new("p", 1000), false);
    machine.phase = Phase::Streaming { schema_len: 2 };
    machine.deadline = Some(now + Duration::from_secs(3));

    let effects = machine.decide(Event::Command(LinkCommand::SetInterval(5000)), now);
    assert_eq!(
        effects,
        vec![
            Effect::SetInterval(Duration::from_millis(5000)),
            Effect::Send(Message::Subscribe(subscribe_request(5000))),
        ]
    );
    assert_eq!(machine.deadline, Some(now + Duration::from_secs(15)));
    assert_eq!(machine.phase, Phase::Streaming { schema_len: 2 });

    // Same value: nothing to do.
    assert!(machine
        .decide(Event::Command(LinkCommand::SetInterval(5000)), now)
        .is_empty());
}

#[test]
fn set_interval_before_the_first_sample_keeps_the_first_sample_deadline() {
    let now = Instant::now();
    let mut machine = Machine::new(LinkSettings::new("p", 1000), false);
    let deadline = now + Duration::from_secs(30);
    machine.phase = Phase::FirstSample { schema_len: None };
    machine.deadline = Some(deadline);
    let effects = machine.decide(Event::Command(LinkCommand::SetInterval(500)), now);
    assert_eq!(
        effects,
        vec![
            Effect::SetInterval(Duration::from_millis(500)),
            Effect::Send(Message::Subscribe(subscribe_request(500))),
        ]
    );
    assert_eq!(machine.deadline, Some(deadline));
}

#[test]
fn a_failed_resubscribe_disconnects() {
    let now = Instant::now();
    let mut machine = Machine::new(LinkSettings::new("p", 1000), false);
    machine.phase = Phase::Streaming { schema_len: 2 };
    machine.deadline = Some(now + Duration::from_secs(3));
    assert!(machine.decide(Event::Sent(true), now).is_empty());
    assert_eq!(machine.phase, Phase::Streaming { schema_len: 2 });
    let effects = machine.decide(Event::Sent(false), now);
    assert_eq!(effects, vec![Effect::Close, Effect::ClearFeed]);
    assert_eq!(machine.phase, Phase::ConnectWait);
}

#[test]
fn set_interval_in_the_other_phases_only_stores_the_value() {
    let now = Instant::now();
    for phase in [
        Phase::ConnectWait,
        Phase::Hello,
        Phase::AntiCheatIdle,
        Phase::Probing { errors: 0 },
    ] {
        let mut machine = Machine::new(LinkSettings::new("p", 1000), false);
        machine.phase = phase;
        let effects = machine.decide(Event::Command(LinkCommand::SetInterval(3000)), now);
        assert!(effects.is_empty(), "{phase:?}");
        assert_eq!(machine.settings.interval_ms, 3000, "{phase:?}");
    }
}

#[test]
fn a_new_schema_on_the_stream_replaces_the_old_one() {
    let control = FakeControl::new(running());
    let (conn, ctl) = streaming_conn(Some(PID));
    let h = Harness::spawn(control, Script::with(vec![conn]), false);
    h.wait_for(is(connected()));
    let generation = h.feed.view().generation;

    ctl.push(schema(3));
    ctl.push(snapshot(2, 3));
    let end = Instant::now() + WAIT;
    loop {
        let view = h.feed.view();
        if view.snapshot.as_ref().map(|(_, s)| s.seq) == Some(2) {
            assert!(view.generation > generation);
            assert_eq!(view.schema.map(|s| s.sensors.len()), Some(3));
            break;
        }
        assert!(Instant::now() < end, "the new schema never arrived");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(shows(&h.status()), shows(&connected()));
}

#[test]
fn snapshot_without_schema_or_wrong_length_disconnects() {
    for script in [
        vec![hello(PROTOCOL_VERSION), snapshot(1, 2)],
        vec![hello(PROTOCOL_VERSION), schema(2), snapshot(1, 3)],
    ] {
        let control = FakeControl::new(running());
        let (conn, ctl) = fake_conn(Some(PID));
        for msg in script {
            ctl.push(msg);
        }
        let h = Harness::spawn(control, Script::with(vec![conn]), false);
        h.wait_for(is(disconnected()));
        let view = h.feed.view();
        assert!(view.schema.is_none());
        assert!(view.snapshot.is_none());
        cycles(1);
        assert!(!ctl.is_alive());
    }
}

#[test]
fn invalid_schema_disconnects() {
    let control = FakeControl::new(running());
    let (conn, ctl) = fake_conn(Some(PID));
    let mut bad = wire_schema(2);
    bad.sensors[1].device_id = "nowhere".to_owned();
    ctl.push(hello(PROTOCOL_VERSION));
    ctl.push(Message::Schema(bad));
    ctl.push(snapshot(1, 2));
    let h = Harness::spawn(control, Script::with(vec![conn]), false);
    h.wait_for(is(disconnected()));
    assert!(h.feed.view().schema.is_none());
    let connects = h.script.connects();
    cycles(2);
    assert!(h.script.connects() > connects, "the link reconnects");
}

#[test]
fn silent_stream_times_out_and_clears_feed() {
    let control = FakeControl::new(running());
    let (conn, _ctl) = streaming_conn(Some(PID));
    let settings = LinkSettings {
        interval_ms: 40,
        ..test_settings()
    };
    let h = Harness::spawn_with(control, Script::with(vec![conn]), false, settings);
    h.wait_for(is(connected()));
    let generation = h.feed.view().generation;

    // Nothing more arrives: after 3 x 40 ms the link gives up.
    h.wait_for(is(disconnected()));
    let view = h.feed.view();
    assert!(view.generation > generation);
    assert!(view.schema.is_none());
    assert!(view.snapshot.is_none());
}

#[test]
fn disconnect_clears_the_feed() {
    let control = FakeControl::new(running());
    let (conn, ctl) = streaming_conn(Some(PID));
    let h = Harness::spawn(control, Script::with(vec![conn]), false);
    h.wait_for(is(connected()));
    let generation = h.feed.view().generation;

    ctl.close();
    h.wait_for(is(disconnected()));
    let view = h.feed.view();
    assert!(view.generation > generation);
    assert!(view.schema.is_none());
    assert!(view.snapshot.is_none());
}

#[test]
fn the_link_reconnects_after_a_close() {
    let control = FakeControl::new(running());
    let (first, ctl) = streaming_conn(Some(PID));
    let h = Harness::spawn(control, Script::with(vec![first]), false);
    h.wait_for(is(connected()));
    let (second, _ctl2) = streaming_conn(Some(PID));
    h.script.add(second);
    ctl.close();
    h.wait_for(is(disconnected()));
    h.wait_for(is(connected()));
    assert_eq!(h.control.starts(), 0);
}

// ---- one queue, no wake-ups ----

/// A driver around `machine` for tests that call `next_event` directly.
fn bare_driver(machine: Machine) -> (Driver, SyncSender<Input>) {
    let (tx, rx) = mpsc::sync_channel(LINK_QUEUE_CAPACITY);
    let driver = Driver {
        machine,
        control: FakeControl::new(running()),
        connector: Script::with(vec![]).connector(),
        status: ServiceStatusTable::default(),
        feed: SvcFeed::default(),
        inbox: rx,
        sender: tx.clone(),
        stop: Arc::new(AtomicBool::new(false)),
        conn: None,
        conn_sink: None,
        conn_id: Some(7),
        unread: VecDeque::new(),
        pending: VecDeque::new(),
        waits: Arc::new(AtomicUsize::new(0)),
    };
    (driver, tx)
}

#[test]
fn connected_link_does_not_wake_between_events() {
    let control = FakeControl::new(running());
    let (conn, ctl) = streaming_conn(Some(PID));
    let h = Harness::spawn(control, Script::with(vec![conn]), false);
    h.wait_for(is(connected()));
    let waits = h.link.as_ref().unwrap().waits();

    // Interval 1000 ms, no message: the silence limit is 3 s away.
    std::thread::sleep(Duration::from_secs(1));
    let idle = h.link.as_ref().unwrap().waits() - waits;
    assert!(idle <= 3, "the thread woke {idle} times in 1 s");
    assert_eq!(shows(&h.status()), shows(&connected()));

    // And it is still listening.
    ctl.push(snapshot(2, 2));
    let end = Instant::now() + WAIT;
    while h.feed.view().snapshot.map(|(_, s)| s.seq) != Some(2) {
        assert!(Instant::now() < end, "the second snapshot never arrived");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn messages_and_commands_share_one_queue() {
    // In order, without waiting between them.
    let t0 = Instant::now();
    let (mut driver, tx) = bare_driver(subscribed(t0));
    tx.send(Input::Message(7, schema(2))).unwrap();
    tx.send(Input::Command(LinkCommand::SetInterval(2000)))
        .unwrap();
    tx.send(Input::Message(7, snapshot(1, 2))).unwrap();
    assert!(matches!(
        driver.next_event(),
        Some(Event::Message(Message::Schema(_)))
    ));
    assert!(matches!(
        driver.next_event(),
        Some(Event::Command(LinkCommand::SetInterval(2000)))
    ));
    assert!(matches!(
        driver.next_event(),
        Some(Event::Message(Message::Snapshot(_)))
    ));
    assert_eq!(
        driver.waits.load(Ordering::Relaxed),
        0,
        "nothing to wait for"
    );
}

#[test]
fn a_command_sent_while_a_snapshot_arrives_is_handled_at_once() {
    let control = FakeControl::new(running());
    let (conn, ctl) = streaming_conn(Some(PID));
    let h = Harness::spawn(control, Script::with(vec![conn]), false);
    h.wait_for(is(connected()));

    // The old 50 ms read slice would take up to 50 ms; here the bound is
    // generous for a loaded machine but still under one slice.
    let start = Instant::now();
    ctl.push(snapshot(2, 2));
    h.send(LinkCommand::SetInterval(2000));
    wait_for_sent(&ctl, 2);
    let took = start.elapsed();
    assert!(took < Duration::from_millis(45), "took {took:?}");
}

#[test]
fn late_events_of_a_closed_connection_are_ignored() {
    let control = FakeControl::new(running());
    let (first, ctl1) = streaming_conn(Some(PID));
    let h = Harness::spawn(control, Script::with(vec![first]), false);
    h.wait_for(is(connected()));
    let (second, _ctl2) = streaming_conn(Some(PID));
    h.script.add(second);
    ctl1.close();
    h.wait_for(is(disconnected()));
    h.wait_for(is(connected()));
    let generation = h.feed.view().generation;

    // The first connection's reader still had events in flight.
    ctl1.push(schema(5));
    ctl1.push(snapshot(9, 5));
    ctl1.close();
    cycles(3);
    assert_eq!(
        shows(&h.status()),
        shows(&connected()),
        "the new connection is untouched"
    );
    let view = h.feed.view();
    assert_eq!(view.generation, generation);
    assert_eq!(view.schema.map(|s| s.sensors.len()), Some(2));
    assert_eq!(view.snapshot.map(|(_, s)| s.seq), Some(1));
}

#[test]
fn events_of_another_connection_are_dropped_by_next_event() {
    let t0 = Instant::now();
    let (mut driver, tx) = bare_driver(subscribed(t0));
    tx.send(Input::Message(6, schema(9))).unwrap();
    tx.send(Input::Closed(6, CloseReason::Disconnected))
        .unwrap();
    tx.send(Input::Message(7, schema(2))).unwrap();
    assert!(matches!(
        driver.next_event(),
        Some(Event::Message(Message::Schema(s))) if s.sensors.len() == 2
    ));
}

#[test]
fn a_hello_that_arrives_while_verifying_is_not_lost() {
    // The server greets at once; the machine is still checking the PID.
    let control = FakeControl::new(running());
    let (conn, ctl) = fake_conn(Some(PID));
    ctl.push(hello(PROTOCOL_VERSION));
    ctl.push(schema(2));
    ctl.push(snapshot(1, 2));
    let h = Harness::spawn(control, Script::with(vec![conn]), false);
    h.wait_for(is(connected()));
    assert_eq!(h.feed.view().snapshot.map(|(_, s)| s.seq), Some(1));
}

#[test]
fn unread_messages_wait_for_the_machine_and_keep_their_order() {
    let t0 = Instant::now();
    let (m, _) = machine(false);
    let (mut driver, tx) = bare_driver(m);
    // Verifying: not reading the connection yet.
    driver.machine.phase = Phase::Verifying {
        server_pid: Some(PID),
    };
    driver.machine.deadline = Some(t0 + Duration::from_millis(30));
    tx.send(Input::Message(7, hello(PROTOCOL_VERSION))).unwrap();
    tx.send(Input::Closed(7, CloseReason::Disconnected))
        .unwrap();
    assert!(matches!(driver.next_event(), Some(Event::Timer)));

    driver.machine.phase = Phase::Hello;
    driver.machine.deadline = None;
    assert!(matches!(
        driver.next_event(),
        Some(Event::Message(Message::Hello(_)))
    ));
    assert!(matches!(driver.next_event(), Some(Event::Closed(_))));
}

#[test]
fn dropping_the_link_without_shutdown_stops_the_thread() {
    let control = FakeControl::new(running());
    let (conn, ctl) = streaming_conn(Some(PID));
    let mut h = Harness::spawn(control, Script::with(vec![conn]), false);
    h.wait_for(is(connected()));
    assert!(ctl.is_alive());
    drop(h.link.take());
    let end = Instant::now() + WAIT;
    while ctl.is_alive() {
        assert!(Instant::now() < end, "the thread never stopped");
        std::thread::sleep(Duration::from_millis(1));
    }
}

// ---- the bounded queue ----

/// A link whose thread is stuck in the write of its first `Subscribe`.
fn stuck_link() -> (Harness, ConnCtl) {
    let control = FakeControl::new(running());
    let (conn, ctl) = fake_conn(Some(PID));
    ctl.hold_writes();
    ctl.push(hello(PROTOCOL_VERSION));
    let h = Harness::spawn(control, Script::with(vec![conn]), false);
    ctl.wait_for_writes(1);
    (h, ctl)
}

/// Fills the stuck link's queue with commands that each cost a write.
fn fill_with_commands(h: &Harness) {
    for i in 0..LINK_QUEUE_CAPACITY {
        let interval = 1001 + u32::try_from(i).unwrap();
        assert_eq!(
            h.try_send(LinkCommand::SetInterval(interval)),
            Ok(()),
            "command {i} fits"
        );
    }
}

#[test]
fn send_returns_busy_when_the_queue_is_full() {
    let (h, ctl) = stuck_link();
    fill_with_commands(&h);

    let t = Instant::now();
    let result = h.try_send(LinkCommand::Start);
    let took = t.elapsed();
    assert_eq!(result, Err(LinkBusy));
    assert!(took < Duration::from_millis(50), "send took {took:?}");
    assert_eq!(LinkBusy.to_string(), "the sensor service link is busy");
    ctl.open_writes();
}

#[test]
fn a_full_queue_drops_snapshots_but_keeps_the_schema() {
    let control = FakeControl::new(running());
    let (conn, ctl) = streaming_conn(Some(PID));
    let h = Harness::spawn(control, Script::with(vec![conn]), false);
    h.wait_for(is(connected()));

    // Stuck in the write of a new `Subscribe`.
    ctl.hold_writes();
    h.send(LinkCommand::SetInterval(2000));
    ctl.wait_for_writes(2);

    // The reader (here, this thread) fills the queue with snapshots...
    let capacity = u64::try_from(LINK_QUEUE_CAPACITY).unwrap();
    for seq in 2..2 + capacity {
        ctl.push(snapshot(seq, 2));
    }
    assert_eq!(ctl.dropped_snapshots(), 0);
    // ...and the next ones are dropped, without blocking it.
    ctl.push(snapshot(2 + capacity, 2));
    ctl.push(snapshot(3 + capacity, 2));
    assert_eq!(ctl.dropped_snapshots(), 2);

    // A schema waits for room instead: the reader blocks until the link
    // drains its queue.
    let reader = {
        let ctl = ctl.clone();
        std::thread::spawn(move || ctl.push(schema(3)))
    };
    std::thread::sleep(Duration::from_millis(20));
    assert!(!reader.is_finished(), "the schema waits for room");
    ctl.open_writes();
    reader.join().unwrap();

    let end = Instant::now() + WAIT;
    while h.feed.view().schema.as_deref() != Some(&wire_schema(3)) {
        assert!(Instant::now() < end, "the schema never arrived");
        std::thread::sleep(Duration::from_millis(1));
    }
    ctl.push(snapshot(10_000, 3));
    let end = Instant::now() + WAIT;
    while h.feed.view().snapshot.map(|(_, s)| s.seq) != Some(10_000) {
        assert!(Instant::now() < end, "the stream did not resume");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        shows(&h.status()),
        shows(&connected()),
        "the connection is kept"
    );
}

#[test]
fn a_reader_waiting_for_room_gives_up_when_the_connection_is_dropped() {
    let (tx, _inbox) = mpsc::sync_channel(1);
    let sink = LinkSink::new(9, tx);
    assert!(sink.message(hello(PROTOCOL_VERSION)));
    let reader = {
        let sink = sink.clone();
        std::thread::spawn(move || sink.message(schema(1)))
    };
    std::thread::sleep(Duration::from_millis(20));
    assert!(!reader.is_finished(), "the schema waits for room");
    // What the link does before dropping (and so joining) the reader.
    sink.shared.cancel();
    assert!(!reader.join().unwrap(), "the reader is told to close");
}

#[test]
fn the_link_thread_never_waits_for_room() {
    // `LinkSink::new` makes this thread the one that drains the queue.
    let (tx, _inbox) = mpsc::sync_channel(1);
    let sink = LinkSink::new(9, tx);
    assert!(sink.message(hello(PROTOCOL_VERSION)));
    let t = Instant::now();
    assert!(!sink.message(schema(1)), "no room: dropped, not waited for");
    sink.closed(CloseReason::Disconnected);
    assert!(t.elapsed() < Duration::from_millis(50));
}
