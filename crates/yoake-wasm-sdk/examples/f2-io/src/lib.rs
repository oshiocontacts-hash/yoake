//! Test guests of FR-F2. One feature per guest: `files`, `main`, `worker`.

#[cfg(feature = "files")]
mod files {
    wit_bindgen::generate!({ path: "wit", world: "files-guest" });

    struct Files;

    fn dispatch(op: &str) -> String {
        match op {
            "write" => match std::fs::write("/data/x.txt", "hello-data") {
                Ok(()) => "ok".into(),
                Err(error) => format!("error: {error}"),
            },
            "read" => match std::fs::read_to_string("/data/x.txt") {
                Ok(text) => text,
                Err(error) => format!("error: {error}"),
            },
            "game" => match std::fs::write("/game/f2-probe.txt", "g") {
                Ok(()) => "ok".into(),
                Err(error) => format!("error: {error}"),
            },
            "host" => match std::fs::read_dir("/host/c").or_else(|_| std::fs::read_dir("/host/root")) {
                Ok(entries) => format!("{}", entries.count()),
                Err(error) => format!("error: {error}"),
            },
            "sleep" => {
                std::thread::sleep(std::time::Duration::from_millis(50));
                "slept".into()
            }
            "net" => match std::net::TcpStream::connect("127.0.0.1:9") {
                Ok(_) => "connected".into(),
                Err(error) => format!("error: {error}"),
            },
            "print" => {
                println!("files guest stdout line");
                "printed".into()
            }
            other => format!("unknown op {other}"),
        }
    }

    impl Guest for Files {
        fn run(op: String) -> String {
            dispatch(&op)
        }
    }

    export!(Files);
}

#[cfg(feature = "main")]
mod main_guest {
    wit_bindgen::generate!({
        path: "../../../yoake-wasm/wit-worker",
        inline: "package test:f2main; world main-guest { include yoake:mods/worker-user@1.6.0; export run: func(op: string) -> string; }",
        world: "main-guest",
        generate_all,
    });

    struct Main;

    impl Guest for Main {
        fn run(op: String) -> String {
            if let Some(text) = op.strip_prefix("send:") {
                return match yoake::mods::worker::worker_send(text.as_bytes()) {
                    Ok(()) => "ok".into(),
                    Err(error) => format!("error: {error}"),
                };
            }
            if let Some(count) = op.strip_prefix("flood:") {
                let count: usize = count.parse().unwrap_or(0);
                let mut sent = 0;
                for _ in 0..count {
                    match yoake::mods::worker::worker_send(b"x") {
                        Ok(()) => sent += 1,
                        Err(error) => return format!("{sent}:{error}"),
                    }
                }
                return format!("{sent}:ok");
            }
            match op.as_str() {
                "recv" => match yoake::mods::worker::worker_recv() {
                    Some(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                    None => "none".into(),
                },
                other => format!("unknown op {other}"),
            }
        }
    }

    export!(Main);
}

#[cfg(feature = "worker")]
mod worker_guest {
    wit_bindgen::generate!({
        path: "../../../yoake-wasm/wit-worker",
        world: "worker-mod",
    });

    use std::io::{Read, Write};
    use yoake::mods::worker_host::{host_recv, host_send};

    struct Worker;

    fn reply(text: impl AsRef<[u8]>) {
        let _ = host_send(text.as_ref());
    }

    fn tcp(address: &str) {
        match std::net::TcpStream::connect(address) {
            Ok(mut stream) => {
                let _ = stream.write_all(b"ping");
                let mut bytes = Vec::new();
                let _ = stream.read_to_end(&mut bytes);
                let mut out = b"tcp:".to_vec();
                out.extend(bytes);
                reply(out);
            }
            Err(error) => reply(format!("tcp-error: {error}")),
        }
    }

    fn http(target: &str) {
        use wasip2::http::outgoing_handler;
        use wasip2::http::types::{Fields, Method, OutgoingRequest, Scheme};
        use wasip2::io::streams::StreamError;
        let (authority, path) = target
            .split_once('/')
            .map_or((target, "/".to_owned()), |(a, p)| (a, format!("/{p}")));
        let request = OutgoingRequest::new(Fields::new());
        let _ = request.set_method(&Method::Get);
        let _ = request.set_scheme(Some(&Scheme::Http));
        let _ = request.set_authority(Some(authority));
        let _ = request.set_path_with_query(Some(&path));
        let future = match outgoing_handler::handle(request, None) {
            Ok(future) => future,
            Err(error) => return reply(format!("http-error: {error:?}")),
        };
        future.subscribe().block();
        let response = match future.get() {
            Some(Ok(Ok(response))) => response,
            other => return reply(format!("http-error: {other:?}")),
        };
        let status = response.status();
        let mut body = Vec::new();
        if let Ok(incoming) = response.consume() {
            if let Ok(stream) = incoming.stream() {
                loop {
                    match stream.blocking_read(4096) {
                        Ok(chunk) => body.extend(chunk),
                        Err(StreamError::Closed) => break,
                        Err(_) => break,
                    }
                }
            }
        }
        let mut out = format!("http:{status}:").into_bytes();
        out.extend(body);
        reply(out);
    }

    impl exports::yoake::mods::worker_run::Guest for Worker {
        fn run() {
            println!("worker started");
            while let Some(message) = host_recv() {
                let text = String::from_utf8_lossy(&message).into_owned();
                if text == "quit" {
                    return;
                } else if text == "trap" {
                    panic!("worker trap on request");
                } else if let Some(address) = text.strip_prefix("tcp:") {
                    tcp(address);
                } else if let Some(target) = text.strip_prefix("http:") {
                    http(target);
                } else if let Some(echo) = text.strip_prefix("echo:") {
                    reply(echo);
                } else if text == "sleep" {
                    // a blocking wait: allowed in a worker
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    reply("slept");
                } else if text == "files" {
                    let wrote = std::fs::write("/data/w.txt", "from-worker").is_ok();
                    reply(format!("files:{wrote}"));
                } else if text == "eprint" {
                    eprintln!("worker stderr line");
                    reply("printed");
                }
            }
        }
    }

    export!(Worker);
}
