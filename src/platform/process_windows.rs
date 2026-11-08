use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicU32, Ordering};

use windows_sys::Win32::Foundation::{ERROR_INVALID_PARAMETER, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Console::{
    ATTACH_PARENT_PROCESS, AttachConsole, CTRL_BREAK_EVENT, CTRL_C_EVENT, FreeConsole,
    GenerateConsoleCtrlEvent, GetConsoleCP, SetConsoleCtrlHandler,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NEW_PROCESS_GROUP, OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    TerminateProcess, WaitForSingleObject,
};

pub fn check_process_alive(pid: u32) -> io::Result<bool> {
    match open_process(pid, PROCESS_SYNCHRONIZE)? {
        Some(process) => Ok(!wait_for_exit(&process, 0)?),
        None => Ok(false),
    }
}

pub fn terminate_process(pid: u32, grace_seconds: u32) -> io::Result<()> {
    let Some(process) = open_process(pid, PROCESS_SYNCHRONIZE | PROCESS_TERMINATE)? else {
        return Ok(());
    };
    if wait_for_exit(&process, 0)? {
        return Ok(());
    }
    request_console_break(pid);
    if wait_for_exit(&process, grace_seconds * 1000)? {
        return Ok(());
    }
    // Hold the same process handle throughout stop so a recycled PID can't
    // redirect the forced termination.
    if unsafe { TerminateProcess(process.as_raw_handle(), 1) } == 0 {
        let error = io::Error::last_os_error();
        if !wait_for_exit(&process, 0)? {
            return Err(error);
        }
    }
    if wait_for_exit(&process, 5000)? {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "process did not exit after TerminateProcess",
    ))
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

pub fn configure_prod_process(command: &mut Command) {
    command.creation_flags(CREATE_NEW_PROCESS_GROUP);
}

pub fn exit_code(status: ExitStatus) -> i32 {
    status
        .code()
        .expect("Windows process status always has an exit code")
}

static RUN_PID: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn forward_interrupt(event: u32) -> i32 {
    let pid = RUN_PID.load(Ordering::Relaxed);
    if event == CTRL_C_EVENT && pid != 0 {
        return unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid) };
    }
    0
}

pub struct RunInterrupts;

impl RunInterrupts {
    pub fn install() -> io::Result<Self> {
        if unsafe { SetConsoleCtrlHandler(Some(forward_interrupt), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self)
    }

    pub fn set_child(&self, pid: u32) {
        RUN_PID.store(pid, Ordering::Relaxed);
    }
}

impl Drop for RunInterrupts {
    fn drop(&mut self) {
        RUN_PID.store(0, Ordering::Relaxed);
        unsafe { SetConsoleCtrlHandler(Some(forward_interrupt), 0) };
    }
}
