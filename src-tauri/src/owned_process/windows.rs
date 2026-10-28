//! Windows ownership boundaries: private directories and a kill-on-close Job.
use super::{Failure, Result};
use std::{ffi::c_void, path::Path, ptr};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, LocalFree},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SDDL_REVISION_1,
        },
        GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    Storage::FileSystem::CreateDirectoryW,
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
        Threading::{
            GetCurrentProcess, OpenProcessToken, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
        },
    },
};

pub fn private_directory(path: &Path) -> Result<()> {
    let mut token = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(Failure::Unsafe);
    }
    let mut size = 0;
    unsafe {
        GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut size);
    }
    let mut buffer = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            size,
            &mut size,
        )
    };
    unsafe {
        CloseHandle(token);
    }
    if ok == 0 {
        return Err(Failure::Unsafe);
    }
    let user = unsafe { &*(buffer.as_ptr().cast::<TOKEN_USER>()) };
    let mut sid = ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid) } == 0 {
        return Err(Failure::Unsafe);
    }
    let mut len = 0;
    while unsafe { *sid.add(len) } != 0 {
        len += 1;
    }
    let sid_text = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(sid, len) });
    unsafe {
        LocalFree(sid.cast());
    }
    let text: Vec<u16> = format!("D:P(A;OICI;FA;;;{sid_text})")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut descriptor: *mut c_void = ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(Failure::Unsafe);
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let created = unsafe { CreateDirectoryW(wide.as_ptr(), &attributes) };
    unsafe {
        LocalFree(descriptor);
    }
    if created == 0 {
        return Err(Failure::Unsafe);
    }
    Ok(())
}

pub struct Job(HANDLE);
// The handle is owned, never exposed, and only used for job lifetime control.
unsafe impl Send for Job {}
impl Job {
    pub(crate) fn attach_suspended(child: &std::process::Child) -> Result<Self> {
        use std::os::windows::io::AsRawHandle;
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if handle.is_null() {
            return Err(Failure::Unsafe);
        }
        let job = Self(handle);
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of_val(&limits) as u32,
            )
        } == 0
            || unsafe { AssignProcessToJobObject(handle, child.as_raw_handle()) } == 0
        {
            return Err(Failure::Unsafe);
        }
        // Command creates the process suspended. Identify only that owned
        // process's initial thread, assign its Job first, then resume it. This
        // closes the spawn/assignment race for early helper descendants.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(Failure::Unsafe);
        }
        let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
        entry.dwSize = size_of::<THREADENTRY32>() as u32;
        let mut found = None;
        let mut next = unsafe { Thread32First(snapshot, &mut entry) };
        while next != 0 {
            if entry.th32OwnerProcessID == child.id() {
                if found.is_some() {
                    unsafe {
                        CloseHandle(snapshot);
                    }
                    return Err(Failure::Unsafe);
                }
                found = Some(entry.th32ThreadID);
            }
            next = unsafe { Thread32Next(snapshot, &mut entry) };
        }
        unsafe {
            CloseHandle(snapshot);
        }
        let id = found.ok_or(Failure::Unsafe)?;
        let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, id) };
        if thread.is_null() {
            return Err(Failure::Unsafe);
        }
        let resumed = unsafe { ResumeThread(thread) };
        unsafe {
            CloseHandle(thread);
        }
        if resumed != 1 {
            return Err(Failure::Unsafe);
        }
        Ok(job)
    }
    pub fn terminate(&self) {
        unsafe {
            TerminateJobObject(self.0, 1);
        }
    }
}

/// Starts an owned child suspended, places it in a kill-on-close Job, then
/// resumes it. This closes the window where the process could create an
/// untracked descendant before ownership is established.
pub(crate) fn spawn_in_job(
    command: &mut std::process::Command,
) -> std::io::Result<(std::process::Child, Job)> {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0800_0004); // CREATE_NO_WINDOW | CREATE_SUSPENDED
    let mut child = command.spawn()?;
    match Job::attach_suspended(&child) {
        Ok(job) => Ok((child, job)),
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(std::io::Error::other("owned child setup failed"))
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
