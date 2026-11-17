use super::{Game, Options};
use horizon_client::{lang, text::Text};
use horizon_friends::{
    social::SignedGrant,
    social_client::{Cancellation, Endpoint, Host, join},
    social_control::SignedProvision,
    social_handoff::{Control, Handoff, Trust, read_once},
    social_stream::Bridge,
};
use std::{
    collections::VecDeque,
    io::{BufRead, Read},
    sync::{
        Arc,
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(super) struct Startup {
    request: Handoff,
    endpoint: Endpoint,
    control_stdin: bool,
}

impl Startup {
    pub(super) fn read(options: &mut Options) -> Option<Result<Self, ()>> {
        let path = options.social_handoff.take()?;
        Some((|| {
            let request = read_once(&path).map_err(|_| ())?;
            if options.dedicated.is_some()
                || options.session_stdin && options.social_control_stdin
                || matches!(request, Handoff::Host { .. }) != options.social_control_stdin
            {
                return Err(());
            }
            let trust = Trust::load(options.social_trust.as_deref()).map_err(|_| ())?;
            let relay = match &request {
                Handoff::Host { relay, .. } | Handoff::Guest { relay, .. } => relay,
            };
            let endpoint = trust.endpoint(relay).map_err(|_| ())?;
            Ok(Self {
                request,
                endpoint,
                control_stdin: options.social_control_stdin,
            })
        })())
    }

    pub(super) fn is_guest(&self) -> bool {
        matches!(self.request, Handoff::Guest { .. })
    }
}

enum Event {
    Host(Result<Arc<Host>, ()>),
    Reprovisioned(Result<(), ()>),
    Paired(Result<Bridge, ()>),
    Guest(Result<Bridge, ()>),
}

#[derive(Default)]
pub(super) struct Runtime {
    host: Option<Arc<Host>>,
    waiting: Option<Startup>,
    grants: VecDeque<SignedGrant>,
    renewal: Option<SignedProvision>,
    renewing: bool,
    hosting: bool,
    workers: Vec<std::thread::JoinHandle<()>>,
    last_failure: Option<Instant>,
    cancellation: Option<Cancellation>,
    controls: Option<Receiver<Control>>,
    events: Option<Receiver<Event>>,
    sender: Option<mpsc::Sender<Event>>,
    guest: Option<Bridge>,
    pending: usize,
    active: bool,
    had_world: bool,
    failed: bool,
}

impl Runtime {
    fn start(&mut self, startup: Startup) {
        let cancellation = Cancellation::default();
        let (sender, events) = mpsc::channel();
        self.cancellation = Some(cancellation.clone());
        self.events = Some(events);
        self.sender = Some(sender.clone());
        self.active = true;
        if matches!(startup.request, Handoff::Host { .. }) {
            self.hosting = true;
            if startup.control_stdin {
                let (controls, receiver) = mpsc::sync_channel(256);
                self.controls = Some(receiver);
                let reader_cancellation = cancellation.clone();
                std::thread::spawn(move || read_controls(controls, reader_cancellation));
            }
            self.waiting = Some(startup);
            return;
        }
        match startup.request {
            Handoff::Host { .. } => unreachable!(),
            Handoff::Guest {
                session_id,
                grant,
                signature,
                ..
            } => {
                self.workers.push(std::thread::spawn(move || {
                    let result = join(
                        startup.endpoint,
                        &session_id,
                        SignedGrant { grant, signature },
                        &cancellation,
                    )
                    .map_err(|_| ());
                    let _ = sender.send(Event::Guest(result));
                }));
            }
        }
    }

    fn provision_host(&mut self, startup: Startup) {
        let Handoff::Host {
            session_id,
            host_principal,
            host_token,
            provision,
            ..
        } = startup.request
        else {
            return;
        };
        let Some(cancellation) = self.cancellation.clone() else {
            return;
        };
        let Some(sender) = self.sender.clone() else {
            return;
        };
        self.workers.push(std::thread::spawn(move || {
            let result = Host::provision(
                startup.endpoint,
                session_id,
                host_principal,
                host_token,
                provision,
                cancellation.clone(),
            )
            .map(Arc::new)
            .map_err(|_| ());
            match result {
                Ok(host) => {
                    let delivered = !cancellation.is_ended()
                        && sender.send(Event::Host(Ok(host.clone()))).is_ok();
                    if !delivered || cancellation.is_ended() {
                        let _ = host.end();
                    }
                }
                Err(()) => {
                    if !cancellation.is_ended() {
                        let _ = sender.send(Event::Host(Err(())));
                    }
                }
            }
        }));
    }

    pub(super) fn had_world(&self) -> bool {
        self.had_world
    }

    pub(super) fn is_host(&self) -> bool {
        self.hosting
    }

    pub(super) fn end(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut retirements = std::mem::take(&mut self.workers);
        if let Some(cancellation) = self.cancellation.take() {
            cancellation.end();
        }
        if let Some(host) = self.host.take() {
            retirements.push(std::thread::spawn(move || {
                let _ = host.end();
            }));
        }
        if let Some(events) = self.events.take() {
            for event in events.try_iter() {
                if let Event::Host(Ok(host)) = event {
                    retirements.push(std::thread::spawn(move || {
                        let _ = host.end();
                    }));
                }
            }
        }
        self.active = false;
        self.controls = None;
        self.sender = None;
        self.guest = None;
        self.pending = 0;
        self.waiting = None;
        self.grants.clear();
        self.renewal = None;
        self.renewing = false;
        self.hosting = false;
        while retirements.iter().any(|worker| !worker.is_finished()) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        for worker in retirements {
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }

    fn pair(&mut self, grant: SignedGrant) {
        let Some(host) = self.host.clone() else {
            return;
        };
        let Some(sender) = self.sender.clone() else {
            return;
        };
        self.pending += 1;
        self.workers.push(std::thread::spawn(move || {
            let _ = sender.send(Event::Paired(host.pair(grant).map_err(|_| ())));
        }));
    }

    fn tick(&mut self, game: &mut Game<'_>) {
        while let Some(index) = self.workers.iter().position(|worker| worker.is_finished()) {
            let _ = self.workers.swap_remove(index).join();
        }
        if self
            .cancellation
            .as_ref()
            .is_some_and(Cancellation::is_ended)
        {
            self.end();
        }
        while let Some(event) = self
            .events
            .as_ref()
            .and_then(|events| events.try_recv().ok())
        {
            match event {
                Event::Host(Ok(host)) => self.host = Some(host),
                Event::Reprovisioned(result) => {
                    self.renewing = false;
                    self.failed |= result.is_err();
                }
                Event::Host(Err(())) | Event::Guest(Err(())) => {
                    self.failed = true;
                    self.end();
                }
                Event::Guest(Ok(bridge)) => self.guest = Some(bridge),
                Event::Paired(result) => {
                    self.pending = self.pending.saturating_sub(1);
                    let connected = match result {
                        Ok(bridge) => game
                            .integrated
                            .as_ref()
                            .and_then(|integrated| {
                                integrated.call(move |server| {
                                    server.lan.as_ref().is_some_and(|host| {
                                        host.hub.lan.connect_social(bridge).is_ok()
                                    })
                                })
                            })
                            .unwrap_or(false),
                        Err(()) => false,
                    };
                    self.failed |= !connected;
                }
            }
        }
        while let Some(control) = self
            .controls
            .as_ref()
            .and_then(|controls| controls.try_recv().ok())
        {
            match control {
                Control::Pair { grant } => {
                    if self.grants.len() < 256 {
                        self.grants.push_back(grant);
                    } else {
                        self.failed = true;
                    }
                }
                Control::Reprovision { provision } => self.renewal = Some(provision),
                Control::End {} => {
                    self.end();
                    break;
                }
            }
        }
        if self.waiting.is_some()
            && let Some(integrated) = game.integrated.as_ref()
        {
            let online = integrated
                .call(|server| {
                    server
                        .lan
                        .as_ref()
                        .is_some_and(|host| host.hub.lan.shared.config.online.is_some())
                })
                .unwrap_or(false);
            if online {
                if let Some(startup) = self.waiting.take() {
                    self.provision_host(startup);
                }
            } else {
                self.failed = true;
                self.end();
            }
        }
        if !self.renewing
            && self.host.is_some()
            && let Some(provision) = self.renewal.take()
            && let (Some(host), Some(sender)) = (self.host.clone(), self.sender.clone())
        {
            self.renewing = true;
            self.workers.push(std::thread::spawn(move || {
                let _ = sender.send(Event::Reprovisioned(
                    host.reprovision(provision).map_err(|_| ()),
                ));
            }));
        }
        if self.host.is_some() && game.integrated.is_some() {
            self.had_world = true;
            while self.pending < 3 && !self.renewing {
                let Some(grant) = self.grants.pop_front() else {
                    break;
                };
                self.pair(grant);
            }
        }
        if !self.renewing
            && self
                .host
                .as_ref()
                .is_some_and(|host| now() >= host.expires_at())
        {
            self.end();
        }
        if game.rest_loader.is_none()
            && let Some(bridge) = self.guest.take()
        {
            if game.connect_social(bridge).is_ok() {
                self.had_world = true;
            } else {
                self.failed = true;
                self.end();
            }
        }
        if self.failed && game.rest_loader.is_none() {
            if self.take_failure(Instant::now()) {
                if let Some(title) = game.title.as_mut() {
                    title.failed(lang::get("connect.failed"));
                } else {
                    game.system_message(Text::tr("connect.failed", &[]));
                }
            }
            self.failed = false;
        }
    }

    fn take_failure(&mut self, current: Instant) -> bool {
        if self
            .last_failure
            .is_some_and(|last| current.duration_since(last) < Duration::from_secs(5))
        {
            return false;
        }
        self.last_failure = Some(current);
        true
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.end();
    }
}

fn read_controls(sender: SyncSender<Control>, cancellation: Cancellation) {
    let stdin = std::io::stdin();
    read_controls_from(stdin.lock(), sender, cancellation);
}

fn read_controls_from(
    mut reader: impl BufRead,
    sender: SyncSender<Control>,
    cancellation: Cancellation,
) {
    while !cancellation.is_ended() {
        let mut bytes = Vec::new();
        if (&mut reader)
            .take(65_537)
            .read_until(b'\n', &mut bytes)
            .is_err()
        {
            break;
        }
        if bytes.len() > 65_536 {
            while bytes.last() != Some(&b'\n') {
                bytes.clear();
                if (&mut reader)
                    .take(65_537)
                    .read_until(b'\n', &mut bytes)
                    .is_err()
                    || bytes.is_empty()
                {
                    cancellation.end();
                    return;
                }
            }
            continue;
        }
        if bytes.last() != Some(&b'\n') {
            break;
        }
        match serde_json::from_slice::<Control>(&bytes) {
            Ok(Control::End {}) => break,
            Ok(control) => {
                if sender.try_send(control).is_err() {
                    break;
                }
            }
            Err(_) => continue,
        }
    }
    cancellation.end();
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl Game<'_> {
    pub(super) fn social_relay_start(&mut self) {
        let startup = self
            .options
            .social_startup
            .lock()
            .ok()
            .and_then(|mut startup| startup.take());
        match startup {
            Some(Ok(startup)) => {
                if !startup.is_guest() && horizon_client::auth::current().is_none() {
                    self.social_relay.failed = true;
                } else {
                    self.social_relay.start(startup);
                }
            }
            Some(Err(())) => self.social_relay.failed = true,
            None => {}
        }
    }

    pub(super) fn social_relay_tick(&mut self) {
        if !self.social_relay.active && !self.social_relay.failed {
            return;
        }
        let mut runtime = std::mem::take(&mut self.social_relay);
        runtime.tick(self);
        self.social_relay = runtime;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_friends::social::Grant;
    use std::io::Cursor;

    fn pair_command() -> Vec<u8> {
        let command = Control::Pair {
            grant: SignedGrant {
                grant: Grant {
                    v: 1,
                    session_id: "test-session".into(),
                    host_principal: "test-host".into(),
                    guest_principal: "test-guest".into(),
                    mc_profile: None,
                    relay_only: true,
                    exp: 1,
                    nonce: "11".repeat(32),
                },
                signature: "22".repeat(64),
            },
        };
        let mut bytes = serde_json::to_vec(&command).unwrap();
        bytes.push(b'\n');
        bytes
    }

    #[test]
    fn social_controls_end_stops_forwarding_and_cancels_session() {
        let (sender, receiver) = mpsc::sync_channel(256);
        let cancellation = Cancellation::default();
        let mut bytes = pair_command();
        bytes.extend_from_slice(b"{\"type\":\"end\"}\n");
        bytes.extend(pair_command());
        read_controls_from(Cursor::new(bytes), sender, cancellation.clone());
        assert!(cancellation.is_ended());
        assert!(matches!(receiver.try_recv(), Ok(Control::Pair { .. })));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn social_controls_fail_closed_on_bad_or_unterminated_frames() {
        for bytes in [
            b"not json\n".to_vec(),
            b"{\"type\":\"end\",\"unexpected\":true}\n".to_vec(),
            b"{\"type\":\"end\"}".to_vec(),
            vec![b' '; 65_537],
            Vec::new(),
        ] {
            let (sender, receiver) = mpsc::sync_channel(256);
            let cancellation = Cancellation::default();
            read_controls_from(Cursor::new(bytes), sender, cancellation.clone());
            assert!(cancellation.is_ended());
            assert!(receiver.try_recv().is_err());
        }
    }

    #[test]
    fn social_controls_drop_bad_lines_and_keep_following_commands() {
        for mut bytes in [b"not json\n".to_vec(), vec![b' '; 65_537]] {
            bytes.push(b'\n');
            bytes.extend(pair_command());
            bytes.extend_from_slice(br#"{"type":"end"}"#);
            bytes.push(b'\n');
            let (sender, receiver) = mpsc::sync_channel(256);
            let cancellation = Cancellation::default();
            read_controls_from(Cursor::new(bytes), sender, cancellation.clone());
            assert!(cancellation.is_ended());
            assert!(matches!(receiver.try_recv(), Ok(Control::Pair { .. })));
            assert!(receiver.try_recv().is_err());
        }
    }

    #[test]
    fn social_connect_failures_are_rate_limited() {
        let mut runtime = Runtime::default();
        let now = Instant::now();
        assert!(runtime.take_failure(now));
        assert!(!runtime.take_failure(now + Duration::from_secs(4)));
        assert!(runtime.take_failure(now + Duration::from_secs(5)));
        assert!(!runtime.take_failure(now + Duration::from_secs(6)));
    }

    #[test]
    fn social_controls_bound_pending_grants_and_cancel_on_overflow() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancellation = Cancellation::default();
        let bytes = [pair_command(), pair_command()].concat();
        read_controls_from(Cursor::new(bytes), sender, cancellation.clone());
        assert!(cancellation.is_ended());
        assert!(matches!(receiver.try_recv(), Ok(Control::Pair { .. })));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn social_startup_without_handoff_is_dormant() {
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        assert!(Startup::read(&mut options).is_none());
        assert!(!Runtime::default().active);
    }

    #[test]
    fn social_control_and_microsoft_auth_cannot_both_consume_stdin() {
        let args = ["--session-stdin", "--yoake-social-control-stdin"];
        assert!(Options::parse(args.into_iter().map(str::to_owned)).is_err());
    }
}
