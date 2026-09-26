//! SCM probes as a non-admin. Never starts or stops a real service (StartServiceW is only
//! called on RpcEptMapper, which is always running and cannot be stopped).
use windows::Win32::Foundation::WIN32_ERROR;
use windows::Win32::System::Services::{
    CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatusEx, SC_HANDLE,
    SC_MANAGER_CONNECT, SC_STATUS_PROCESS_INFO, SERVICE_CHANGE_CONFIG, SERVICE_QUERY_STATUS,
    SERVICE_START, SERVICE_STATUS_PROCESS, SERVICE_STOP, StartServiceW,
};
use windows::core::{HSTRING, PCWSTR};

const _: () = assert!(size_of::<SERVICE_STATUS_PROCESS>() == 36);

fn code(e: &windows::core::Error) -> WIN32_ERROR {
    WIN32_ERROR::from_error(e).unwrap_or(WIN32_ERROR(e.code().0 as u32))
}

pub struct ScHandle(pub SC_HANDLE);
impl Drop for ScHandle {
    fn drop(&mut self) {
        // SAFETY: we own the SCM/service handle and close it once.
        unsafe {
            let _ = CloseServiceHandle(self.0);
        }
    }
}

pub fn open_scm() -> Result<ScHandle, WIN32_ERROR> {
    // SAFETY: local machine, default database, connect right only.
    unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) }
        .map(ScHandle)
        .map_err(|e| code(&e))
}

pub fn open_service(scm: &ScHandle, name: &str, access: u32) -> Result<ScHandle, WIN32_ERROR> {
    let n = HSTRING::from(name);
    // SAFETY: live SCM handle, valid wide string.
    unsafe { OpenServiceW(scm.0, &n, access) }.map(ScHandle).map_err(|e| code(&e))
}

/// (state, pid) — pid is 0 when not running.
pub fn query(svc: &ScHandle) -> Result<(u32, u32), WIN32_ERROR> {
    let mut st = SERVICE_STATUS_PROCESS::default();
    let mut needed = 0u32;
    // SAFETY: the byte view covers exactly the struct, which outlives the call.
    let buf = unsafe {
        std::slice::from_raw_parts_mut(
            (&mut st as *mut SERVICE_STATUS_PROCESS).cast::<u8>(),
            size_of::<SERVICE_STATUS_PROCESS>(),
        )
    };
    // SAFETY: live service handle opened with SERVICE_QUERY_STATUS.
    unsafe { QueryServiceStatusEx(svc.0, SC_STATUS_PROCESS_INFO, Some(buf), &mut needed) }
        .map_err(|e| code(&e))?;
    Ok((st.dwCurrentState.0, st.dwProcessId))
}

fn show(label: &str, r: Result<ScHandle, WIN32_ERROR>) -> Option<ScHandle> {
    match r {
        Ok(h) => {
            println!("  {label}: OpenServiceW OK");
            Some(h)
        }
        Err(e) => {
            println!("  {label}: OpenServiceW failed, error {} ", e.0);
            None
        }
    }
}

pub fn run() {
    let t0 = std::time::Instant::now();
    let scm = open_scm().expect("OpenSCManagerW(SC_MANAGER_CONNECT)");
    println!("OpenSCManagerW(SC_MANAGER_CONNECT) OK in {:?}", t0.elapsed());

    show("missing service, QUERY_STATUS", open_service(&scm, "OpenMonitorAdvanced.NoSuchSvc", SERVICE_QUERY_STATUS));

    let t1 = std::time::Instant::now();
    if let Some(h) = show("Spooler QUERY_STATUS", open_service(&scm, "Spooler", SERVICE_QUERY_STATUS)) {
        println!("  Spooler status (state, pid) = {:?}  [open+query {:?}]", query(&h), t1.elapsed());
    }
    show("Spooler START", open_service(&scm, "Spooler", SERVICE_START));
    show("Spooler STOP", open_service(&scm, "Spooler", SERVICE_STOP));
    show("Spooler QUERY_STATUS|START", open_service(&scm, "Spooler", SERVICE_QUERY_STATUS | SERVICE_START));

    for name in ["EasyAntiCheat_EOS", "EABackgroundService"] {
        if let Some(h) = show(
            &format!("{name} QUERY_STATUS|START|STOP"),
            open_service(&scm, name, SERVICE_QUERY_STATUS | SERVICE_START | SERVICE_STOP),
        ) {
            println!("  {name} status (state, pid) = {:?} (handle closed, NOT started)", query(&h));
        }
        show(&format!("{name} CHANGE_CONFIG"), open_service(&scm, name, SERVICE_CHANGE_CONFIG));
    }

    // StartServiceW on an always-running service that grants BU the start right.
    if let Some(h) = show(
        "RpcEptMapper QUERY_STATUS|START",
        open_service(&scm, "RpcEptMapper", SERVICE_QUERY_STATUS | SERVICE_START),
    ) {
        let st = query(&h);
        println!("  RpcEptMapper status = {st:?}");
        if matches!(st, Ok((4, _))) {
            // SAFETY: live handle with SERVICE_START; the service is running, so this is a no-op.
            let r = unsafe { StartServiceW(h.0, None) };
            println!("  StartServiceW(RpcEptMapper, running) -> {:?}", r.map_err(|e| code(&e).0));
        }
    }
}
