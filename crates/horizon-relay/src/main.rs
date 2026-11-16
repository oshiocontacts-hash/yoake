//! `horizon-relay`: listens on `$PORT` (7000 when unset; `RELAY_BIND` for one
//! address), limits from the `RELAY_*` variables (docs/play-with-friends.md).
//! Prints counts every five minutes, nothing else.
use horizon_relay::{Config, Relay, social_cert::tool, social_server::SocialServer};
use std::{io, net::TcpListener, path::Path, time::Duration};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // No arguments (the systemd unit, the Dockerfile) starts the relay. Only the
    // two certificate flags select certificate mode; anything else is an error.
    if !args.is_empty() {
        if let Err(error) = certificate_command(&args) {
            eprintln!("horizon-relay: {error}");
            std::process::exit(2);
        }
        return;
    }
    run_relay();
}

fn certificate_command(args: &[String]) -> io::Result<()> {
    let usage = "usage: horizon-relay --gen-social-certs <dir> --host <dns-name> | --renew-social-leaf <dir> --host <dns-name>";
    if !matches!(args[0].as_str(), "--gen-social-certs" | "--renew-social-leaf") {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("unknown argument {:?}; {usage}", args[0])));
    }
    let [flag, dir, host_flag, host] = args else { return Err(io::Error::new(io::ErrorKind::InvalidInput, usage)); };
    if host_flag != "--host" { return Err(io::Error::new(io::ErrorKind::InvalidInput, usage)); }
    let dir = Path::new(dir);
    match flag.as_str() {
        "--gen-social-certs" => {
            tool::generate(dir, host)?;
            let paths = tool::generated_paths(dir);
            println!("CA certificate for clients to pin: {}", paths[0].display());
            println!("CA private key: {} -- STORE IT OFFLINE; never put it on the relay machine.", paths[1].display());
            println!("Only {} and {} go to the relay (HORIZON_RELAY_SOCIAL_CERT / HORIZON_RELAY_SOCIAL_CERT_KEY).", paths[2].display(), paths[3].display());
        }
        "--renew-social-leaf" => {
            tool::renew(dir, host)?;
            println!("Renewed the 30-day leaf in {}; the previous pair has .old suffixes.", dir.display());
            println!("Keep ca-key.der OFFLINE. Copy only leaf.der and leaf-key.der to the relay, then restart it.");
        }
        _ => return Err(io::Error::new(io::ErrorKind::InvalidInput, usage)),
    }
    Ok(())
}

fn run_relay() {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.trim().parse().ok())
        .unwrap_or(7000);
    // RELAY_BIND picks one address (e.g. only a tailnet); else dual-stack
    // where the system allows it, else IPv4.
    let bind = std::env::var("RELAY_BIND")
        .ok()
        .filter(|b| !b.trim().is_empty());
    let listener = match bind {
        Some(address) => TcpListener::bind((address.trim(), port)),
        None => TcpListener::bind(("::", port)).or_else(|_| TcpListener::bind(("0.0.0.0", port))),
    }
    .unwrap_or_else(|e| {
        eprintln!("horizon-relay: can't listen on port {port}: {e}");
        std::process::exit(1);
    });
    let config = Config::from_env();
    println!(
        "horizon-relay {} listening on port {port} (hosts {}, bridges {}, {} KiB/s per stream, per-address limits {})",
        env!("CARGO_PKG_VERSION"),
        config.max_hosts,
        config.max_bridges,
        (config.session_rate / 1024.0) as u64,
        if config.per_address { "on" } else { "off" },
    );
    let relay = Relay::new(config);
    let r = relay.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(300));
            println!("{}", r.counts().json());
        }
    });
    if let Some((address, server)) = SocialServer::from_env() {
        match TcpListener::bind(address) {
            Ok(social_listener) => {
                println!("horizon-relay social listener on {address}");
                std::thread::spawn(move || server.serve(social_listener));
            }
            Err(error) => eprintln!("horizon-relay: can't start social listener on {address}: {error}"),
        }
    }
    relay.serve(listener);
}
