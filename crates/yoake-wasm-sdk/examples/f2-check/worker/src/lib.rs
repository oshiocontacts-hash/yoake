//! FR-REM05, worker: the network part of the F2 check. It runs on its own thread, so the blocking calls here
//! (a 2 s HTTP answer, a TCP echo) never stop the game's tick.
//!
//! The main instance sends the jobs (`worker::send`) and the worker answers each one with `host-send`; the mod
//! logs the answers from its tick hook. A job is a line `<id> <op> <args>`:
//! - `<id> post <url> <body>`: an HTTP POST of `<body>` (wasi:http), the status and the reply body;
//! - `<id> tcp <host:port> <text>`: a TCP connection, `<text>` sent, the echo read back until the peer closes;
//! - `quit`: the worker returns.
use std::io::{Read, Write};

wit_bindgen::generate!({
    path: "../../../../yoake-wasm/wit-worker",
    world: "worker-mod",
});

use wasip2::http::outgoing_handler;
use wasip2::http::types::{Fields, Method, OutgoingBody, OutgoingRequest, Scheme};
use wasip2::io::streams::StreamError;

struct Worker;

fn post(url: &str, body: &str) -> String {
    let Some(rest) = url.strip_prefix("http://") else {
        return "error:only-http".to_owned();
    };
    let (authority, path) = rest.split_once('/').map_or((rest, "/".to_owned()), |(a, p)| (a, format!("/{p}")));
    // a known length, so the request is not chunked: a plain HTTP/1.1 POST
    let headers = Fields::from_list(&[
        ("content-type".to_owned(), b"text/plain".to_vec()),
        ("content-length".to_owned(), body.len().to_string().into_bytes()),
    ])
    .unwrap_or_else(|_| Fields::new());
    let request = OutgoingRequest::new(headers);
    let _ = request.set_method(&Method::Post);
    let _ = request.set_scheme(Some(&Scheme::Http));
    let _ = request.set_authority(Some(authority));
    let _ = request.set_path_with_query(Some(&path));
    let Ok(outgoing) = request.body() else {
        return "error:no-body".to_owned();
    };
    {
        let Ok(stream) = outgoing.write() else {
            return "error:no-stream".to_owned();
        };
        if stream.blocking_write_and_flush(body.as_bytes()).is_err() {
            return "error:write".to_owned();
        }
    }
    if let Err(error) = OutgoingBody::finish(outgoing, None) {
        return format!("error:finish:{error:?}");
    }
    let future = match outgoing_handler::handle(request, None) {
        Ok(future) => future,
        Err(error) => return format!("error:handle:{error:?}"),
    };
    future.subscribe().block();
    let response = match future.get() {
        Some(Ok(Ok(response))) => response,
        other => return format!("error:response:{other:?}"),
    };
    let status = response.status();
    let mut reply = Vec::new();
    if let Ok(incoming) = response.consume() {
        if let Ok(stream) = incoming.stream() {
            loop {
                match stream.blocking_read(4096) {
                    Ok(chunk) => reply.extend(chunk),
                    Err(StreamError::Closed) => break,
                    Err(_) => break,
                }
            }
        }
    }
    format!("status={status} reply={}", String::from_utf8_lossy(&reply))
}

fn tcp(address: &str, text: &str) -> String {
    let mut stream = match std::net::TcpStream::connect(address) {
        Ok(stream) => stream,
        Err(error) => return format!("error:connect:{error}"),
    };
    if let Err(error) = stream.write_all(text.as_bytes()) {
        return format!("error:write:{error}");
    }
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut echo = Vec::new();
    match stream.read_to_end(&mut echo) {
        Ok(_) => format!("echo={}", String::from_utf8_lossy(&echo)),
        Err(error) => format!("error:read:{error}"),
    }
}

/// Does one job line: the answer, or `None` for the quit line.
fn job(line: &str) -> Option<String> {
    let mut words = line.splitn(4, ' ');
    let (Some(id), Some(op)) = (words.next(), words.next()) else {
        return Some("error:usage".to_owned());
    };
    let answer = match op {
        "post" => match (words.next(), words.next()) {
            (Some(url), Some(body)) => post(url, body),
            (Some(url), None) => post(url, ""),
            _ => "error:usage".to_owned(),
        },
        "tcp" => match (words.next(), words.next()) {
            (Some(address), Some(text)) => tcp(address, text),
            _ => "error:usage".to_owned(),
        },
        "quit" => return None,
        other => format!("error:unknown:{other}"),
    };
    Some(format!("{id} {op} {answer}"))
}

impl exports::yoake::mods::worker_run::Guest for Worker {
    fn run() {
        println!("f2-check worker started");
        // blocks until a job comes; `None` when the worker is being stopped
        while let Some(message) = yoake::mods::worker_host::host_recv() {
            let line = String::from_utf8_lossy(&message).into_owned();
            let Some(reply) = job(&line) else {
                break;
            };
            if yoake::mods::worker_host::host_send(reply.as_bytes()).is_err() {
                println!("f2-check worker: the mod does not take replies");
            }
        }
        println!("f2-check worker quit");
    }
}

export!(Worker);
