//! End-to-end runs of the real `oma-load.exe` over the private pipe. Every plan uses at
//! most 2 logical CPUs and at most 2 s per phase.
#![cfg(windows)]

use std::os::windows::process::CommandExt;
use std::process::{Child, Command};
use std::sync::mpsc::{sync_channel, Receiver};
use std::time::{Duration, Instant};

use oma_ipc::load::*;
use oma_win::job::KillOnCloseJob;
use oma_win::load_pipe::{create_load_server, random_load_pipe_name, LoadConnection};
use oma_win::private_pipe::PipeEvent;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const WAIT: Duration = Duration::from_secs(10);

struct Session {
    conn: LoadConnection,
    rx: Receiver<PipeEvent<LoadMessage>>,
    child: Child,
    _job: KillOnCloseJob,
    _reader: oma_win::private_pipe::PipeReader,
    isa: Isa,
    cores: Vec<u32>,
}

impl Session {
    fn start(extra: &[&str]) -> Self {
        let name = random_load_pipe_name().unwrap();
        let server = create_load_server(&name).unwrap();
        let job = KillOnCloseJob::new().unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_oma-load"))
            .args(["--pipe", &name])
            .args(extra)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .unwrap();
        job.assign(&child).unwrap();
        server.accept(WAIT).expect("the child connects");
        let conn: LoadConnection = server.into_connection();
        let (tx, rx) = sync_channel(4096);
        let reader = conn.start_reader(move |e| tx.try_send(e).is_ok());
        let mut s = Session {
            conn,
            rx,
            child,
            _job: job,
            _reader: reader,
            isa: Isa::Sse2,
            cores: vec![],
        };
        let LoadMessage::Hello(h) = s.recv() else {
            panic!("Hello first")
        };
        assert!(!h.isa.is_empty());
        s.isa = h.isa[0];
        let LoadMessage::Topology(t) = s.recv() else {
            panic!("Topology second")
        };
        assert!(t.logical.iter().all(|l| l.apic_id.is_some()));
        for l in &t.logical {
            if !s.cores.contains(&l.core) && s.cores.len() < 2 {
                s.cores.push(l.core);
            }
        }
        s.send_hello();
        s
    }

    fn send_hello(&self) {
        self.conn
            .send(&LoadMessage::Hello(LoadHello {
                protocol_version: LOAD_PROTOCOL_VERSION,
                version: "e2e".into(),
                isa: vec![],
            }))
            .unwrap();
    }

    fn recv(&mut self) -> LoadMessage {
        match self.rx.recv_timeout(WAIT).expect("a message in time") {
            PipeEvent::Message(m) => m,
            PipeEvent::Closed(r) => panic!("pipe closed: {r:?}"),
        }
    }

    /// Messages up to and including `Finished`.
    fn until_finished(&mut self) -> Vec<LoadMessage> {
        let mut all = vec![];
        loop {
            let m = self.recv();
            let done = matches!(m, LoadMessage::Finished(_));
            all.push(m);
            if done {
                return all;
            }
        }
    }

    fn phase(&self, kernel: KernelId, secs: u32, workers: usize) -> Phase {
        let ram = matches!(kernel, KernelId::K3 | KernelId::K4 | KernelId::K10);
        Phase {
            kernel,
            alt_kernel: None,
            isa: self.isa,
            size: if ram { DataSize::Ram } else { DataSize::L2 },
            mode: LoadMode::Steady,
            placement: Placement::OnePerCore,
            duration_s: secs,
            per_core_s: None,
            both_smt: false,
            cores: Some(self.cores[..workers.min(self.cores.len())].to_vec()),
            patterns: if kernel == KernelId::K10 {
                vec![RamPattern::Random]
            } else {
                vec![]
            },
            stop_on_error: false,
        }
    }

    fn run(&self, phases: Vec<Phase>) {
        let ram = phases.iter().any(|p| p.size == DataSize::Ram);
        self.conn
            .send(&LoadMessage::Run(RunRequest {
                plan: Plan {
                    seed: 7,
                    ram_bytes: if ram { 256 << 20 } else { 0 },
                    phases,
                },
            }))
            .unwrap();
    }

    /// Closes our end of the pipe (the reader holds the connection too) and waits for the exit.
    fn close_and_wait(self) -> i32 {
        drop(self.conn);
        drop(self._reader);
        wait_exit(self.child, self._job)
    }
}

fn wait_exit(mut child: Child, _job: KillOnCloseJob) -> i32 {
    {
        let end = Instant::now() + WAIT;
        loop {
            if let Some(s) = child.try_wait().unwrap() {
                return s.code().unwrap();
            }
            assert!(Instant::now() < end, "the process did not exit");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn finished(all: &[LoadMessage]) -> &Finished {
    match all.last() {
        Some(LoadMessage::Finished(f)) => f,
        _ => panic!("no Finished"),
    }
}

#[test]
fn handshake_topology_and_short_plan_complete() {
    let mut s = Session::start(&[]);
    let mut p = s.phase(KernelId::K5, 2, 2);
    p.size = DataSize::L2;
    s.run(vec![p]);
    let all = s.until_finished();
    assert!(all.iter().any(|m| matches!(m, LoadMessage::Progress(_))));
    assert!(all
        .iter()
        .any(|m| matches!(m, LoadMessage::PhaseDone(d) if d.skipped.is_none())));
    assert!(!all.iter().any(|m| matches!(m, LoadMessage::Error(_))));
    assert_eq!(finished(&all).reason, FinishReason::Completed);
    assert_eq!(s.close_and_wait(), 0);
}

#[test]
fn every_kernel_runs_one_second() {
    let mut s = Session::start(&[]);
    use KernelId::*;
    let phases: Vec<Phase> = [K1, K2, K3, K4, K5, K7, K8, K9, K10]
        .into_iter()
        .map(|k| s.phase(k, 1, if k == K9 { 2 } else { 1 }))
        .collect();
    let n = phases.len();
    s.run(phases);
    let all = s.until_finished();
    assert!(!all.iter().any(|m| matches!(m, LoadMessage::Error(_))));
    let done: Vec<_> = all
        .iter()
        .filter_map(|m| match m {
            LoadMessage::PhaseDone(d) => Some(d),
            _ => None,
        })
        .collect();
    assert_eq!(done.len(), n);
    assert_eq!(finished(&all).reason, FinishReason::Completed);
}

#[cfg(debug_assertions)]
#[test]
fn injected_fault_reaches_the_app_as_error_on_core_1() {
    let mut s = Session::start(&["--inject-fault", "k5:1"]);
    let mut p = s.phase(KernelId::K5, 2, 2);
    p.size = DataSize::L2;
    s.run(vec![p]);
    let all = s.until_finished();
    let err = all
        .iter()
        .find_map(|m| match m {
            LoadMessage::Error(e) => Some(e),
            _ => None,
        })
        .expect("an Error");
    assert_eq!(err.kind, ErrorKind::Mismatch);
    assert_eq!(err.core, Some(1));
}

#[test]
fn stop_finishes_within_one_second() {
    let mut s = Session::start(&[]);
    s.run(vec![s.phase(KernelId::K5, 2, 2)]);
    // Wait for the first Progress so the workers are running.
    while !matches!(s.recv(), LoadMessage::Progress(_)) {}
    let t = Instant::now();
    s.conn.send(&LoadMessage::Stop(StopRequest {})).unwrap();
    let all = s.until_finished();
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    assert_eq!(finished(&all).reason, FinishReason::Stopped);
}

#[test]
fn closing_the_pipe_exits_the_process() {
    let mut s = Session::start(&[]);
    s.run(vec![s.phase(KernelId::K5, 2, 2)]);
    while !matches!(s.recv(), LoadMessage::Progress(_)) {}
    assert_eq!(s.close_and_wait(), 0);
}

#[test]
fn invalid_plan_exits_with_usage() {
    let s = Session::start(&[]);
    s.run(vec![]);
    assert_eq!(wait_exit(s.child, s._job), 1);
}
