use super::stream::wait_for_sent;
use super::*;

// ---- shutdown ----

fn assert_quick_shutdown(h: &mut Harness, what: &str) {
    let t = Instant::now();
    h.link.take().unwrap().shutdown();
    let took = t.elapsed();
    assert!(
        took < Duration::from_secs(1),
        "{what}: shutdown took {took:?}"
    );
}

#[test]
fn shutdown_joins_quickly() {
    // Waiting for Hello (2 s timeout).
    let control = FakeControl::new(running());
    let (conn, ctl) = fake_conn(Some(PID));
    let mut h = Harness::spawn(control, Script::with(vec![conn]), false);
    let end = Instant::now() + WAIT;
    while h.script.connects() == 0 {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
    std::thread::sleep(Duration::from_millis(30));
    assert_quick_shutdown(&mut h, "during Hello");
    assert!(!ctl.is_alive(), "the connection is released");

    // A write the server never takes (the pipe client gives up after 2 s).
    let control = FakeControl::new(running());
    let (mut conn, ctl) = fake_conn(Some(PID));
    conn.send_block = Duration::from_secs(3);
    ctl.push(hello(PROTOCOL_VERSION));
    let mut h = Harness::spawn(control, Script::with(vec![conn]), false);
    std::thread::sleep(Duration::from_millis(100));
    assert_quick_shutdown(&mut h, "during a blocked write");

    // Verifying a STOP that never completes (30 s timeout).
    let control = FakeControl::new(running());
    control.with(|s| s.after_stop = Some(in_state(RunState::StopPending)));
    let settings = LinkSettings {
        stop_timeout: Duration::from_secs(30),
        stop_poll: Duration::from_millis(250),
        ..test_settings()
    };
    let mut h = Harness::spawn_with(control, Script::with(vec![]), true, settings);
    std::thread::sleep(Duration::from_millis(50));
    assert_quick_shutdown(&mut h, "during the STOP wait");

    // Idle in anti-cheat mode: stop confirmed, no deadline at all.
    let control = FakeControl::new(in_state(RunState::Stopped));
    let mut h = Harness::spawn(control, Script::with(vec![]), true);
    h.wait_for(is(st(ServiceState::AntiCheat, None)));
    assert_quick_shutdown(&mut h, "idle in anti-cheat mode");

    // In the retry wait.
    let control = FakeControl::new(ServiceQuery::NotInstalled);
    let settings = LinkSettings {
        retry: Duration::from_secs(5),
        ..test_settings()
    };
    let mut h = Harness::spawn_with(control, Script::with(vec![]), false, settings);
    std::thread::sleep(Duration::from_millis(50));
    assert_quick_shutdown(&mut h, "during the retry wait");
}

// ---- schema validation ----

#[test]
fn validate_schema_accepts_a_good_schema() {
    assert_eq!(validate_schema(&wire_schema(3)), Ok(()));
    assert_eq!(validate_schema(&wire_schema(0)), Ok(()));
    // The same name under another kind is another sensor id.
    let mut schema = wire_schema(2);
    schema.sensors[1].name = schema.sensors[0].name.clone();
    schema.sensors[1].kind = "load".to_owned();
    assert_eq!(validate_schema(&schema), Ok(()));
}

#[test]
fn validate_schema_rejects_what_the_provider_cannot_bind() {
    type Spoil = fn(&mut WireSchema);
    let cases: [(&str, Spoil); 9] = [
        ("duplicate device", |s| s.devices.push(s.devices[0].clone())),
        ("empty device id", |s| s.devices[0].id.clear()),
        ("slash in device id", |s| {
            s.devices[0].id = "cpu/0".to_owned()
        }),
        ("unknown device", |s| {
            s.sensors[0].device_id = "gpu-0".to_owned()
        }),
        ("empty sensor name", |s| s.sensors[0].name.clear()),
        ("slash in sensor name", |s| {
            s.sensors[0].name = "a/b".to_owned()
        }),
        ("duplicate sensor id", |s| {
            s.sensors.push(s.sensors[1].clone())
        }),
        ("empty sensor kind", |s| s.sensors[0].kind.clear()),
        ("slash in sensor kind", |s| {
            s.sensors[0].kind = "a/b".to_owned()
        }),
    ];
    for (what, spoil) in cases {
        let mut schema = wire_schema(2);
        spoil(&mut schema);
        assert!(validate_schema(&schema).is_err(), "{what} was accepted");
    }
}

// ---- source requests, effective sources and PawnIO ----

const DISK_ID: &str = "storage/device-a";

fn disk_key() -> String {
    oma_ipc::drive_key("Model A", "SN-A").unwrap()
}

/// A drive table that knows one disk.
fn one_disk_table() -> DriveIdTable {
    let drives = DriveIdTable::default();
    drives.publish(vec![DriveEntry::new(
        0,
        DISK_ID.to_owned(),
        Some("Model A".to_owned()),
        Some("SN-A".to_owned()),
    )]);
    drives
}

fn request(modules: &[&str], drives: &[&str]) -> SourceRequest {
    SourceRequest {
        disabled_modules: modules.iter().map(|m| (*m).to_owned()).collect(),
        smart_disabled_drives: drives.iter().map(|d| (*d).to_owned()).collect(),
        smart_enabled_drives: Vec::new(),
    }
}

fn subscribe_with(modules: &[&str], keys: &[String]) -> Message {
    Message::Subscribe(Subscribe {
        interval_ms: 1000,
        disabled_modules: modules.iter().map(|m| (*m).to_owned()).collect(),
        smart_disabled_drives: keys.to_vec(),
        smart_enabled_drives: Vec::new(),
    })
}

fn wire_drive(
    physical_drive: u32,
    key: Option<String>,
    state: &str,
    blocks_smart: bool,
) -> WireDrive {
    WireDrive {
        physical_drive,
        key,
        model: Some(format!("Model {physical_drive}")),
        state: state.to_owned(),
        blocks_smart,
    }
}

/// A `Schema` whose `service` block is `block`.
fn schema_with_block(block: WireServiceState) -> Message {
    let mut schema = wire_schema(2);
    schema.service = block;
    Message::Schema(schema)
}

fn block(reconfiguration: &str) -> WireServiceState {
    WireServiceState {
        reconfiguration: reconfiguration.to_owned(),
        ..WireServiceState::default()
    }
}

fn reconfiguration_of(m: &Machine) -> Option<Reconfiguration> {
    m.status.sources.as_ref().map(|s| s.reconfiguration)
}

/// A subscribed machine that has its schema (with `block`) and a snapshot.
fn streaming_machine(settings: LinkSettings, block: WireServiceState, now: Instant) -> Machine {
    let mut m = subscribed_with(settings, now, subscribe_with(&[], &[]));
    m.decide(Event::Message(schema_with_block(block)), now);
    m.decide(Event::Message(snapshot(1, 2)), now);
    assert_eq!(m.status.state, ServiceState::Connected);
    m
}

#[test]
fn set_sources_resubscribes_with_drive_keys() {
    let now = Instant::now();
    let settings = LinkSettings {
        drives: one_disk_table(),
        ..test_settings()
    };
    let mut m = subscribed_with(settings, now, subscribe_with(&[], &[]));
    m.decide(Event::Message(schema(2)), now);

    let wanted = request(&["psu", "cpu"], &[DISK_ID, "storage/unplugged"]);
    let effects = m.decide(Event::Command(LinkCommand::SetSources(wanted.clone())), now);
    assert_eq!(
        effects,
        vec![
            Effect::SetRequest(wanted.clone()),
            Effect::Send(subscribe_with(&["psu", "cpu"], &[disk_key()])),
        ],
        "core ids became keys, the unknown disk was dropped"
    );

    // The same request again is not sent twice.
    assert!(m
        .decide(Event::Command(LinkCommand::SetSources(wanted)), now)
        .is_empty());

    // While the link is not subscribed the request is only remembered.
    let mut idle = Machine::new(test_settings(), false);
    idle.phase = Phase::ConnectWait;
    let effects = idle.decide(
        Event::Command(LinkCommand::SetSources(request(&["psu"], &[]))),
        now,
    );
    assert_eq!(effects, vec![Effect::SetRequest(request(&["psu"], &[]))]);
}

#[test]
fn last_request_is_sent_on_connect() {
    let now = Instant::now();
    let settings = LinkSettings {
        sources: request(&["motherboard"], &[DISK_ID]),
        drives: one_disk_table(),
        ..test_settings()
    };
    // The first `Subscribe` of a connection already carries the request.
    subscribed_with(
        settings,
        now,
        subscribe_with(&["motherboard"], &[disk_key()]),
    );

    // A request that arrived while disconnected is used by the next connection.
    let mut m = Machine::new(test_settings(), false);
    m.phase = Phase::ConnectWait;
    m.decide(
        Event::Command(LinkCommand::SetSources(request(&["psu"], &[]))),
        now,
    );
    m.phase = Phase::Hello;
    let effects = m.decide(Event::Message(hello(PROTOCOL_VERSION)), now);
    assert!(effects.contains(&Effect::Send(subscribe_with(&["psu"], &[]))));
}

#[test]
fn a_disk_published_before_the_first_schema_is_still_sent() {
    let now = Instant::now();
    let drives = DriveIdTable::default();
    let settings = LinkSettings {
        sources: request(&[], &[DISK_ID]),
        drives: drives.clone(),
        ..test_settings()
    };
    // Hello and Subscribe go out with no disk known.
    let mut m = subscribed_with(settings, now, subscribe_with(&[], &[]));
    // Storage discovery runs before the first schema arrives.
    drives.publish(one_disk_table().get().drives);
    m.decide(Event::Message(schema(2)), now);
    // The schema translated the sources with the new table, but that must
    // not count as having sent its keys: the first snapshot sends them.
    let effects = m.decide(Event::Message(snapshot(1, 2)), now);
    assert!(
        effects.contains(&Effect::Send(subscribe_with(&[], &[disk_key()]))),
        "{effects:?}"
    );
    // Then it is settled.
    assert_eq!(
        m.decide(Event::Message(snapshot(2, 2)), now),
        vec![Effect::SetSnapshot(wire_snapshot(2, 2))]
    );
}

#[test]
fn a_disk_that_appears_later_is_sent_with_a_new_subscribe() {
    let now = Instant::now();
    let drives = DriveIdTable::default();
    let settings = LinkSettings {
        sources: request(&[], &[DISK_ID]),
        drives: drives.clone(),
        ..test_settings()
    };
    // The disk is not in the table yet: nothing to send for it.
    let mut m = subscribed_with(settings, now, subscribe_with(&[], &[]));
    m.decide(Event::Message(schema(2)), now);
    assert_eq!(
        m.decide(Event::Message(snapshot(1, 2)), now),
        vec![Effect::SetSnapshot(wire_snapshot(1, 2))]
    );

    // The storage provider publishes it: the next snapshot brings the key.
    drives.publish(one_disk_table().get().drives);
    let effects = m.decide(Event::Message(snapshot(2, 2)), now);
    assert!(effects.contains(&Effect::Send(subscribe_with(&[], &[disk_key()]))));
    // Then it is settled.
    assert_eq!(
        m.decide(Event::Message(snapshot(3, 2)), now),
        vec![Effect::SetSnapshot(wire_snapshot(3, 2))]
    );
}

#[test]
fn sources_are_pending_until_the_service_reflects_the_request() {
    let now = Instant::now();
    let mut m = streaming_machine(test_settings(), block("applied"), now);
    assert_eq!(reconfiguration_of(&m), Some(Reconfiguration::Applied));

    m.decide(
        Event::Command(LinkCommand::SetSources(request(&["psu"], &[]))),
        now,
    );
    assert_eq!(reconfiguration_of(&m), Some(Reconfiguration::Pending));

    // The service is still working on it.
    m.decide(Event::Message(schema_with_block(block("pending"))), now);
    assert_eq!(reconfiguration_of(&m), Some(Reconfiguration::Pending));

    // The forced schema of the request arrives, applied.
    let mut done = block("applied");
    done.active_modules.retain(|name| name != "psu");
    m.decide(Event::Message(schema_with_block(done)), now);
    let sources = m.status.sources.clone().expect("sources");
    assert_eq!(sources.reconfiguration, Reconfiguration::Applied);
    assert!(!sources.active_modules.contains(&"psu".to_owned()));
}

#[test]
fn sources_name_the_request_they_refer_to() {
    let now = Instant::now();
    let mut m = streaming_machine(test_settings(), block("applied"), now);
    let requested = |m: &Machine| {
        m.status
            .sources
            .as_ref()
            .map(|s| s.requested_disabled_modules.clone())
    };
    // Before any request the service reflects nothing this app turned off.
    assert_eq!(requested(&m), Some(Vec::new()));

    m.decide(
        Event::Command(LinkCommand::SetSources(request(&["psu"], &[]))),
        now,
    );
    assert_eq!(requested(&m), Some(vec!["psu".to_owned()]));
    m.decide(Event::Message(schema_with_block(block("applied"))), now);
    assert_eq!(requested(&m), Some(vec!["psu".to_owned()]));
}

#[test]
fn another_client_keeping_a_module_on_still_reads_as_applied() {
    let now = Instant::now();
    let mut m = streaming_machine(test_settings(), block("applied"), now);
    m.decide(
        Event::Command(LinkCommand::SetSources(request(&["psu"], &[]))),
        now,
    );
    // The block still lists psu as active: someone else wants it.
    m.decide(Event::Message(schema_with_block(block("applied"))), now);
    let sources = m.status.sources.clone().expect("sources");
    assert_eq!(sources.reconfiguration, Reconfiguration::Applied);
    assert!(sources.active_modules.contains(&"psu".to_owned()));
}

#[test]
fn failed_reconfiguration_is_reported() {
    let now = Instant::now();
    let mut m = streaming_machine(test_settings(), block("applied"), now);
    m.decide(
        Event::Command(LinkCommand::SetSources(request(&["cpu"], &[]))),
        now,
    );
    m.decide(Event::Message(schema_with_block(block("failed"))), now);
    assert_eq!(reconfiguration_of(&m), Some(Reconfiguration::Failed));
}

#[test]
fn service_keys_come_back_as_core_ids() {
    let now = Instant::now();
    let settings = LinkSettings {
        drives: one_disk_table(),
        ..test_settings()
    };
    let mut wire = block("applied");
    wire.smart_disabled_drives = vec![disk_key(), "unknown-disk".to_owned()];
    wire.drives = vec![
        wire_drive(0, Some(disk_key()), "standby", true),
        wire_drive(1, Some("unknown-disk".to_owned()), "bogus", true),
        wire_drive(2, None, "active", false),
    ];
    let m = streaming_machine(settings, wire, now);
    let sources = m.status.sources.expect("sources");
    assert_eq!(sources.smart_disabled_drives, vec![DISK_ID.to_owned()]);
    assert_eq!(
        sources.drives,
        vec![
            SourceDrive {
                physical_drive: 0,
                device_id: Some(DISK_ID.to_owned()),
                model: Some("Model 0".to_owned()),
                state: DriveState::Standby,
                blocks_smart: true,
            },
            SourceDrive {
                physical_drive: 1,
                device_id: None,
                model: Some("Model 1".to_owned()),
                state: DriveState::Unknown,
                blocks_smart: true,
            },
            SourceDrive {
                physical_drive: 2,
                device_id: None,
                model: Some("Model 2".to_owned()),
                state: DriveState::Active,
                blocks_smart: false,
            },
        ]
    );
}

#[test]
fn a_drive_is_matched_to_a_core_disk_by_number_and_key() {
    let now = Instant::now();
    let settings = LinkSettings {
        drives: one_disk_table(),
        ..test_settings()
    };
    let device_ids = |wire: WireServiceState| -> Vec<Option<String>> {
        let m = streaming_machine(settings.clone(), wire, now);
        let sources = m.status.sources.expect("sources");
        sources.drives.into_iter().map(|d| d.device_id).collect()
    };

    // The disk's key under another drive number: not the same disk.
    let mut moved = block("applied");
    moved.drives = vec![wire_drive(1, Some(disk_key()), "active", false)];
    assert_eq!(device_ids(moved), vec![None]);

    // The key twice in the service's table: neither is matched.
    let mut twice = block("applied");
    twice.drives = vec![
        wire_drive(0, Some(disk_key()), "active", false),
        wire_drive(1, Some(disk_key()), "active", false),
    ];
    assert_eq!(device_ids(twice), vec![None, None]);

    let mut same = block("applied");
    same.drives = vec![wire_drive(0, Some(disk_key()), "active", false)];
    assert_eq!(device_ids(same), vec![Some(DISK_ID.to_owned())]);
}

#[test]
fn enabled_drives_are_translated_and_a_change_resubscribes() {
    let now = Instant::now();
    let drives = DriveIdTable::default();
    let settings = LinkSettings {
        sources: SourceRequest {
            smart_enabled_drives: vec![DISK_ID.to_owned()],
            ..SourceRequest::default()
        },
        drives: drives.clone(),
        ..test_settings()
    };
    let enabled_subscribe = |keys: Vec<String>| {
        Message::Subscribe(Subscribe {
            smart_enabled_drives: keys,
            ..subscribe_request(1000)
        })
    };
    // The disk is not known yet: the first Subscribe carries no key.
    let mut m = subscribed_with(settings, now, enabled_subscribe(vec![]));
    m.decide(Event::Message(schema(2)), now);
    assert_eq!(
        m.decide(Event::Message(snapshot(1, 2)), now),
        vec![Effect::SetSnapshot(wire_snapshot(1, 2))]
    );

    // Once the disk is known, the key goes out in smart_enabled_drives.
    drives.publish(one_disk_table().get().drives);
    let effects = m.decide(Event::Message(snapshot(2, 2)), now);
    assert!(
        effects.contains(&Effect::Send(enabled_subscribe(vec![disk_key()]))),
        "{effects:?}"
    );
    // Then it is settled.
    assert_eq!(
        m.decide(Event::Message(snapshot(3, 2)), now),
        vec![Effect::SetSnapshot(wire_snapshot(3, 2))]
    );
}

#[test]
fn sources_and_pawn_io_vanish_with_the_connection() {
    let now = Instant::now();
    let mut m = streaming_machine(test_settings(), block("applied"), now);
    assert_eq!(m.status.pawn_io, Some(PawnIoStatus::Ok));
    assert!(m.status.sources.is_some());
    m.decide(Event::Closed(CloseReason::Disconnected), now);
    assert_eq!(m.status, disconnected());
}

#[test]
fn pawn_io_status_reaches_the_service_status() {
    let control = FakeControl::new(running());
    let (conn, ctl) = fake_conn(Some(PID));
    ctl.push(Message::Hello(Hello {
        protocol_version: PROTOCOL_VERSION,
        service_version: "test".to_owned(),
        pawn_io: "rebootPending".to_owned(),
    }));
    ctl.push(schema(2));
    ctl.push(snapshot(1, 2));
    let h = Harness::spawn(control, Script::with(vec![conn]), false);
    let status = h.wait_for(|s| s.state == ServiceState::Connected);
    assert_eq!(status.pawn_io, Some(PawnIoStatus::RebootPending));
    let sources = status.sources.expect("the block of the first schema");
    assert_eq!(sources.reconfiguration, Reconfiguration::Applied);
    assert_eq!(sources.active_modules.len(), oma_ipc::MODULES.len());

    // Gone with the connection.
    ctl.close();
    let status = h.wait_for(|s| s.state != ServiceState::Connected);
    assert_eq!((status.pawn_io, status.sources), (None, None));
}

#[test]
fn the_last_hello_gives_the_service_version() {
    let control = FakeControl::new(running());
    let (conn, ctl) = fake_conn(Some(PID));
    ctl.push(Message::Hello(Hello {
        protocol_version: PROTOCOL_VERSION,
        service_version: "9.8.7".to_owned(),
        pawn_io: "ok".to_owned(),
    }));
    ctl.push(schema(2));
    ctl.push(snapshot(1, 2));
    let h = Harness::spawn(control, Script::with(vec![conn]), false);
    h.wait_for(|s| s.state == ServiceState::Connected);
    assert_eq!(h.status.service_version().as_deref(), Some("9.8.7"));

    // Kept once the connection is gone: the last service that answered.
    ctl.close();
    h.wait_for(|s| s.state != ServiceState::Connected);
    assert_eq!(h.status.service_version().as_deref(), Some("9.8.7"));
}

#[test]
fn set_sources_reaches_the_service_and_the_feed() {
    let control = FakeControl::new(running());
    let (conn, ctl) = streaming_conn(Some(PID));
    let h = Harness::spawn(control, Script::with(vec![conn]), false);
    h.wait_for(is(connected()));

    let wanted = request(&["storage"], &[]);
    h.send(LinkCommand::SetSources(wanted.clone()));
    let sent = wait_for_sent(&ctl, 2);
    assert_eq!(sent[1], subscribe_with(&["storage"], &[]));
    let end = Instant::now() + WAIT;
    while *h.feed.view().request != wanted {
        assert!(Instant::now() < end, "the feed never got the request");
        std::thread::sleep(Duration::from_millis(1));
    }
    h.wait_for(|s| {
        s.sources
            .as_ref()
            .is_some_and(|x| x.reconfiguration == Reconfiguration::Pending)
    });
}

#[test]
fn shutdown_returns_within_join_wait_with_a_full_queue() {
    let control = FakeControl::new(running());
    let (conn, ctl) = fake_conn(Some(PID));
    ctl.hold_writes();
    ctl.push(hello(PROTOCOL_VERSION));
    let mut h = Harness::spawn(control, Script::with(vec![conn]), false);
    ctl.wait_for_writes(1);
    // Each queued command would cost a write of its own.
    for i in 0..LINK_QUEUE_CAPACITY {
        let interval = 1001 + u32::try_from(i).unwrap();
        assert_eq!(h.try_send(LinkCommand::SetInterval(interval)), Ok(()));
    }
    assert_eq!(h.try_send(LinkCommand::Start), Err(LinkBusy));

    // The write completes while shutdown waits: the thread sees the stop
    // flag before the next input, so it exits instead of draining the queue.
    let opener = {
        let ctl = ctl.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            ctl.open_writes();
        })
    };
    let t = Instant::now();
    h.link.take().unwrap().shutdown();
    let took = t.elapsed();
    opener.join().unwrap();
    assert!(took < JOIN_WAIT, "shutdown took {took:?}");
    assert!(
        !ctl.is_alive(),
        "the thread exited and released the connection"
    );
    assert_eq!(ctl.sent().len(), 1, "no queued command was run");
}
