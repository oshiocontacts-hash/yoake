use super::{Game, Options};
use horizon_client::{lang, text::Text};
use horizon_friends::{
    social::SignedGrant,
    social_client::{Cancellation, Endpoint, Host, join},
    social_handoff::{Control, Handoff, Trust, read_once},
    social_stream::Bridge,
};
use std::{
    io::{BufRead, Read},
    sync::{
        Arc,
        mpsc::{self, Receiver, SyncSender},
    },
    time::{SystemTime, UNIX_EPOCH},
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
            let trust = Trust::read(options.social_trust.as_ref().ok_or(())?).map_err(|_| ())?;
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
    Paired(Result<Bridge, ()>),
    Guest(Result<Bridge, ()>),
}

#[derive(Default)]
pub(super) struct Runtime {
    host: Option<Arc<Host>>,
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
        match startup.request {
            Handoff::Host {
                session_id,
                host_principal,
                host_token,
                provision,
                ..
            } => {
                if startup.control_stdin {
                    let (controls, receiver) = mpsc::sync_channel(256);
                    self.controls = Some(receiver);
                    let reader_cancellation = cancellation.clone();
                    std::thread::spawn(move || read_controls(controls, reader_cancellation));
                }
                std::thread::spawn(move || {
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
                    if cancellation.is_ended() {
                        if let Ok(host) = result {
                            let _ = host.end();
                        }
                    } else if let Err(error) = sender.send(Event::Host(result))
                        && let Event::Host(Ok(host)) = error.0
                    {
                        let _ = host.end();
                    }
                });
            }
            Handoff::Guest {
                session_id,
                grant,
                signature,
                ..
            } => {
                std::thread::spawn(move || {
                    let result = join(
                        startup.endpoint,
                        &session_id,
                        SignedGrant { grant, signature },
                        &cancellation,
                    )
                    .map_err(|_| ());
                    let _ = sender.send(Event::Guest(result));
                });
            }
        }
    }

    pub(super) fn had_world(&self) -> bool {
        self.had_world
    }

    pub(super) fn is_host(&self) -> bool {
        self.host.is_some()
    }

    pub(super) fn end(&mut self) {
        if let Some(cancellation) = self.cancellation.take() {
            cancellation.end();
        }
        if let Some(host) = self.host.take() {
            std::thread::spawn(move || {
                let _ = host.end();
            });
        }
        if let Some(events) = self.events.take() {
            for event in events.try_iter() {
                if let Event::Host(Ok(host)) = event {
                    std::thread::spawn(move || {
                        let _ = host.end();
                    });
                }
            }
        }
        self.active = false;
        self.controls = None;
        self.sender = None;
        self.guest = None;
        self.pending = 0;
    }

    fn pair(&mut self, grant: SignedGrant) {
        let Some(host) = self.host.clone() else {
            return;
        };
        let Some(sender) = self.sender.clone() else {
            return;
        };
        self.pending += 1;
        std::thread::spawn(move || {
            let _ = sender.send(Event::Paired(host.pair(grant).map_err(|_| ())));
        });
    }

    fn tick(&mut self, game: &mut Game<'_>) {
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
        if self
            .host
            .as_ref()
            .is_some_and(|host| now() >= host.expires_at())
        {
            self.end();
        }
        if self.host.is_some() && game.integrated.is_some() {
            self.had_world = true;
            while self.pending < 3 {
                match self
                    .controls
                    .as_ref()
                    .and_then(|controls| controls.try_recv().ok())
                {
                    Some(Control::Pair { grant }) => self.pair(grant),
                    Some(Control::End {}) => {
                        self.end();
                        break;
                    }
                    None => break,
                }
            }
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
            if let Some(title) = game.title.as_mut() {
                title.failed(lang::get("connect.failed"));
            } else {
                game.system_message(Text::tr("connect.failed", &[]));
            }
            self.failed = false;
        }
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
            || bytes.len() > 65_536
            || bytes.last() != Some(&b'\n')
        {
            break;
        }
        match serde_json::from_slice::<Control>(&bytes) {
            Ok(Control::End {}) => break,
            Ok(control) => {
                if sender.try_send(control).is_err() {
                    break;
                }
            }
            _ => break,
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
            Some(Ok(startup)) => self.social_relay.start(startup),
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
