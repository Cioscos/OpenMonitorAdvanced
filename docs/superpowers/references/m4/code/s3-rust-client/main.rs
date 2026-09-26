//! Throwaway M4 spike: named pipe client + SCM probes (see s3-pipe-scm.md).
mod fake;
mod pipe;
mod scm;
mod synctest;

use std::os::windows::io::AsRawHandle;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{FILETIME, HANDLE};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes, GetThreadTimes};

use pipe::{Conn, OwnedHandle, PipeError};

fn ft100ns(f: FILETIME) -> u64 {
    (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime)
}

fn thread_cpu(h: HANDLE) -> Duration {
    let (mut c, mut e, mut k, mut u) = Default::default();
    // SAFETY: h is a live thread handle owned by a JoinHandle.
    unsafe { GetThreadTimes(h, &mut c, &mut e, &mut k, &mut u) }.expect("GetThreadTimes");
    Duration::from_nanos((ft100ns(k) + ft100ns(u)) * 100)
}

fn thread_cycles(h: HANDLE) -> u64 {
    let mut c = 0u64;
    // SAFETY: live thread handle, valid out pointer.
    unsafe { windows::Win32::System::WindowsProgramming::QueryThreadCycleTime(h, &mut c) }.expect("QueryThreadCycleTime");
    c
}

fn process_cpu() -> Duration {
    let (mut c, mut e, mut k, mut u) = Default::default();
    // SAFETY: pseudo handle of the current process.
    unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) }
        .expect("GetProcessTimes");
    Duration::from_nanos((ft100ns(k) + ft100ns(u)) * 100)
}

fn connect(name: &str) -> Result<Conn, PipeError> {
    pipe::open(name, pipe::CLIENT_ACCESS_GENERIC, true, Duration::from_secs(2)).map(Conn::new)
}

type Frames = mpsc::Receiver<Result<Vec<u8>, PipeError>>;

/// Reader thread: blocks on overlapped reads + stop event; forwards frames until error/stop.
fn spawn_reader(conn: &Arc<Conn>, stop: &Arc<OwnedHandle>) -> (std::thread::JoinHandle<()>, Frames) {
    let (tx, rx) = mpsc::channel();
    let (c, s) = (Arc::clone(conn), Arc::clone(stop));
    let j = std::thread::spawn(move || {
        loop {
            let r = c.read_frame(&s);
            let end = r.is_err();
            if tx.send(r).is_err() || end {
                return;
            }
        }
    });
    (j, rx)
}

fn text(r: Result<Vec<u8>, PipeError>) -> String {
    match r {
        Ok(v) => String::from_utf8_lossy(&v).into_owned(),
        Err(e) => format!("ERR {e:?}"),
    }
}

fn client(name: &str, expected_pid: u32) {
    let never = pipe::new_event(true);
    // 1. Connect + first frame, and request/response RTT, 20 times.
    let mut connect_hello = vec![];
    let mut rtt = vec![];
    for i in 0..20 {
        let t0 = Instant::now();
        let conn = connect(name).expect("connect");
        let hello = text(conn.read_frame(&never));
        let t1 = t0.elapsed();
        assert_eq!(hello, "hello");
        if i == 0 {
            let pid = pipe::server_pid(&conn.h).expect("GetNamedPipeServerProcessId");
            println!("server pid via GetNamedPipeServerProcessId = {pid}, expected {expected_pid}, match={}", pid == expected_pid);
        }
        let t2 = Instant::now();
        conn.write_frame(b"subscribe", 1000).unwrap();
        let echo = text(conn.read_frame(&never));
        rtt.push(t2.elapsed());
        assert_eq!(echo, "echo:subscribe");
        for _ in 0..3 {
            conn.read_frame(&never).unwrap();
        }
        connect_hello.push(t1);
    }
    connect_hello.sort();
    rtt.sort();
    println!(
        "connect+hello: min {:?} median {:?} max {:?} (first {:?} is not sorted out)",
        connect_hello[0], connect_hello[10], connect_hello[19], connect_hello.iter().max()
    );
    println!("write->echo RTT: min {:?} median {:?} max {:?}", rtt[0], rtt[10], rtt[19]);

    // 2. Long-lived connection with a reader thread.
    let conn = Arc::new(connect(name).unwrap());
    let stop = Arc::new(pipe::new_event(true));
    let (j, rx) = spawn_reader(&conn, &stop);
    println!("reader: {}", text(rx.recv().unwrap()));
    conn.write_frame(b"subscribe", 1000).unwrap();
    for _ in 0..4 {
        println!("reader: {}", text(rx.recv().unwrap()));
    }
    conn.write_frame(b"silent", 1000).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let th = HANDLE(j.as_raw_handle());
    let (c0, p0, y0, t0) = (thread_cpu(th), process_cpu(), thread_cycles(th), Instant::now());
    std::thread::sleep(Duration::from_secs(10));
    let (c1, p1, y1, el) = (thread_cpu(th), process_cpu(), thread_cycles(th), t0.elapsed());
    println!("idle: reader thread cycles consumed = {}", y1 - y0);
    println!(
        "idle {:?}: reader thread CPU {:?}, whole process CPU {:?}",
        el,
        c1 - c0,
        p1 - p0
    );
    // Concurrent write while the reader's ReadFile is pending (overlapped handle).
    let tw = Instant::now();
    conn.write_frame(b"again", 1000).unwrap();
    println!("write while read pending took {:?}; reader: {}", tw.elapsed(), text(rx.recv().unwrap()));
    for _ in 0..3 {
        rx.recv().unwrap().unwrap();
    }
    // Stop the reader from this thread.
    let ts = Instant::now();
    pipe::signal(&stop);
    let last = rx.recv().unwrap();
    j.join().unwrap();
    println!("stop event -> reader returned {:?}, thread joined in {:?}", last.err(), ts.elapsed());
    drop(conn);

    // 3. Server-side disconnect detection.
    let conn = Arc::new(connect(name).unwrap());
    let stop = Arc::new(pipe::new_event(true));
    let (j, rx) = spawn_reader(&conn, &stop);
    rx.recv().unwrap().unwrap(); // hello
    conn.write_frame(b"bye", 1000).unwrap();
    println!("after server Disconnect(): reader -> {}", text(rx.recv().unwrap()));
    j.join().unwrap();
    println!("write after disconnect -> {:?}", conn.write_frame(b"x", 1000));
}

fn killwatch(name: &str) {
    let conn = Arc::new(connect(name).unwrap());
    let stop = Arc::new(pipe::new_event(true));
    let (j, rx) = spawn_reader(&conn, &stop);
    rx.recv().unwrap().unwrap();
    println!("WAITING (kill the server now)");
    let t = Instant::now();
    let r = rx.recv_timeout(Duration::from_secs(30));
    println!("after server process exit: reader -> {:?} after {:?}", r.map(text), t.elapsed());
    let _ = j.join();
}

fn busy(name: &str) {
    let mut held = vec![];
    for i in 0..8 {
        match connect(name) {
            Ok(c) => held.push(c),
            Err(e) => println!("connection {i}: {e:?}"),
        }
    }
    println!("holding {} connections", held.len());
    let t = Instant::now();
    let r = pipe::open(name, pipe::CLIENT_ACCESS_GENERIC, true, Duration::ZERO);
    println!("9th CreateFileW, no wait: {:?}", r.err());
    let r = pipe::wait_named_pipe_raw(name, 500);
    println!("WaitNamedPipeW(500 ms) while all busy: {r:?} after {:?}", t.elapsed());
    let r = pipe::open(name, pipe::CLIENT_ACCESS_GENERIC, true, Duration::from_millis(700));
    println!("open with 700 ms busy-wait: {:?}", r.err());
    // Free one slot from another thread after 300 ms; the waiting open must succeed.
    let one = held.pop().unwrap();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        drop(one);
    });
    let t = Instant::now();
    let r = pipe::open(name, pipe::CLIENT_ACCESS_GENERIC, true, Duration::from_secs(3));
    println!("open while a slot frees after 300 ms: ok={} after {:?}", r.is_ok(), t.elapsed());
}

fn absent() {
    let name = format!("OpenMonitorAdvanced.Sensors.absent-{}", std::process::id());
    let t = Instant::now();
    println!("CreateFileW on absent pipe: {:?} in {:?}", pipe::open(&name, pipe::CLIENT_ACCESS_GENERIC, true, Duration::ZERO).err(), t.elapsed());
    let t = Instant::now();
    println!("WaitNamedPipeW(2000) on absent pipe: {:?} in {:?}", pipe::wait_named_pipe_raw(&name, 2000), t.elapsed());
}

fn access(name: &str) {
    for (label, acc) in [("GENERIC_READ|GENERIC_WRITE", pipe::CLIENT_ACCESS_GENERIC), ("FILE_GENERIC_READ|FILE_WRITE_DATA", pipe::CLIENT_ACCESS_MIN)] {
        match pipe::open(name, acc, true, Duration::from_secs(2)) {
            Ok(h) => {
                let c = Conn::new(h);
                let never = pipe::new_event(true);
                let hello = text(c.read_frame(&never));
                c.write_frame(b"subscribe", 1000).unwrap();
                println!("{label}: open OK, hello={hello}, echo={}", text(c.read_frame(&never)));
            }
            Err(e) => println!("{label}: {e:?}"),
        }
    }
}

fn remote(name: &str) {
    use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_MODE, OPEN_EXISTING};
    let path = windows::core::HSTRING::from(format!(r"\\localhost\pipe\{name}"));
    // SAFETY: valid path string.
    let r = unsafe { CreateFileW(&path, pipe::CLIENT_ACCESS_GENERIC, FILE_SHARE_MODE(0), None, OPEN_EXISTING, pipe::flags_none(), None) };
    match r {
        Ok(h) => {
            let _h = OwnedHandle(h);
            println!("\\\\localhost\\pipe\\{name}: connected (remote clients NOT rejected)");
        }
        Err(e) => println!("\\\\localhost\\pipe\\{name}: {:?}", e.code()),
    }
}

fn selftest() {
    let name = format!("OpenMonitorAdvanced.Sensors.test-{}", std::process::id());
    let inst = fake::create_instance(&name, fake::TIGHT_SDDL, true).expect("create first instance");
    println!("fake server: first instance created (tight SDDL, FIRST_PIPE_INSTANCE)");
    println!(
        "fake server: 2nd instance by same unprivileged process -> {:?}",
        fake::create_instance(&name, fake::TIGHT_SDDL, false).err()
    );
    println!(
        "fake server: FIRST_PIPE_INSTANCE again -> {:?}",
        fake::create_instance(&name, fake::TIGHT_SDDL, true).err()
    );
    let srv = std::thread::spawn(move || fake::serve(inst, 3));
    let conn = connect(&name).expect("connect to fake");
    let pid = pipe::server_pid(&conn.h).unwrap();
    println!("fake server pid {pid} == own pid {}: {}", std::process::id(), pid == std::process::id());
    let never = pipe::new_event(true);
    println!("hello: {}", text(conn.read_frame(&never)));
    conn.write_frame(b"subscribe", 1000).unwrap();
    for _ in 0..4 {
        println!("frame: {}", text(conn.read_frame(&never)));
    }
    conn.write_frame(b"bye", 1000).unwrap();
    println!("after fake server disconnect: {}", text(conn.read_frame(&never)));
    drop(conn);
    std::thread::sleep(Duration::from_millis(100)); // let the fake re-listen
    remote(&name); // PIPE_REJECT_REMOTE_CLIENTS is set on this instance
    // Same instance reused (DisconnectNamedPipe + ConnectNamedPipe); minimal access mask.
    let h = pipe::open(&name, pipe::CLIENT_ACCESS_MIN, true, Duration::from_secs(2)).expect("min access");
    let c = Conn::new(h);
    println!("FILE_GENERIC_READ|FILE_WRITE_DATA client: hello={}", text(c.read_frame(&never)));
    c.write_frame(b"bye", 1000).unwrap();
    let _ = c.read_frame(&never);
    drop(srv); // detached; process exit ends it
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let name = a.get(2).cloned().unwrap_or_default();
    match a.get(1).map(String::as_str) {
        Some("scm") => scm::run(),
        Some("absent") => absent(),
        Some("client") => client(&name, a[3].parse().unwrap()),
        Some("busy") => busy(&name),
        Some("sync") => synctest::run(&name),
        Some("access") => access(&name),
        Some("remote") => remote(&name),
        Some("killwatch") => killwatch(&name),
        Some("selftest") => selftest(),
        _ => eprintln!("usage: scm|absent|selftest|client <name> <pid>|busy|sync|access|remote|killwatch <name>"),
    }
}
