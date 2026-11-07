//! Running as a Windows service: the service manager starts the process (on demand: a client starts it), the process
//! serves the pipe (server.rs), and stops itself after a minute with no client, so it runs only around profiles.

use std::{
    ffi::c_void,
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use crate::{
    etw,
    ffi::{self, Sid},
    protocol, server,
};

static ALLOWED: OnceLock<Sid> = OnceLock::new();
static STOP: AtomicUsize = AtomicUsize::new(0);
static STATUS: AtomicUsize = AtomicUsize::new(0);

const SERVICE_WIN32_OWN_PROCESS: u32 = 0x10;
const SERVICE_STOPPED: u32 = 1;
const SERVICE_START_PENDING: u32 = 2;
const SERVICE_STOP_PENDING: u32 = 3;
const SERVICE_RUNNING: u32 = 4;
const SERVICE_ACCEPT_STOP: u32 = 1;
const SERVICE_ACCEPT_SHUTDOWN: u32 = 4;
const SERVICE_CONTROL_STOP: u32 = 1;
const SERVICE_CONTROL_INTERROGATE: u32 = 4;
const SERVICE_CONTROL_SHUTDOWN: u32 = 5;
const IDLE: std::time::Duration = std::time::Duration::from_secs(60);

fn set_status(state: u32, exit_code: u32) {
    let handle = STATUS.load(Ordering::Acquire) as ffi::Handle;
    if handle.is_null() {
        return;
    }
    let mut status = ffi::ServiceStatus {
        service_type: SERVICE_WIN32_OWN_PROCESS,
        current_state: state,
        controls_accepted: if state == SERVICE_RUNNING { SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN } else { 0 },
        win32_exit_code: exit_code,
        wait_hint: if state == SERVICE_RUNNING || state == SERVICE_STOPPED { 0 } else { 10_000 },
        ..Default::default()
    };
    unsafe { ffi::SetServiceStatus(handle, &mut status) };
}

unsafe extern "system" fn control(code: u32, _event_type: u32, _event_data: *mut c_void, _context: *mut c_void) -> u32 {
    match code {
        SERVICE_CONTROL_STOP | SERVICE_CONTROL_SHUTDOWN => {
            set_status(SERVICE_STOP_PENDING, 0);
            let stop = STOP.load(Ordering::Acquire) as ffi::Handle;
            if !stop.is_null() {
                unsafe { ffi::SetEvent(stop) };
            }
            0
        }
        SERVICE_CONTROL_INTERROGATE => 0,
        _ => ffi::ERROR_CALL_NOT_IMPLEMENTED,
    }
}

unsafe extern "system" fn service_main(_argc: u32, _argv: *mut *mut u16) {
    let name = ffi::wide(protocol::SERVICE_NAME);
    let handle = unsafe { ffi::RegisterServiceCtrlHandlerExW(name.as_ptr(), Some(control), std::ptr::null_mut()) };
    if handle.is_null() {
        return;
    }
    STATUS.store(handle as usize, Ordering::Release);
    set_status(SERVICE_START_PENDING, 0);
    let stop = unsafe { ffi::CreateEventW(std::ptr::null_mut(), 1, 0, std::ptr::null()) };
    STOP.store(stop as usize, Ordering::Release);
    let Some(allowed) = ALLOWED.get() else {
        set_status(SERVICE_STOPPED, 87);
        return;
    };
    set_status(SERVICE_RUNNING, 0);
    // (the profiling right, enabled: a kernel session's checks need it on, and wpr.exe inherits it)
    ffi::enable_privilege("SeSystemProfilePrivilege");
    // (a session a crash left would count on for nobody)
    etw::stop_stale(etw::SESSION_NAME);
    let result = server::run(allowed, protocol::PIPE_NAME, etw::SESSION_NAME, stop, Some(IDLE), &|_line| {});
    set_status(SERVICE_STOPPED, if result.is_ok() { 0 } else { 1 });
}

/// Hands the process to the service manager (returns when the service has stopped)
pub fn run(allowed: Sid) -> Result<(), String> {
    let _ = ALLOWED.set(allowed);
    let mut name = ffi::wide(protocol::SERVICE_NAME);
    let table = [ffi::ServiceTableEntry { name: name.as_mut_ptr(), main: Some(service_main) }, ffi::ServiceTableEntry { name: std::ptr::null_mut(), main: None }];
    if unsafe { ffi::StartServiceCtrlDispatcherW(table.as_ptr()) } == 0 {
        let error = unsafe { ffi::GetLastError() };
        return Err(if error == ffi::ERROR_FAILED_SERVICE_CONTROLLER_CONNECT {
            "`service` is what the service manager runs; in a console, run `ree-hw-counters serve`".into()
        } else {
            format!("the service manager: error {error}")
        });
    }
    Ok(())
}
