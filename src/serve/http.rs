//! Minimal HTTP/1.1 for the local service (as roblox-object-renderer's render service speaks it):
//! one request per connection, Content-Length bodies only.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    time::Duration,
};

pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
    }
}

const MAX_BODY: usize = 16 << 20;

pub fn read_request(stream: &TcpStream) -> Result<Request, String> {
    stream.set_read_timeout(Some(Duration::from_secs(30))).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| format!("request line: {e}"))?;
    let mut parts = line.split_whitespace();
    let method = parts.next().ok_or("no method")?.to_string();
    let path = parts.next().ok_or("no path")?.to_string();
    let mut headers = Vec::new();
    loop {
        line.clear();
        reader.read_line(&mut line).map_err(|e| format!("header: {e}"))?;
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some((key, value)) = trimmed.split_once(':') {
            headers.push((key.trim().to_string(), value.trim().to_string()));
        }
        if headers.len() > 100 {
            return Err("too many headers".into());
        }
    }
    let length: usize = headers.iter().find(|(key, _)| key.eq_ignore_ascii_case("content-length")).and_then(|(_, value)| value.parse().ok()).unwrap_or(0);
    if length > MAX_BODY {
        return Err("request body too large".into());
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).map_err(|e| format!("body: {e}"))?;
    Ok(Request { method, path, headers, body })
}

pub fn write_response(mut stream: &TcpStream, status: u16, body: &[u8]) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Status",
    };
    let head = format!("HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

/// Whether the client has closed the connection (asked only by the thread that writes the reply, before it writes)
pub fn closed(stream: &TcpStream) -> bool {
    if stream.set_nonblocking(true).is_err() {
        return false;
    }
    let mut byte = [0u8; 1];
    let closed = match stream.peek(&mut byte) {
        Ok(read) => read == 0,
        Err(error) => !matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted),
    };
    let _ = stream.set_nonblocking(false);
    closed
}

/// A request to the local service on `port`: the status and body.
pub fn request(port: u16, method: &str, path: &str, body: &[u8], timeout: Duration) -> Result<(u16, Vec<u8>), String> {
    let error = |e: std::io::Error| e.to_string();
    let mut stream = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(500)).map_err(|e| format!("connecting to 127.0.0.1:{port}: {e}"))?;
    stream.set_read_timeout(Some(timeout)).map_err(error)?;
    stream.set_nodelay(true).map_err(error)?;
    let head = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-REE: 1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    stream.write_all(head.as_bytes()).map_err(error)?;
    stream.write_all(body).map_err(error)?;
    stream.flush().map_err(error)?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(error)?;
    let status: u16 = line.split_whitespace().nth(1).and_then(|code| code.parse().ok()).ok_or_else(|| format!("bad status line {line:?}"))?;
    let mut length = None;
    loop {
        line.clear();
        reader.read_line(&mut line).map_err(error)?;
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some((key, value)) = trimmed.split_once(':') {
            if key.trim().eq_ignore_ascii_case("content-length") {
                length = value.trim().parse::<usize>().ok();
            }
        }
    }
    let mut body = Vec::new();
    match length {
        Some(length) => {
            body.resize(length, 0);
            reader.read_exact(&mut body).map_err(error)?;
        }
        None => {
            reader.read_to_end(&mut body).map_err(error)?;
        }
    }
    Ok((status, body))
}
