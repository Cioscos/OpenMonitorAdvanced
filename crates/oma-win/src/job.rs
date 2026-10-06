//! A Job Object that kills its processes when the last handle closes (M8a1).
//!
//! The app assigns `oma-load.exe` to it, so the helper dies with the app even
//! when the app is killed (the handle closes with the process).

use std::io;
use std::os::windows::io::AsRawHandle;
use std::process::Child;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

use crate::overlay_pipe::os_error;
use crate::pipe_io::OwnedHandle;

/// A Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Dropping it closes
/// the handle, which kills every process assigned to it.
pub struct KillOnCloseJob(OwnedHandle);

impl KillOnCloseJob {
    pub fn new() -> io::Result<Self> {
        // SAFETY: no security attributes and no name; the handle is owned right away.
        let h = unsafe { CreateJobObjectW(None, None) }.map_err(|e| os_error(&e))?;
        let job = Self(OwnedHandle(h));
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `job.0` is a live job handle; `info` is a live
        // JOBOBJECT_EXTENDED_LIMIT_INFORMATION of the size passed, which the call only reads.
        unsafe {
            SetInformationJobObject(
                job.0 .0,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }
        .map_err(|e| os_error(&e))?;
        Ok(job)
    }

    /// Puts `child` in the job.
    pub fn assign(&self, child: &Child) -> io::Result<()> {
        let process = HANDLE(child.as_raw_handle());
        // SAFETY: both handles are live: the job is owned by `self`, the process handle by `child`.
        unsafe { AssignProcessToJobObject(self.0 .0, process) }.map_err(|e| os_error(&e))
    }
}

#[cfg(test)]
mod tests {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use std::time::{Duration, Instant};

    use super::*;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    #[test]
    fn killing_the_job_kills_the_child() {
        // A sleeping child (ping), no CPU load.
        let mut child = Command::new("cmd.exe")
            .args(["/c", "ping -n 30 127.0.0.1 >nul"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .expect("spawn cmd");
        let job = KillOnCloseJob::new().expect("create the job");
        job.assign(&child).expect("assign the child");
        assert!(child.try_wait().unwrap().is_none(), "still running");
        drop(job);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if child.try_wait().unwrap().is_some() {
                return;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                panic!("the child outlived its job");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
