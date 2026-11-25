use std::process::Child;

use crate::Win32Error;

/// RAII wrapper around a Win32 Job Object configured with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
///
/// Used by Spike W1 and Phase 04 sidecar supervision so child processes (`llama-server`,
/// `whisper`, etc.) are guaranteed to terminate if the host exits or crashes.
#[derive(Debug)]
pub struct JobObject {
    #[cfg(windows)]
    handle: windows::Win32::Foundation::HANDLE,
    memory_limit_bytes: Option<usize>,
}

// SAFETY: A Win32 Job Object kernel handle is process-wide and safe to send/drop across threads.
unsafe impl Send for JobObject {}
// SAFETY: Operations on `JobObject` take `&self` and forward to thread-safe Win32 kernel APIs.
unsafe impl Sync for JobObject {}

impl JobObject {
    /// Creates a new anonymous Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`
    /// and an optional per-process memory cap in bytes.
    pub fn new(memory_limit_bytes: Option<usize>) -> Result<Self, Win32Error> {
        #[cfg(windows)]
        {
            windows_impl::create(memory_limit_bytes)
        }
        #[cfg(not(windows))]
        {
            Ok(Self { memory_limit_bytes })
        }
    }

    #[must_use]
    pub const fn memory_limit_bytes(&self) -> Option<usize> {
        self.memory_limit_bytes
    }

    /// Assigns an existing child process handle/PID to this Job Object.
    pub fn assign_child(&self, child: &Child) -> Result<(), Win32Error> {
        self.assign_process_id(child.id())
    }

    /// Assigns a running process by PID to this Job Object.
    pub fn assign_process_id(&self, pid: u32) -> Result<(), Win32Error> {
        #[cfg(windows)]
        {
            windows_impl::assign_pid(self.handle, pid)
        }
        #[cfg(not(windows))]
        {
            let _ = pid;
            Ok(())
        }
    }
}

#[cfg(windows)]
impl Drop for JobObject {
    fn drop(&mut self) {
        if !self.handle.is_invalid() {
            // SAFETY: `self.handle` is owned exclusively by this `JobObject` and closed once on drop.
            let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.handle) };
        }
    }
}

#[cfg(windows)]
mod windows_impl {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};
    use windows::core::PCWSTR;

    use super::JobObject;
    use crate::Win32Error;

    pub(super) fn create(memory_limit_bytes: Option<usize>) -> Result<JobObject, Win32Error> {
        // SAFETY: Creating an anonymous job object with null security attributes and null name.
        let handle = unsafe { CreateJobObjectW(None, PCWSTR::null()) }
            .map_err(|e| Win32Error::api("CreateJobObjectW", e.to_string()))?;

        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if let Some(limit) = memory_limit_bytes {
            info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
            info.ProcessMemoryLimit = limit;
        }

        let size = u32::try_from(std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
            .map_err(|e| Win32Error::InvalidArgument(e.to_string()))?;

        // SAFETY: `handle` is valid and `info` is a properly initialized `JOBOBJECT_EXTENDED_LIMIT_INFORMATION`.
        let set_res = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                size,
            )
        };
        if let Err(err) = set_res {
            // SAFETY: Clean up `handle` before returning error.
            let _ = unsafe { CloseHandle(handle) };
            return Err(Win32Error::api("SetInformationJobObject", err.to_string()));
        }

        Ok(JobObject {
            handle,
            memory_limit_bytes,
        })
    }

    pub(super) fn assign_pid(job: HANDLE, pid: u32) -> Result<(), Win32Error> {
        // SAFETY: Opening process handle with minimum rights required by `AssignProcessToJobObject`.
        let proc_handle = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid) }
            .map_err(|e| Win32Error::api("OpenProcess", e.to_string()))?;

        // SAFETY: Both `job` and `proc_handle` are valid kernel handles.
        let assign_res = unsafe { AssignProcessToJobObject(job, proc_handle) };
        // SAFETY: Always close the temporary process handle.
        let _ = unsafe { CloseHandle(proc_handle) };

        assign_res.map_err(|e| Win32Error::api("AssignProcessToJobObject", e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_object_constructs_with_optional_memory_limit() {
        let job = JobObject::new(Some(256 * 1024 * 1024)).expect("create job object");
        assert_eq!(job.memory_limit_bytes(), Some(256 * 1024 * 1024));
    }

    #[cfg(windows)]
    #[test]
    fn dropping_job_object_kills_assigned_child_process() {
        use std::process::Command;
        use std::time::{Duration, Instant};

        let job = JobObject::new(None).expect("create job object");
        let mut child = Command::new("cmd")
            .args(["/C", "ping", "-n", "15", "127.0.0.1", ">NUL"])
            .spawn()
            .expect("spawn benign child");
        job.assign_child(&child).expect("assign child to job");
        drop(job);

        let deadline = Instant::now() + Duration::from_secs(3);
        let mut exited = false;
        while Instant::now() < deadline {
            if child.try_wait().expect("try_wait").is_some() {
                exited = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if !exited {
            let _ = child.kill();
        }
        assert!(
            exited,
            "child process should terminate when JobObject drops"
        );
    }
}
