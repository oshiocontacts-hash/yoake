use crate::yoake_auth::{self, YoakeAccount};
use serde_json::{Value, json};
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};
use url::Url;

const MAX_RESPONSE: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum LoginEvent {
    VerificationRequired { user_code: String, url: String },
    AuthorizationPending,
    SlowDown { interval: u64 },
    Success(YoakeAccount),
    WrongCode,
    ExpiredOrUsed,
    TooManyTries,
    Expired,
    AccessDenied,
    Cancelled,
    Offline,
    InvalidResponse,
    StorageError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortalError {
    Configuration,
    Network,
    InvalidResponse,
}

pub struct PortalResponse {
    pub status: u16,
    pub body: Value,
}

pub trait PortalTransport: Send + Sync {
    fn post(&self, endpoint: &str, body: Value) -> Result<PortalResponse, PortalError>;
}

pub struct HttpPortalTransport {
    root: Url,
}

impl HttpPortalTransport {
    pub fn new(root: &str) -> Result<Self, PortalError> {
        let mut root = Url::parse(root).map_err(|_| PortalError::Configuration)?;
        if root.scheme() != "https"
            || root.host_str().is_none()
            || !root.username().is_empty()
            || root.password().is_some()
            || root.query().is_some()
            || root.fragment().is_some()
        {
            return Err(PortalError::Configuration);
        }
        if !root.path().ends_with('/') {
            root.set_path(&format!("{}/", root.path()));
        }
        Ok(Self { root })
    }
}

impl PortalTransport for HttpPortalTransport {
    fn post(&self, endpoint: &str, body: Value) -> Result<PortalResponse, PortalError> {
        let url = self
            .root
            .join(endpoint)
            .map_err(|_| PortalError::Configuration)?;
        if url.origin() != self.root.origin() {
            return Err(PortalError::Configuration);
        }
        let builder = ureq::AgentBuilder::new()
            .redirects(0)
            .timeout(Duration::from_secs(20));
        let agent = horizon_launcher::proxy::agent(builder, url.as_str()).build();
        let response = match agent.post(url.as_str()).send_json(body) {
            Ok(response) | Err(ureq::Error::Status(_, response)) => response,
            Err(ureq::Error::Transport(_)) => return Err(PortalError::Network),
        };
        let status = response.status();
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(MAX_RESPONSE + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| PortalError::Network)?;
        if bytes.len() as u64 > MAX_RESPONSE {
            return Err(PortalError::InvalidResponse);
        }
        let body = match serde_json::from_slice(&bytes) {
            Ok(body) => body,
            Err(_) if !(200..300).contains(&status) => Value::Null,
            Err(_) => return Err(PortalError::InvalidResponse),
        };
        Ok(PortalResponse { status, body })
    }
}

#[derive(Clone)]
pub struct PortalClient {
    transport: Arc<dyn PortalTransport>,
    data_dir: PathBuf,
}

impl PortalClient {
    pub fn configured() -> Result<Option<Self>, PortalError> {
        let Some(root) = option_env!("YOAKE_PORTAL_URL") else {
            return Ok(None);
        };
        Ok(Some(Self::new(
            root,
            horizon_launcher::backend::data_dir(),
        )?))
    }

    pub fn new(root: &str, data_dir: PathBuf) -> Result<Self, PortalError> {
        Ok(Self::with_transport(
            Arc::new(HttpPortalTransport::new(root)?),
            data_dir,
        ))
    }

    pub fn with_transport(transport: Arc<dyn PortalTransport>, data_dir: PathBuf) -> Self {
        Self {
            transport,
            data_dir,
        }
    }

    pub fn start_device_login(&self, label: &str) -> LoginTask {
        let client = self.clone();
        let label = device_label(label);
        let epoch = yoake_auth::credential_epoch();
        spawn(move |events, cancelled| client.device_login(&label, &events, &cancelled, epoch))
    }

    pub fn start_invite_redeem(&self, code: String, friend_name: String, label: &str) -> LoginTask {
        let client = self.clone();
        let label = device_label(label);
        let epoch = yoake_auth::credential_epoch();
        spawn(move |_events, cancelled| {
            if cancelled.load(Ordering::Acquire) {
                return LoginEvent::Cancelled;
            }
            let response = client.transport.post(
                "api/launcher/invite/redeem",
                json!({"code": code, "friend_name": friend_name, "label": label}),
            );
            if cancelled.load(Ordering::Acquire) {
                return LoginEvent::Cancelled;
            }
            match response {
                Ok(response) if (200..300).contains(&response.status) => {
                    client.save_response(response.body, epoch, &cancelled)
                }
                Ok(response) => match response.status {
                    400 => LoginEvent::WrongCode,
                    403 => LoginEvent::ExpiredOrUsed,
                    429 => LoginEvent::TooManyTries,
                    _ => LoginEvent::InvalidResponse,
                },
                Err(error) => transport_event(error),
            }
        })
    }

    fn device_login(
        &self,
        label: &str,
        events: &Sender<LoginEvent>,
        cancelled: &AtomicBool,
        epoch: u64,
    ) -> LoginEvent {
        if cancelled.load(Ordering::Acquire) {
            return LoginEvent::Cancelled;
        }
        let response = match self
            .transport
            .post("api/launcher/device", json!({"label": label}))
        {
            Ok(response) if (200..300).contains(&response.status) => response.body,
            Ok(_) => return LoginEvent::InvalidResponse,
            Err(error) => return transport_event(error),
        };
        if cancelled.load(Ordering::Acquire) {
            return LoginEvent::Cancelled;
        }
        let Some(device_code) = nonempty(&response, "device_code", 4096) else {
            return LoginEvent::InvalidResponse;
        };
        let Some(user_code) = nonempty(&response, "user_code", 80) else {
            return LoginEvent::InvalidResponse;
        };
        let Some(uri) = nonempty(&response, "verification_uri", 2048) else {
            return LoginEvent::InvalidResponse;
        };
        let Ok(mut url) = Url::parse(uri) else {
            return LoginEvent::InvalidResponse;
        };
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return LoginEvent::InvalidResponse;
        }
        let Some(expires_in) = response.get("expires_in").and_then(Value::as_u64) else {
            return LoginEvent::InvalidResponse;
        };
        let Some(mut interval) = response.get("interval").and_then(Value::as_u64) else {
            return LoginEvent::InvalidResponse;
        };
        interval = interval.max(1);
        let Some(deadline) = Instant::now().checked_add(Duration::from_secs(expires_in)) else {
            return LoginEvent::InvalidResponse;
        };
        url.query_pairs_mut().append_pair("code", user_code);
        let _ = events.send(LoginEvent::VerificationRequired {
            user_code: user_code.to_owned(),
            url: url.to_string(),
        });
        loop {
            let Some(next_poll) = Instant::now().checked_add(Duration::from_secs(interval)) else {
                return LoginEvent::InvalidResponse;
            };
            while Instant::now() < next_poll && Instant::now() < deadline {
                if cancelled.load(Ordering::Acquire) {
                    return LoginEvent::Cancelled;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            if cancelled.load(Ordering::Acquire) {
                return LoginEvent::Cancelled;
            }
            if Instant::now() >= deadline {
                return LoginEvent::Expired;
            }
            let response = match self.transport.post(
                "api/launcher/device/token",
                json!({"device_code": device_code}),
            ) {
                Ok(response) => response,
                Err(error) => return transport_event(error),
            };
            if cancelled.load(Ordering::Acquire) {
                return LoginEvent::Cancelled;
            }
            match response.body.get("error").and_then(Value::as_str) {
                Some("authorization_pending") => {
                    let _ = events.send(LoginEvent::AuthorizationPending);
                }
                Some("slow_down") => {
                    let Some(next_interval) = interval.checked_add(5) else {
                        return LoginEvent::InvalidResponse;
                    };
                    interval = next_interval;
                    let _ = events.send(LoginEvent::SlowDown { interval });
                }
                Some("expired_token") => return LoginEvent::Expired,
                Some("access_denied") => return LoginEvent::AccessDenied,
                Some(_) => return LoginEvent::InvalidResponse,
                None if (200..300).contains(&response.status) => {
                    return self.save_response(response.body, epoch, cancelled);
                }
                None => return LoginEvent::InvalidResponse,
            }
        }
    }

    fn save_response(&self, body: Value, epoch: u64, cancelled: &AtomicBool) -> LoginEvent {
        let Some(token) = nonempty(&body, "token", 8192) else {
            return LoginEvent::InvalidResponse;
        };
        let Some(kind) = nonempty(&body, "kind", 80) else {
            return LoginEvent::InvalidResponse;
        };
        let Some(expires_at) = body.get("expires_at") else {
            return LoginEvent::InvalidResponse;
        };
        let account = YoakeAccount {
            kind: kind.to_owned(),
            expires_at: expires_at.clone(),
        };
        match yoake_auth::save_credentials_if_current(
            &self.data_dir,
            token,
            &account,
            epoch,
            cancelled,
        ) {
            Ok(true) => LoginEvent::Success(account),
            Ok(false) => LoginEvent::Cancelled,
            Err(_) => LoginEvent::StorageError,
        }
    }
}

pub struct LoginTask {
    pub events: Receiver<LoginEvent>,
    cancelled: Arc<AtomicBool>,
}

impl LoginTask {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

fn spawn(
    work: impl FnOnce(Sender<LoginEvent>, Arc<AtomicBool>) -> LoginEvent + Send + 'static,
) -> LoginTask {
    let (sender, events) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);
    std::thread::spawn(move || {
        let result = work(sender.clone(), worker_cancelled);
        let _ = sender.send(result);
    });
    LoginTask { events, cancelled }
}

fn device_label(label: &str) -> String {
    let label: String = label
        .chars()
        .filter(|character| !character.is_control())
        .take(40)
        .collect();
    if label.trim().is_empty() {
        "Horizon launcher".to_owned()
    } else {
        label
    }
}

fn nonempty<'a>(body: &'a Value, name: &str, max: usize) -> Option<&'a str> {
    body.get(name).and_then(Value::as_str).filter(|value| {
        !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
    })
}

fn transport_event(error: PortalError) -> LoginEvent {
    match error {
        PortalError::Network => LoginEvent::Offline,
        PortalError::Configuration | PortalError::InvalidResponse => LoginEvent::InvalidResponse,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yoake_auth::YoakeAuth;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct FakePortal {
        replies: Mutex<VecDeque<PortalResponse>>,
        requests: Mutex<Vec<(String, Value)>>,
    }

    impl PortalTransport for FakePortal {
        fn post(&self, endpoint: &str, body: Value) -> Result<PortalResponse, PortalError> {
            self.requests
                .lock()
                .unwrap()
                .push((endpoint.to_owned(), body));
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .ok_or(PortalError::Network)
        }
    }

    fn client(replies: Vec<PortalResponse>) -> (PortalClient, Arc<FakePortal>) {
        let transport = Arc::new(FakePortal {
            replies: Mutex::new(replies.into()),
            requests: Mutex::new(Vec::new()),
        });
        (
            PortalClient::with_transport(transport.clone(), std::env::temp_dir()),
            transport,
        )
    }

    fn response(status: u16, body: Value) -> PortalResponse {
        PortalResponse { status, body }
    }

    #[test]
    fn invite_errors_are_readable_and_request_uses_bounded_label() {
        for (status, expected) in [
            (400, LoginEvent::WrongCode),
            (403, LoginEvent::ExpiredOrUsed),
            (429, LoginEvent::TooManyTries),
        ] {
            let (client, transport) = client(vec![response(status, json!({}))]);
            let task =
                client.start_invite_redeem("invite".into(), "friend".into(), &"x".repeat(60));
            assert_eq!(
                task.events.recv_timeout(Duration::from_secs(5)).unwrap(),
                expected
            );
            let requests = transport.requests.lock().unwrap();
            assert_eq!(requests[0].0, "api/launcher/invite/redeem");
            assert_eq!(requests[0].1["code"], "invite");
            assert_eq!(requests[0].1["friend_name"], "friend");
            assert_eq!(requests[0].1["label"].as_str().unwrap().chars().count(), 40);
        }
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn invite_success_saves_private_token_separately_and_logout_removes_it() {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).unwrap();
        let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let data_dir = std::env::temp_dir().join(format!("yoake-login-test-{suffix}"));
        let (mut client, _) = client(vec![response(
            200,
            json!({
                "token": "secret-token",
                "kind": "friend",
                "expires_at": "2027-01-01T00:00:00Z"
            }),
        )]);
        client.data_dir = data_dir.clone();
        let task = client.start_invite_redeem("invite".into(), "friend".into(), "device");
        assert_eq!(
            task.events.recv_timeout(Duration::from_secs(5)).unwrap(),
            LoginEvent::Success(YoakeAccount {
                kind: "friend".to_owned(),
                expires_at: json!("2027-01-01T00:00:00Z"),
            })
        );
        assert_eq!(
            yoake_auth::TokenFileAuth::new(&data_dir).authorization(),
            Ok(Some("Bearer secret-token".to_owned()))
        );
        let metadata = std::fs::read_to_string(data_dir.join("yoake-account.json")).unwrap();
        assert!(!metadata.contains("secret-token"));
        let account: YoakeAccount = serde_json::from_str(&metadata).unwrap();
        assert_eq!(account.kind, "friend");
        assert_eq!(account.expires_at, json!("2027-01-01T00:00:00Z"));
        yoake_auth::logout(&data_dir).unwrap();
        assert!(!data_dir.join("yoake-token").exists());
        assert!(!data_dir.join("yoake-account.json").exists());
        std::fs::remove_dir(&data_dir).unwrap();
    }

    #[test]
    fn device_login_shows_code_then_expires_without_polling() {
        let (client, transport) = client(vec![response(
            200,
            json!({
                "device_code": "private-device-code",
                "user_code": "AB CD",
                "verification_uri": "https://portal.example/verify",
                "expires_in": 0,
                "interval": 1
            }),
        )]);
        let task = client.start_device_login("test device");
        assert_eq!(
            task.events.recv_timeout(Duration::from_secs(5)).unwrap(),
            LoginEvent::VerificationRequired {
                user_code: "AB CD".to_owned(),
                url: "https://portal.example/verify?code=AB+CD".to_owned(),
            }
        );
        assert_eq!(
            task.events.recv_timeout(Duration::from_secs(5)).unwrap(),
            LoginEvent::Expired
        );
        assert_eq!(transport.requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn device_login_polls_pending_slows_down_and_can_cancel() {
        let (client, transport) = client(vec![
            response(
                200,
                json!({
                    "device_code": "device",
                    "user_code": "CODE",
                    "verification_uri": "https://portal.example/verify",
                    "expires_in": 30,
                    "interval": 1
                }),
            ),
            response(400, json!({"error": "authorization_pending"})),
            response(400, json!({"error": "slow_down"})),
        ]);
        let task = client.start_device_login("test device");
        assert!(matches!(
            task.events.recv_timeout(Duration::from_secs(5)).unwrap(),
            LoginEvent::VerificationRequired { .. }
        ));
        assert_eq!(
            task.events.recv_timeout(Duration::from_secs(5)).unwrap(),
            LoginEvent::AuthorizationPending
        );
        assert_eq!(
            task.events.recv_timeout(Duration::from_secs(5)).unwrap(),
            LoginEvent::SlowDown { interval: 6 }
        );
        task.cancel();
        assert_eq!(
            task.events.recv_timeout(Duration::from_secs(5)).unwrap(),
            LoginEvent::Cancelled
        );
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[1].0, "api/launcher/device/token");
        assert_eq!(requests[1].1["device_code"], "device");
    }
}
