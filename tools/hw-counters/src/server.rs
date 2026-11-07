//! The service's pipe: one client at a time, from the one account it was installed for (the pipe's ACL grants only that
//! account and SYSTEM, and the client process's token is checked again). A client asks to count some of the CPU's
//! counters and gets back only its own process's threads' slices; the session stops when the client says so, goes
//! away, or has counted for 30 minutes.

use std::ffi::c_void;

use crate::{
    etw,
    ffi::{self, Owned, Sid},
    protocol::{self, Reply, Request},
    register,
};

/// How long a session may count, and how long a client may take to ask
const MAX_SESSION: std::time::Duration = std::time::Duration::from_secs(30 * 60);
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Serves clients on `pipe_name` until `stop` is set or no client came for `idle`; `session` names the trace session
pub fn run(allowed: &Sid, pipe_name: &str, session: &str, stop: ffi::Handle, idle: Option<std::time::Duration>, log: &dyn Fn(&str)) -> Result<(), String> {
    loop {
        let pipe = create_pipe(pipe_name, allowed)?;
        match wait_for_client(&pipe, stop, idle)? {
            false => return Ok(()),
            true => {
                let outcome = serve(&pipe, allowed, session, stop);
                match &outcome {
                    Ok(summary) => log(summary),
                    Err(error) => log(&format!("a client: {error}")),
                }
                unsafe {
                    ffi::FlushFileBuffers(pipe.0);
                    ffi::DisconnectNamedPipe(pipe.0);
                }
            }
        }
    }
}

/// The pipe, for `allowed` (and SYSTEM) only, local clients only, one instance (a process that took the name first
/// makes this fail rather than serve beside it)
fn create_pipe(name: &str, allowed: &Sid) -> Result<Owned, String> {
    // FILE_READ_DATA | FILE_WRITE_DATA | FILE_READ_ATTRIBUTES | READ_CONTROL | SYNCHRONIZE for the account: what a client
    // needs, and not FILE_CREATE_PIPE_INSTANCE
    let sddl = ffi::wide(&format!("D:P(A;;GA;;;SY)(A;;0x120083;;;{})", allowed.text()));
    let mut descriptor: *mut c_void = std::ptr::null_mut();
    if unsafe { ffi::ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut descriptor, std::ptr::null_mut()) } == 0 {
        return Err(format!("the pipe's security: error {}", unsafe { ffi::GetLastError() }));
    }
    let mut attributes = ffi::SecurityAttributes { length: std::mem::size_of::<ffi::SecurityAttributes>() as u32, descriptor, inherit: 0 };
    const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
    const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
    const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
    const PIPE_REJECT_REMOTE_CLIENTS: u32 = 0x0000_0008;
    let wide = ffi::wide(name);
    let pipe = unsafe { ffi::CreateNamedPipeW(wide.as_ptr(), PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_REJECT_REMOTE_CLIENTS, 1, 1 << 16, 1 << 16, 0, &mut attributes) };
    let error = unsafe { ffi::GetLastError() };
    unsafe { ffi::LocalFree(descriptor) };
    if pipe == ffi::INVALID_HANDLE {
        return Err(format!("creating the pipe {name}: error {error} (another process holds the name?)"));
    }
    Ok(Owned(pipe))
}

/// An event for one overlapped operation
fn event() -> Result<Owned, String> {
    let event = unsafe { ffi::CreateEventW(std::ptr::null_mut(), 1, 0, std::ptr::null()) };
    if event.is_null() { Err(format!("an event: error {}", unsafe { ffi::GetLastError() })) } else { Ok(Owned(event)) }
}

/// Waits for an overlapped operation on `pipe`, or `stop`, or the timeout: the bytes moved, or None when it was given up
fn finish(pipe: &Owned, overlapped: &mut ffi::Overlapped, started: i32, stop: ffi::Handle, timeout: Option<std::time::Duration>) -> Result<Option<u32>, String> {
    let mut moved = 0u32;
    if started != 0 {
        unsafe { ffi::GetOverlappedResult(pipe.0, overlapped, &mut moved, 0) };
        return Ok(Some(moved));
    }
    match unsafe { ffi::GetLastError() } {
        ffi::ERROR_IO_PENDING => {}
        ffi::ERROR_PIPE_CONNECTED => return Ok(Some(0)),
        error => return Err(format!("error {error}")),
    }
    let handles = [overlapped.event, stop];
    let milliseconds = timeout.map_or(ffi::INFINITE, |timeout| timeout.as_millis().min(u32::MAX as u128 - 1) as u32);
    let waited = unsafe { ffi::WaitForMultipleObjects(2, handles.as_ptr(), 0, milliseconds) };
    if waited == ffi::WAIT_OBJECT_0 {
        if unsafe { ffi::GetOverlappedResult(pipe.0, overlapped, &mut moved, 0) } == 0 {
            return Err(format!("error {}", unsafe { ffi::GetLastError() }));
        }
        return Ok(Some(moved));
    }
    // (stopped or timed out: the operation is cancelled and waited for, so its buffer is not written later)
    unsafe {
        ffi::CancelIoEx(pipe.0, overlapped);
        ffi::GetOverlappedResult(pipe.0, overlapped, &mut moved, 1);
    }
    Ok(None)
}

/// Whether a client connected (false: stopped, or idle for `idle`)
fn wait_for_client(pipe: &Owned, stop: ffi::Handle, idle: Option<std::time::Duration>) -> Result<bool, String> {
    let event = event()?;
    let mut overlapped = ffi::Overlapped { internal: 0, internal_high: 0, offset: 0, offset_high: 0, event: event.0 };
    let started = unsafe { ffi::ConnectNamedPipe(pipe.0, &mut overlapped) };
    Ok(finish(pipe, &mut overlapped, started, stop, idle).map_err(|error| format!("waiting for a client: {error}"))?.is_some())
}

/// Reads from the pipe with a deadline (std::io::Read for protocol::read_message)
struct Reader<'a> {
    pipe: &'a Owned,
    stop: ffi::Handle,
    deadline: std::time::Instant,
}

impl std::io::Read for Reader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let event = event().map_err(std::io::Error::other)?;
        let mut overlapped = ffi::Overlapped { internal: 0, internal_high: 0, offset: 0, offset_high: 0, event: event.0 };
        let wanted = buffer.len().min(1 << 16) as u32;
        let mut read = 0u32;
        let started = unsafe { ffi::ReadFile(self.pipe.0, buffer.as_mut_ptr(), wanted, &mut read, &mut overlapped as *mut ffi::Overlapped as *mut c_void) };
        let left = self.deadline.saturating_duration_since(std::time::Instant::now());
        match finish(self.pipe, &mut overlapped, started, self.stop, Some(left)) {
            Ok(Some(0)) => Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the client went away")),
            Ok(Some(read)) => Ok(read as usize),
            Ok(None) => Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "no request in time")),
            Err(error) => Err(std::io::Error::other(error)),
        }
    }
}

fn write(pipe: &Owned, stop: ffi::Handle, bytes: &[u8]) -> Result<(), String> {
    let mut sent = 0usize;
    while sent < bytes.len() {
        let event = event()?;
        let mut overlapped = ffi::Overlapped { internal: 0, internal_high: 0, offset: 0, offset_high: 0, event: event.0 };
        let chunk = (bytes.len() - sent).min(1 << 20) as u32;
        let mut written = 0u32;
        let started = unsafe { ffi::WriteFile(pipe.0, bytes[sent..].as_ptr(), chunk, &mut written, &mut overlapped as *mut ffi::Overlapped as *mut c_void) };
        match finish(pipe, &mut overlapped, started, stop, Some(std::time::Duration::from_secs(60)))? {
            Some(written) if written > 0 => sent += written as usize,
            _ => return Err("the client stopped reading".into()),
        }
    }
    Ok(())
}

fn reply(pipe: &Owned, stop: ffi::Handle, reply: &Reply) -> Result<(), String> {
    write(pipe, stop, &reply.encode())
}

/// The client's process, checked to run as the allowed account
fn client_process(pipe: &Owned, allowed: &Sid) -> Result<u32, String> {
    let mut process = 0u32;
    if unsafe { ffi::GetNamedPipeClientProcessId(pipe.0, &mut process) } == 0 {
        return Err(format!("the client's process: error {}", unsafe { ffi::GetLastError() }));
    }
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    let handle = Owned(unsafe { ffi::OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process) });
    if handle.0.is_null() {
        return Err(format!("opening the client's process {process}: error {}", unsafe { ffi::GetLastError() }));
    }
    let user = Sid::of_process(handle.0)?;
    if !user.equals(allowed) {
        return Err(format!("process {process} runs as {}, not the account the service serves", user.text()));
    }
    Ok(process)
}

/// One client: its request, the session, its results; a summary line for the log
fn serve(pipe: &Owned, allowed: &Sid, session_name: &str, stop: ffi::Handle) -> Result<String, String> {
    let process = match client_process(pipe, allowed) {
        Ok(process) => process,
        Err(error) => {
            let _ = reply(pipe, stop, &Reply::Error(format!("the counter service refused the client: {error}")));
            return Err(error);
        }
    };
    let mut reader = Reader { pipe, stop, deadline: std::time::Instant::now() + REQUEST_TIMEOUT };
    let (kind, body) = protocol::read_message(&mut reader, protocol::MAX_REQUEST_BYTES)?;
    let (version, counters, threads) = match Request::decode(kind, &body)? {
        Request::Start { version, counters, threads } => (version, counters, threads),
        Request::Stop => return Err("a stop before any start".into()),
    };
    match start(session_name, process, version, &counters, &threads) {
        Err(error) => {
            reply(pipe, stop, &Reply::Error(error.clone()))?;
            Err(error)
        }
        Ok((session, notes)) => {
            let frequency = {
                let mut frequency = 0i64;
                unsafe { ffi::QueryPerformanceFrequency(&mut frequency) };
                frequency as u64
            };
            reply(pipe, stop, &Reply::Started { version: protocol::VERSION, sources: counters.clone(), qpc_frequency: frequency, notes })?;
            // the client counts, then asks to stop (or goes away, and the session stops with it)
            let mut reader = Reader { pipe, stop, deadline: std::time::Instant::now() + MAX_SESSION };
            let request = protocol::read_message(&mut reader, protocol::MAX_REQUEST_BYTES).and_then(|(kind, body)| Request::decode(kind, &body));
            let results = session.stop();
            match (request, results) {
                (Ok(Request::Stop), Ok(results)) => {
                    let line = format!("counted {} for process {process}: {} threads, {} switches", counters.join(", "), results.threads.len(), results.switches);
                    reply(pipe, stop, &Reply::Results(results))?;
                    Ok(line)
                }
                (Ok(Request::Stop), Err(error)) => {
                    reply(pipe, stop, &Reply::Error(error.clone()))?;
                    Err(error)
                }
                (Ok(Request::Start { .. }), _) => {
                    let _ = reply(pipe, stop, &Reply::Error("a second start while counting".into()));
                    Err("a second start".into())
                }
                (Err(error), _) => {
                    let _ = reply(pipe, stop, &Reply::Error(format!("the counting stopped: {error}")));
                    Err(error)
                }
            }
        }
    }
}

/// Checks a request and starts its session
fn start(session_name: &str, process: u32, version: u32, counters: &[String], threads: &[u32]) -> Result<(etw::Session, Vec<String>), String> {
    if version != protocol::VERSION {
        return Err(format!("the counter service speaks version {}, the client {version}: build both from the same checkout", protocol::VERSION));
    }
    let max = etw::max_counters().unwrap_or(4).min(protocol::MAX_COUNTERS);
    if counters.is_empty() || counters.len() > max {
        return Err(format!("{} counters asked for: the CPU counts 1 to {max} at once", counters.len()));
    }
    for (index, name) in counters.iter().enumerate() {
        if counters[..index].contains(name) {
            return Err(format!("{name} asked for twice"));
        }
    }
    let mut notes = Vec::new();
    register::ensure(counters, &mut notes)?;
    let list = etw::sources()?;
    let ids: Vec<u32> = counters.iter().map(|name| list.iter().find(|(_, source, _, _)| source == name).map(|(id, _, _, _)| *id).ok_or_else(|| format!("the CPU offers no counter {name}"))).collect::<Result<_, _>>()?;
    // the threads the client named are its own (their handles are held while counting, so their ids stay theirs)
    let mut held = Vec::with_capacity(threads.len());
    for &thread in threads {
        held.push(etw::open_thread_of(process, thread).ok_or_else(|| format!("thread {thread} is not one of the client's"))?);
    }
    let session = etw::Session::start(session_name, &ids, process, threads, held, &mut notes)?;
    Ok((session, notes))
}
