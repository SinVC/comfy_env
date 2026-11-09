use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_INVALID_PARAMETER,
    FILETIME, GetLastError, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Console::{
    ATTACH_PARENT_PROCESS, AttachConsole, CTRL_BREAK_EVENT, CTRL_C_EVENT, FreeConsole,
    GenerateConsoleCtrlEvent, GetConsoleCP, SetConsoleCtrlHandler,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation, OpenJobObjectW,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NEW_PROCESS_GROUP, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
};

use super::ProcessIdentity;

// Job-specific access masks defined by WinNT.h.
const JOB_OBJECT_QUERY: u32 = 0x0004;
const JOB_OBJECT_TERMINATE: u32 = 0x0008;

pub fn check_process_alive(pid: u32) -> io::Result<bool> {
    match open_process(pid, PROCESS_SYNCHRONIZE)? {
        Some(process) => Ok(!wait_for_exit(&process, 0)?),
        None => Ok(false),
    }
}

pub fn process_creation(pid: u32) -> io::Result<Option<String>> {
    open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION)?
        .as_ref()
        .map(handle_creation)
        .transpose()
}

fn handle_creation(process: &impl AsRawHandle) -> io::Result<String> {
    let mut times = [FILETIME::default(); 4];
    let [created, exited, kernel, user] = &mut times;
    if unsafe { GetProcessTimes(process.as_raw_handle(), created, exited, kernel, user) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(format!(
        "{:08x}{:08x}",
        created.dwHighDateTime, created.dwLowDateTime
    ))
}

pub fn terminate_process(identity: &ProcessIdentity, grace_seconds: u32) -> io::Result<()> {
    let pid = identity.pid;
    let job = open_job(identity, JOB_OBJECT_QUERY | JOB_OBJECT_TERMINATE)?;
    let process = match verified_process(identity, PROCESS_SYNCHRONIZE | PROCESS_TERMINATE) {
        Err(error) if job.is_some() && error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => {
            None
        }
        result => result?,
    };
    let verified = process.as_ref();
    if let Some(process) = verified
        && !wait_for_exit(process, 0)?
    {
        request_console_break(pid);
    }
    let grace = Duration::from_secs(grace_seconds.into());
    let exited = match &job {
        Some(job) => wait_job(job, grace)?,
        None => match verified {
            Some(process) => wait_for_exit(process, grace_seconds.saturating_mul(1000))?,
            None => true,
        },
    };
    if exited {
        return Ok(());
    }
    // Hold the same process handle throughout stop so a recycled PID can't
    // redirect the forced termination.
    if let Some(process) = verified {
        if unsafe { TerminateProcess(process.as_raw_handle(), 1) } == 0 {
            let error = io::Error::last_os_error();
            if !wait_for_exit(process, 0)? {
                return Err(error);
            }
        }
        if job.is_none() && !wait_for_exit(process, 5000)? {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "process did not exit after TerminateProcess",
            ));
        }
    }
    if let Some(job) = job {
        terminate_job(&job)?;
    }
    Ok(())
}

pub struct CandidateProcess {
    pub child: Child,
    job: OwnedHandle,
}

impl CandidateProcess {
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        let mut child = command.creation_flags(CREATE_NEW_PROCESS_GROUP).spawn()?;
        match assign_job(&child) {
            Ok(job) => Ok(Self { child, job }),
            Err(error) => {
                let _ = child.kill();
                child.wait()?;
                Err(error)
            }
        }
    }

    pub fn has_exited(&mut self) -> io::Result<bool> {
        Ok(self.child.try_wait()?.is_some())
    }

    pub fn terminate(&mut self) -> io::Result<()> {
        terminate_job(&self.job)
    }
}

fn assign_job(child: &Child) -> io::Result<OwnedHandle> {
    let identity = ProcessIdentity {
        pid: child.id(),
        creation: handle_creation(child)?,
    };
    let name = job_name(&identity);
    let job = unsafe { CreateJobObjectW(std::ptr::null(), name.as_ptr()) };
    if job.is_null() {
        return Err(io::Error::last_os_error());
    }
    let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    let job = unsafe { OwnedHandle::from_raw_handle(job) };
    if already_exists {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "process job name already exists",
        ));
    }
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limits) as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // The trusted launcher waits for its start gate before running code.
    if unsafe { AssignProcessToJobObject(job.as_raw_handle(), child.as_raw_handle()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(job)
}

fn job_name(identity: &ProcessIdentity) -> Vec<u16> {
    format!("Local\\gov-process-{}-{}", identity.pid, identity.creation)
        .encode_utf16()
        .chain([0])
        .collect()
}

fn open_job(identity: &ProcessIdentity, access: u32) -> io::Result<Option<OwnedHandle>> {
    if identity.creation.len() != 16
        || !identity
            .creation
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Ok(None);
    }
    let name = job_name(identity);
    let job = unsafe { OpenJobObjectW(access, 0, name.as_ptr()) };
    if !job.is_null() {
        return Ok(Some(unsafe { OwnedHandle::from_raw_handle(job) }));
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_FILE_NOT_FOUND as i32) {
        return Ok(None);
    }
    Err(error)
}

fn wait_job(job: &OwnedHandle, timeout: Duration) -> io::Result<bool> {
    let start = Instant::now();
    loop {
        let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        if unsafe {
            QueryInformationJobObject(
                job.as_raw_handle(),
                JobObjectBasicAccountingInformation,
                (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                std::mem::size_of_val(&info) as u32,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if info.ActiveProcesses == 0 {
            return Ok(true);
        }
        let remaining = timeout.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            return Ok(false);
        }
        std::thread::sleep(remaining.min(Duration::from_millis(20)));
    }
}

fn terminate_job(job: &OwnedHandle) -> io::Result<()> {
    if unsafe { TerminateJobObject(job.as_raw_handle(), 1) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if !wait_job(job, Duration::from_secs(5))? {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "process job is still active",
        ));
    }
    Ok(())
}

pub fn candidate_is_alive(identity: &ProcessIdentity) -> anyhow::Result<bool> {
    if let Some(job) = open_job(identity, JOB_OBJECT_QUERY)? {
        return Ok(!wait_job(&job, Duration::ZERO)?);
    }
    identity.is_alive()
}

pub fn verified_is_alive(identity: &ProcessIdentity) -> io::Result<bool> {
    match verified_process(identity, PROCESS_SYNCHRONIZE)? {
        Some(process) => Ok(!wait_for_exit(&process, 0)?),
        None => Ok(false),
    }
}

fn verified_process(identity: &ProcessIdentity, access: u32) -> io::Result<Option<OwnedHandle>> {
    if process_creation(identity.pid)?.as_ref() != Some(&identity.creation) {
        return Ok(None);
    }
    let process = open_process(identity.pid, access | PROCESS_QUERY_LIMITED_INFORMATION)?;
    if let Some(process) = process
        && handle_creation(&process)? == identity.creation
    {
        return Ok(Some(process));
    }
    Ok(None)
}

fn open_process(pid: u32, access: u32) -> io::Result<Option<OwnedHandle>> {
    let handle = unsafe { OpenProcess(access, 0, pid) };
    if !handle.is_null() {
        // OpenProcess returned a new handle owned by the caller.
        return Ok(Some(unsafe { OwnedHandle::from_raw_handle(handle) }));
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
        return Ok(None);
    }
    Err(error)
}

fn wait_for_exit(process: &OwnedHandle, millis: u32) -> io::Result<bool> {
    match unsafe { WaitForSingleObject(process.as_raw_handle(), millis) } {
        WAIT_OBJECT_0 => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        _ => Err(io::Error::last_os_error()),
    }
}

fn request_console_break(pid: u32) {
    // Stop can be invoked from a different terminal. Ctrl+Break only reaches
    // the group when the sender is attached to the target's console.
    unsafe {
        let had_console = GetConsoleCP() != 0;
        if FreeConsole() == 0 {
            return;
        }
        if AttachConsole(pid) != 0 {
            GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid);
            FreeConsole();
        }
        if had_console {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

pub fn keep_launcher_alive_on_break() -> io::Result<()> {
    unsafe extern "system" fn ignore_break(event: u32) -> i32 {
        (event == CTRL_BREAK_EVENT) as i32
    }
    if unsafe { SetConsoleCtrlHandler(Some(ignore_break), 1) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn exit_code(status: ExitStatus) -> i32 {
    status
        .code()
        .expect("Windows process status always has an exit code")
}

static RUN_PID: AtomicU32 = AtomicU32::new(0);
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn forward_interrupt(event: u32) -> i32 {
    let pid = RUN_PID.load(Ordering::Relaxed);
    if event == CTRL_C_EVENT {
        INTERRUPTED.store(true, Ordering::Relaxed);
        if pid != 0 {
            unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid) };
        }
        return 1;
    }
    0
}

pub struct RunInterrupts;

impl RunInterrupts {
    pub fn install() -> io::Result<Self> {
        INTERRUPTED.store(false, Ordering::Relaxed);
        if unsafe { SetConsoleCtrlHandler(Some(forward_interrupt), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self)
    }

    pub fn set_child(&self, pid: u32) {
        RUN_PID.store(pid, Ordering::Relaxed);
    }

    pub fn interrupted(&self) -> bool {
        INTERRUPTED.load(Ordering::Relaxed)
    }
}

impl Drop for RunInterrupts {
    fn drop(&mut self) {
        RUN_PID.store(0, Ordering::Relaxed);
        unsafe { SetConsoleCtrlHandler(Some(forward_interrupt), 0) };
    }
}
