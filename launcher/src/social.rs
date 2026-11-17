//! The portal's social API v1 (`docs/social-api.md` in the portal): friend codes, friends,
//! requests, blocks and groups. The portal is the authority; this module only asks it.
//!
//! Three layers, so the page and the tests never touch the network:
//!
//! - [`SocialBackend`]: what the Friends page needs, in the portal's own words.
//! - [`HttpSocial`]: the real backend. It builds the portal's requests (method, path, JSON body)
//!   and reads its answers over a [`Transport`].
//! - [`UreqTransport`]: the real transport. It reads the saved sign-in (`yoake_auth`) for each
//!   call, sends it as the bearer and keeps nothing; tokens, friend codes and invite codes never
//!   go in a URL or a log.
//!
//! `social_mock.rs` is a second backend (an in-memory portal) for tests and for
//! `HORIZON_LAUNCHER_SOCIAL=mock`, which shows the page before the portal has the API.
use crate::yoake_auth::{AuthError, TokenFileAuth, YoakeAuth};
pub mod handoff;
pub mod host;
pub mod rooms;

use rooms::{Audience, CreateRequest, HostedRoom, JoinGrant, Pending, PendingJoin, Reprovision, RoomInfo};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::Read;
use std::time::Duration;
use url::Url;

/// A friend code is `YKF-XXXX-XXXX`, a group invite `YKG-XXXX-XXXX`.
pub const FRIEND_PREFIX: &str = "YKF";
pub const GROUP_PREFIX: &str = "YKG";
/// The code alphabet (no 0, 1, I, O).
pub const ALPHABET: &str = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// The portal's limits (`Errors and limits`), shown in the messages.
pub const MAX_GROUP_MEMBERS: usize = 100;
pub const MAX_GROUPS: usize = 50;
pub const MAX_ACTIVE_INVITES: usize = 20;
pub const MAX_GROUP_NAME: usize = 40;
/// What an invite can be set to: uses 1..=50, days 1..=30.
pub const INVITE_USES: [u32; 4] = [1, 5, 10, 50];
pub const INVITE_DAYS: [u32; 4] = [1, 7, 14, 30];

// ----- Objects ----------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Principal {
    pub id: String,
    /// `member` or `guest`.
    pub kind: String,
    pub display_name: String,
}

impl Principal {
    pub fn is_guest(&self) -> bool {
        self.kind == "guest"
    }
}

/// `GET /me`: the player's own principal with the friend code.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Me {
    pub id: String,
    pub kind: String,
    pub display_name: String,
    #[serde(default)]
    pub friend_code: String,
    #[serde(default)]
    pub has_friend_code: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Friend {
    pub principal: Principal,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FriendRequest {
    pub id: String,
    pub from_id: String,
    pub to_id: String,
    pub status: String,
    pub created_at: String,
    pub from: Principal,
    pub to: Principal,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct FriendsList {
    #[serde(default)]
    pub friends: Vec<Friend>,
    #[serde(default)]
    pub incoming: Vec<FriendRequest>,
    #[serde(default)]
    pub outgoing: Vec<FriendRequest>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    pub blocker_id: String,
    pub blocked_id: String,
    pub created_at: String,
    pub blocked: Principal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    Admin,
    Member,
}

impl Role {
    pub fn word(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Admin => "admin",
            Role::Member => "member",
        }
    }

    pub fn manages(self) -> bool {
        matches!(self, Role::Owner | Role::Admin)
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupSummary {
    pub id: String,
    pub name: String,
    pub owner_id: String,
    pub created_at: String,
    pub role: Role,
    pub joined_at: String,
}

/// `POST /groups`' answer.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub name: String,
    pub owner_id: String,
    #[serde(default)]
    pub created_by_id: String,
    pub created_at: String,
    #[serde(default)]
    pub deleted_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub group_id: String,
    pub principal_id: String,
    pub role: Role,
    pub joined_at: String,
    pub principal: Principal,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Invite {
    pub id: String,
    pub group_id: String,
    pub created_by_id: String,
    pub max_uses: u32,
    pub uses: u32,
    pub created_at: String,
    pub expires_at: String,
    #[serde(default)]
    pub revoked_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ban {
    pub group_id: String,
    pub principal_id: String,
    pub removed_by_id: String,
    pub created_at: String,
    pub principal: Principal,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupDetail {
    pub id: String,
    pub name: String,
    pub owner_id: String,
    pub created_at: String,
    pub role: Role,
    #[serde(default)]
    pub can_recover_ownership: bool,
    #[serde(default)]
    pub members: Vec<Member>,
    #[serde(default)]
    pub invites: Vec<Invite>,
    #[serde(default)]
    pub bans: Vec<Ban>,
}

// ----- Errors -----------------------------------------------------------------------------

/// The portal's error codes (`Errors and limits`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiCode {
    InvalidInput,
    InvalidInvite,
    Forbidden,
    NotFound,
    Conflict,
    GroupFull,
    LimitReached,
    RateLimited,
    Internal,
}

impl ApiCode {
    fn parse(code: &str) -> Option<ApiCode> {
        Some(match code {
            "INVALID_INPUT" => ApiCode::InvalidInput,
            "INVALID_INVITE" => ApiCode::InvalidInvite,
            "FORBIDDEN" => ApiCode::Forbidden,
            "NOT_FOUND" => ApiCode::NotFound,
            "CONFLICT" => ApiCode::Conflict,
            "GROUP_FULL" => ApiCode::GroupFull,
            "LIMIT_REACHED" => ApiCode::LimitReached,
            "RATE_LIMITED" => ApiCode::RateLimited,
            "INTERNAL_ERROR" => ApiCode::Internal,
            _ => return None,
        })
    }
}

/// Why a call did not work, as the page needs to tell them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocialError {
    /// The portal has no social API (not deployed yet, or this build has no portal): a 404 that
    /// is not the API's own `NOT_FOUND`.
    NotAvailable,
    /// The portal cannot be reached (no network, a timeout, or a gateway error).
    Offline,
    /// No sign-in saved, or the portal does not accept it.
    SignedOut,
    /// The portal refused with one of its codes.
    Api(ApiCode),
    /// The portal's answer was not what the API says.
    Invalid,
}

pub type Result<T> = std::result::Result<T, SocialError>;

/// A failed reply (`status` and JSON body) as an error.
pub fn error_from(status: u16, body: &Value) -> SocialError {
    let code = body["error"]["code"].as_str();
    if code == Some("UNAUTHENTICATED") {
        return SocialError::SignedOut;
    }
    if let Some(code) = code.and_then(ApiCode::parse) {
        return SocialError::Api(code);
    }
    match status {
        401 => SocialError::SignedOut,
        403 => SocialError::Api(ApiCode::Forbidden),
        // Not the API's own answer: the route is not there.
        404 | 405 | 410 => SocialError::NotAvailable,
        429 => SocialError::Api(ApiCode::RateLimited),
        502..=504 => SocialError::Offline,
        500..=599 => SocialError::Api(ApiCode::Internal),
        _ => SocialError::Invalid,
    }
}

/// The day `days` from today (UTC) as the portal writes it: `2026-11-16`.
pub fn day_from_now(days: i64) -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    let z = secs.div_euclid(86_400) + days + 719_468;
    // Howard Hinnant's days-to-civil.
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

// ----- Codes ------------------------------------------------------------------------------

/// What a person typed or pasted as a `prefix` code, as the portal writes it (`YKF-ABCD-2345`),
/// or `None` when it cannot be one. Case, spaces and the hyphens do not matter.
pub fn normalize_code(input: &str, prefix: &str) -> Option<String> {
    let letters: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect::<String>()
        .to_ascii_uppercase();
    let body = letters.strip_prefix(prefix)?;
    if body.chars().count() != 8 || !body.chars().all(|c| ALPHABET.contains(c)) {
        return None;
    }
    Some(format!("{prefix}-{}-{}", &body[..4], &body[4..]))
}

/// A group's name as the portal takes it: trimmed, 1 to 40 characters, no control characters.
pub fn clean_group_name(input: &str) -> Option<String> {
    let name = input.trim();
    let length = name.chars().count();
    (length >= 1 && length <= MAX_GROUP_NAME && !name.chars().any(char::is_control))
        .then(|| name.to_string())
}

// ----- The backend ------------------------------------------------------------------------

/// Whom a friend request is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A friend code (`YKF-XXXX-XXXX`).
    Code(String),
    /// A principal of one's own groups.
    Principal(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Accept,
    Decline,
    Cancel,
}

/// What the Friends page asks of the portal. Every call may block: run them off the UI thread.
pub trait SocialBackend: Send + Sync {
    fn me(&self) -> Result<Me>;
    /// A new friend code; the old one stops working.
    fn rotate_friend_code(&self) -> Result<Me>;
    fn friends(&self) -> Result<FriendsList>;
    fn send_request(&self, target: &Target) -> Result<()>;
    fn answer_request(&self, id: &str, answer: Answer) -> Result<()>;
    fn remove_friend(&self, principal_id: &str) -> Result<()>;
    fn blocks(&self) -> Result<Vec<Block>>;
    fn block(&self, principal_id: &str) -> Result<()>;
    fn unblock(&self, principal_id: &str) -> Result<()>;
    fn groups(&self) -> Result<Vec<GroupSummary>>;
    fn create_group(&self, name: &str) -> Result<Group>;
    fn group(&self, id: &str) -> Result<GroupDetail>;
    fn delete_group(&self, id: &str) -> Result<()>;
    /// The invite and its code, which the portal shows only once.
    fn create_invite(&self, group: &str, max_uses: u32, expires_in_days: u32)
        -> Result<(Invite, String)>;
    fn revoke_invite(&self, group: &str, invite: &str) -> Result<()>;
    /// Joins with a `YKG-` code; the group's id.
    fn redeem_invite(&self, code: &str) -> Result<String>;
    fn set_role(&self, group: &str, principal_id: &str, role: Role) -> Result<()>;
    /// Removes a member (a kick that bans) or, with one's own id, leaves.
    fn remove_member(&self, group: &str, principal_id: &str) -> Result<()>;
    fn lift_ban(&self, group: &str, principal_id: &str) -> Result<()>;
    fn transfer_ownership(&self, group: &str, principal_id: &str) -> Result<()>;
    // ----- Rooms (`rooms.rs`) -----
    /// Opens a room. The host token in the answer comes only once.
    fn create_room(&self, audience: &Audience, name: Option<&str>) -> Result<HostedRoom>;
    /// A new ticket (the host only): the launcher asks shortly before the room's time is up
    /// and hands it to the game (`host.rs`).
    fn reprovision_room(&self, session_id: &str) -> Result<Reprovision>;
    /// The rooms the player may join.
    fn rooms(&self) -> Result<Vec<RoomInfo>>;
    /// Asks to join: the grant, good for about two minutes.
    fn join_room(&self, session_id: &str) -> Result<JoinGrant>;
    fn close_room(&self, session_id: &str) -> Result<()>;
    /// The guests who asked to join and were not acknowledged yet (the host only).
    fn pending_joins(&self, session_id: &str) -> Result<Vec<PendingJoin>>;
    /// Marks these grants (by nonce) as handed to the game: they leave `pending_joins`.
    fn ack_joins(&self, session_id: &str, nonces: &[String]) -> Result<()>;
}

/// A backend whose every call fails the same way: what a build with no portal has (`NO_PORTAL`),
/// and what `HORIZON_LAUNCHER_SOCIAL=signedout` shows for screenshots.
pub struct Failing(pub SocialError);

pub const NO_PORTAL: Failing = Failing(SocialError::NotAvailable);

macro_rules! unavailable {
    ($($name:ident($($arg:ident: $ty:ty),*) -> $ret:ty;)*) => {
        $(fn $name(&self, $($arg: $ty),*) -> $ret { let _ = ($(&$arg,)*); Err(self.0) })*
    };
}

impl SocialBackend for Failing {
    unavailable! {
        me() -> Result<Me>;
        rotate_friend_code() -> Result<Me>;
        friends() -> Result<FriendsList>;
        send_request(target: &Target) -> Result<()>;
        answer_request(id: &str, answer: Answer) -> Result<()>;
        remove_friend(principal_id: &str) -> Result<()>;
        blocks() -> Result<Vec<Block>>;
        block(principal_id: &str) -> Result<()>;
        unblock(principal_id: &str) -> Result<()>;
        groups() -> Result<Vec<GroupSummary>>;
        create_group(name: &str) -> Result<Group>;
        group(id: &str) -> Result<GroupDetail>;
        delete_group(id: &str) -> Result<()>;
        create_invite(group: &str, max_uses: u32, expires_in_days: u32) -> Result<(Invite, String)>;
        revoke_invite(group: &str, invite: &str) -> Result<()>;
        redeem_invite(code: &str) -> Result<String>;
        set_role(group: &str, principal_id: &str, role: Role) -> Result<()>;
        remove_member(group: &str, principal_id: &str) -> Result<()>;
        lift_ban(group: &str, principal_id: &str) -> Result<()>;
        transfer_ownership(group: &str, principal_id: &str) -> Result<()>;
        create_room(audience: &Audience, name: Option<&str>) -> Result<HostedRoom>;
        reprovision_room(session_id: &str) -> Result<Reprovision>;
        rooms() -> Result<Vec<RoomInfo>>;
        join_room(session_id: &str) -> Result<JoinGrant>;
        close_room(session_id: &str) -> Result<()>;
        pending_joins(session_id: &str) -> Result<Vec<PendingJoin>>;
        ack_joins(session_id: &str, nonces: &[String]) -> Result<()>;
    }
}

// ----- The portal over HTTP ---------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Patch,
    Delete,
}

impl Method {
    fn name(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Patch => "PATCH",
            Method::Delete => "DELETE",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportError {
    /// No sign-in saved or readable.
    SignedOut,
    /// Not reachable, or the answer was too big or not JSON where it had to be.
    Offline,
    /// The portal address is not usable.
    Configuration,
}

pub struct Reply {
    pub status: u16,
    pub body: Value,
}

/// One HTTP call to `api/social/v1/<segments>`. Segments are the path's parts, unescaped (the
/// transport escapes them: ids are opaque).
pub trait Transport: Send + Sync {
    fn call(
        &self,
        method: Method,
        segments: &[&str],
        body: Option<Value>,
    ) -> std::result::Result<Reply, TransportError>;
}

/// The real backend: the API's requests over any [`Transport`].
pub struct HttpSocial<T: Transport> {
    transport: T,
}

impl<T: Transport> HttpSocial<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    fn send(&self, method: Method, segments: &[&str], body: Option<Value>) -> Result<Value> {
        // An id that is empty, "." or ".." would change the path, not name a thing in it.
        if segments.iter().any(|s| matches!(*s, "" | "." | "..")) {
            return Err(SocialError::Invalid);
        }
        let reply = self
            .transport
            .call(method, segments, body)
            .map_err(|error| match error {
                TransportError::SignedOut => SocialError::SignedOut,
                TransportError::Offline => SocialError::Offline,
                TransportError::Configuration => SocialError::NotAvailable,
            })?;
        if (200..300).contains(&reply.status) {
            Ok(reply.body)
        } else {
            Err(error_from(reply.status, &reply.body))
        }
    }

    fn read<D: serde::de::DeserializeOwned>(
        &self,
        method: Method,
        segments: &[&str],
        body: Option<Value>,
        field: &str,
    ) -> Result<D> {
        let mut answer = self.send(method, segments, body)?;
        let value = if field.is_empty() {
            answer
        } else {
            answer
                .get_mut(field)
                .map(Value::take)
                .ok_or(SocialError::Invalid)?
        };
        serde_json::from_value(value).map_err(|_| SocialError::Invalid)
    }

    fn done(&self, method: Method, segments: &[&str], body: Option<Value>) -> Result<()> {
        self.send(method, segments, body).map(|_| ())
    }
}

impl<T: Transport> SocialBackend for HttpSocial<T> {
    fn me(&self) -> Result<Me> {
        self.read(Method::Get, &["me"], None, "principal")
    }

    fn rotate_friend_code(&self) -> Result<Me> {
        self.read(Method::Post, &["me", "friend-code"], None, "principal")
    }

    fn friends(&self) -> Result<FriendsList> {
        self.read(Method::Get, &["friends"], None, "")
    }

    fn send_request(&self, target: &Target) -> Result<()> {
        let body = match target {
            Target::Code(code) => json!({ "friendCode": code }),
            Target::Principal(id) => json!({ "principalId": id }),
        };
        self.done(Method::Post, &["friend-requests"], Some(body))
    }

    fn answer_request(&self, id: &str, answer: Answer) -> Result<()> {
        let action = match answer {
            Answer::Accept => "accept",
            Answer::Decline => "decline",
            Answer::Cancel => "cancel",
        };
        self.done(Method::Post, &["friend-requests", id, action], None)
    }

    fn remove_friend(&self, principal_id: &str) -> Result<()> {
        self.done(Method::Delete, &["friends", principal_id], None)
    }

    fn blocks(&self) -> Result<Vec<Block>> {
        self.read(Method::Get, &["blocks"], None, "blocks")
    }

    fn block(&self, principal_id: &str) -> Result<()> {
        self.done(
            Method::Post,
            &["blocks"],
            Some(json!({ "principalId": principal_id })),
        )
    }

    fn unblock(&self, principal_id: &str) -> Result<()> {
        self.done(
            Method::Delete,
            &["blocks"],
            Some(json!({ "principalId": principal_id })),
        )
    }

    fn groups(&self) -> Result<Vec<GroupSummary>> {
        self.read(Method::Get, &["groups"], None, "groups")
    }

    fn create_group(&self, name: &str) -> Result<Group> {
        self.read(Method::Post, &["groups"], Some(json!({ "name": name })), "group")
    }

    fn group(&self, id: &str) -> Result<GroupDetail> {
        self.read(Method::Get, &["groups", id], None, "group")
    }

    fn delete_group(&self, id: &str) -> Result<()> {
        self.done(Method::Delete, &["groups", id], None)
    }

    fn create_invite(
        &self,
        group: &str,
        max_uses: u32,
        expires_in_days: u32,
    ) -> Result<(Invite, String)> {
        let mut answer = self.send(
            Method::Post,
            &["groups", group, "invites"],
            Some(json!({ "maxUses": max_uses, "expiresInDays": expires_in_days })),
        )?;
        let invite = answer
            .get_mut("invite")
            .map(Value::take)
            .ok_or(SocialError::Invalid)
            .and_then(|v| serde_json::from_value(v).map_err(|_| SocialError::Invalid))?;
        let code = answer["code"]
            .as_str()
            .map(str::to_string)
            .ok_or(SocialError::Invalid)?;
        Ok((invite, code))
    }

    fn revoke_invite(&self, group: &str, invite: &str) -> Result<()> {
        self.done(Method::Delete, &["groups", group, "invites", invite], None)
    }

    fn redeem_invite(&self, code: &str) -> Result<String> {
        let answer = self.send(
            Method::Post,
            &["group-invites", "redeem"],
            Some(json!({ "code": code })),
        )?;
        answer["groupId"]
            .as_str()
            .map(str::to_string)
            .ok_or(SocialError::Invalid)
    }

    fn set_role(&self, group: &str, principal_id: &str, role: Role) -> Result<()> {
        self.done(
            Method::Patch,
            &["groups", group, "members", principal_id],
            Some(json!({ "role": role.word() })),
        )
    }

    fn remove_member(&self, group: &str, principal_id: &str) -> Result<()> {
        self.done(
            Method::Delete,
            &["groups", group, "members", principal_id],
            None,
        )
    }

    fn lift_ban(&self, group: &str, principal_id: &str) -> Result<()> {
        self.done(
            Method::Delete,
            &["groups", group, "bans", principal_id],
            None,
        )
    }

    fn transfer_ownership(&self, group: &str, principal_id: &str) -> Result<()> {
        self.done(
            Method::Post,
            &["groups", group, "transfer"],
            Some(json!({ "principalId": principal_id })),
        )
    }

    fn create_room(&self, audience: &Audience, name: Option<&str>) -> Result<HostedRoom> {
        let body = serde_json::to_value(CreateRequest { audience, name })
            .map_err(|_| SocialError::Invalid)?;
        self.read(Method::Post, &["sessions"], Some(body), "")
    }

    fn reprovision_room(&self, session_id: &str) -> Result<Reprovision> {
        self.read(Method::Post, &["sessions", session_id, "provision"], None, "")
    }

    fn rooms(&self) -> Result<Vec<RoomInfo>> {
        self.read(Method::Get, &["sessions"], None, "sessions")
    }

    fn join_room(&self, session_id: &str) -> Result<JoinGrant> {
        self.read(Method::Post, &["sessions", session_id, "join"], None, "")
    }

    fn close_room(&self, session_id: &str) -> Result<()> {
        self.done(Method::Delete, &["sessions", session_id], None)
    }

    fn pending_joins(&self, session_id: &str) -> Result<Vec<PendingJoin>> {
        let pending: Pending =
            self.read(Method::Get, &["sessions", session_id, "pending"], None, "")?;
        Ok(pending.joins)
    }

    fn ack_joins(&self, session_id: &str, nonces: &[String]) -> Result<()> {
        self.done(
            Method::Post,
            &["sessions", session_id, "pending", "ack"],
            Some(json!({ "nonces": nonces })),
        )
    }
}

/// A reply this big is not the API's (a group of 100 with its invites and bans is about 60 KB).
const MAX_REPLY: u64 = 512 * 1024;

/// The real transport: HTTPS to the portal, with the saved sign-in as the bearer.
pub struct UreqTransport {
    root: Url,
    auth: Box<dyn YoakeAuth>,
}

impl UreqTransport {
    /// The portal this launcher was built for, with the sign-in saved in `data_dir`.
    pub fn configured(data_dir: &std::path::Path) -> Option<Self> {
        let portal = option_env!("YOAKE_PORTAL_URL")?;
        Self::new(portal, Box::new(TokenFileAuth::new(data_dir))).ok()
    }

    pub fn new(portal: &str, auth: Box<dyn YoakeAuth>) -> std::result::Result<Self, TransportError> {
        let mut root = Url::parse(portal).map_err(|_| TransportError::Configuration)?;
        if root.scheme() != "https"
            || root.host_str().is_none()
            || !root.username().is_empty()
            || root.password().is_some()
            || root.query().is_some()
            || root.fragment().is_some()
        {
            return Err(TransportError::Configuration);
        }
        if !root.path().ends_with('/') {
            root.set_path(&format!("{}/", root.path()));
        }
        Ok(Self { root, auth })
    }

    /// `<portal>/api/social/v1/<segments>`, each segment escaped.
    fn url(&self, segments: &[&str]) -> std::result::Result<Url, TransportError> {
        let mut url = self
            .root
            .join("api/social/v1/")
            .map_err(|_| TransportError::Configuration)?;
        url.path_segments_mut()
            .map_err(|()| TransportError::Configuration)?
            .pop_if_empty()
            .extend(segments);
        Ok(url)
    }
}

impl Transport for UreqTransport {
    fn call(
        &self,
        method: Method,
        segments: &[&str],
        body: Option<Value>,
    ) -> std::result::Result<Reply, TransportError> {
        let url = self.url(segments)?;
        let authorization = match self.auth.authorization() {
            Ok(Some(value)) => value,
            Ok(None) | Err(AuthError::InvalidToken) => return Err(TransportError::SignedOut),
            Err(_) => return Err(TransportError::SignedOut),
        };
        let builder = ureq::AgentBuilder::new()
            .redirects(0)
            .timeout_connect(Duration::from_secs(15))
            .timeout(Duration::from_secs(25));
        let agent = horizon_launcher::proxy::agent(builder, url.as_str()).build();
        let request = agent
            .request(method.name(), url.as_str())
            .set("Authorization", &authorization)
            .set("Accept", "application/json");
        let response = match body {
            Some(body) => request.send_json(body),
            None => request.call(),
        };
        let response = match response {
            Ok(response) | Err(ureq::Error::Status(_, response)) => response,
            Err(ureq::Error::Transport(_)) => return Err(TransportError::Offline),
        };
        let status = response.status();
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(MAX_REPLY + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| TransportError::Offline)?;
        if bytes.len() as u64 > MAX_REPLY {
            return Err(TransportError::Offline);
        }
        Ok(reply_from(status, &bytes))
    }
}

/// A reply from its status and body bytes.
fn reply_from(status: u16, bytes: &[u8]) -> Reply {
    let ok = (200..300).contains(&status);
    let body = match serde_json::from_slice(bytes) {
        Ok(body) => body,
        // A 204 (closing a room) has no body.
        Err(_) if bytes.is_empty() && ok => Value::Null,
        // A page that is not JSON (a proxy's 404, a gateway error) still has its status.
        Err(_) if !ok => Value::Null,
        Err(_) => return Reply { status: 502, body: Value::Null },
    };
    Reply { status, body }
}

/// The backend for this launcher: the real portal when the build has one.
pub fn configured(data_dir: &std::path::Path) -> std::sync::Arc<dyn SocialBackend> {
    match std::env::var("HORIZON_LAUNCHER_SOCIAL").as_deref() {
        Ok("mock") => {
            // `HORIZON_LAUNCHER_SOCIAL_ROOM=open`: the player already has a room open.
            let mock = crate::social_mock::MockSocial::sample();
            let mock = match std::env::var("HORIZON_LAUNCHER_SOCIAL_ROOM").as_deref() {
                Ok("open") => mock.with_my_room_open(),
                _ => mock,
            };
            return std::sync::Arc::new(mock);
        }
        Ok("unavailable") => return std::sync::Arc::new(NO_PORTAL),
        Ok("signedout") => return std::sync::Arc::new(Failing(SocialError::SignedOut)),
        Ok("offline") => return std::sync::Arc::new(Failing(SocialError::Offline)),
        _ => {}
    }
    match UreqTransport::configured(data_dir) {
        Some(transport) => std::sync::Arc::new(HttpSocial::new(transport)),
        None => std::sync::Arc::new(NO_PORTAL),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Records the calls and answers from a script.
    struct Script {
        calls: Mutex<Vec<(Method, Vec<String>, Option<Value>)>>,
        replies: Mutex<Vec<std::result::Result<Reply, TransportError>>>,
    }

    impl Script {
        fn new(replies: Vec<std::result::Result<Reply, TransportError>>) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                replies: Mutex::new(replies.into_iter().rev().collect()),
            }
        }
    }

    impl Transport for &Script {
        fn call(
            &self,
            method: Method,
            segments: &[&str],
            body: Option<Value>,
        ) -> std::result::Result<Reply, TransportError> {
            self.calls.lock().unwrap().push((
                method,
                segments.iter().map(|s| s.to_string()).collect(),
                body,
            ));
            self.replies.lock().unwrap().pop().expect("a scripted reply")
        }
    }

    fn ok(body: Value) -> std::result::Result<Reply, TransportError> {
        Ok(Reply { status: 200, body })
    }

    fn principal(id: &str, name: &str) -> Value {
        json!({ "id": id, "kind": "member", "displayName": name })
    }

    #[test]
    fn every_call_is_the_documented_method_path_and_body() {
        let script = Script::new((0..20).map(|_| ok(json!({ "ok": true }))).collect());
        let api = HttpSocial::new(&script);
        // The reads need a shape: those calls are checked below. Writes answer `{ok: true}`.
        api.send_request(&Target::Code("YKF-ABCD-2345".into())).unwrap();
        api.send_request(&Target::Principal("p1".into())).unwrap();
        api.answer_request("r1", Answer::Accept).unwrap();
        api.answer_request("r1", Answer::Decline).unwrap();
        api.answer_request("r1", Answer::Cancel).unwrap();
        api.remove_friend("p1").unwrap();
        api.block("p1").unwrap();
        api.unblock("p1").unwrap();
        api.delete_group("g1").unwrap();
        api.revoke_invite("g1", "i1").unwrap();
        api.set_role("g1", "p1", Role::Admin).unwrap();
        api.set_role("g1", "p1", Role::Member).unwrap();
        api.remove_member("g1", "p1").unwrap();
        api.lift_ban("g1", "p1").unwrap();
        api.transfer_ownership("g1", "p1").unwrap();
        let calls = script.calls.lock().unwrap();
        let line = |i: usize| (calls[i].0, calls[i].1.join("/"), calls[i].2.clone());
        assert_eq!(
            line(0),
            (
                Method::Post,
                "friend-requests".into(),
                Some(json!({ "friendCode": "YKF-ABCD-2345" }))
            )
        );
        assert_eq!(line(1).2, Some(json!({ "principalId": "p1" })));
        assert_eq!(line(2), (Method::Post, "friend-requests/r1/accept".into(), None));
        assert_eq!(line(3).1, "friend-requests/r1/decline");
        assert_eq!(line(4).1, "friend-requests/r1/cancel");
        assert_eq!(line(5), (Method::Delete, "friends/p1".into(), None));
        assert_eq!(
            line(6),
            (Method::Post, "blocks".into(), Some(json!({ "principalId": "p1" })))
        );
        assert_eq!(
            line(7),
            (Method::Delete, "blocks".into(), Some(json!({ "principalId": "p1" })))
        );
        assert_eq!(line(8), (Method::Delete, "groups/g1".into(), None));
        assert_eq!(line(9), (Method::Delete, "groups/g1/invites/i1".into(), None));
        assert_eq!(
            line(10),
            (
                Method::Patch,
                "groups/g1/members/p1".into(),
                Some(json!({ "role": "admin" }))
            )
        );
        assert_eq!(line(11).2, Some(json!({ "role": "member" })));
        assert_eq!(line(12), (Method::Delete, "groups/g1/members/p1".into(), None));
        assert_eq!(line(13), (Method::Delete, "groups/g1/bans/p1".into(), None));
        assert_eq!(
            line(14),
            (
                Method::Post,
                "groups/g1/transfer".into(),
                Some(json!({ "principalId": "p1" }))
            )
        );
    }

    #[test]
    fn ids_that_change_the_path_are_refused_without_a_call() {
        let script = Script::new(vec![]);
        let api = HttpSocial::new(&script);
        for id in ["", ".", ".."] {
            assert_eq!(api.remove_friend(id), Err(SocialError::Invalid));
            assert_eq!(api.delete_group(id), Err(SocialError::Invalid));
            assert_eq!(api.revoke_invite("g1", id), Err(SocialError::Invalid));
            assert_eq!(api.remove_member(id, "p1"), Err(SocialError::Invalid));
        }
        assert!(script.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn an_invite_answer_that_is_not_an_object_is_invalid_not_a_panic() {
        for body in [json!([1]), json!("x"), json!(7), json!({ "code": "YKG-ABCD-2345" })] {
            let script = Script::new(vec![ok(body)]);
            let api = HttpSocial::new(&script);
            assert_eq!(api.create_invite("g1", 1, 1).err(), Some(SocialError::Invalid));
        }
    }

    #[test]
    fn reads_take_the_documented_shapes() {
        let script = Script::new(vec![
            ok(json!({ "principal": { "id": "me", "kind": "member", "displayName": "Ann",
                "friendCode": "YKF-ABCD-2345", "hasFriendCode": true } })),
            ok(json!({
                "friends": [{ "principal": principal("a", "Bo"), "createdAt": "2026-11-15T00:00:00.000Z" }],
                "incoming": [{ "id": "r1", "fromId": "c", "toId": "me", "status": "pending",
                    "createdAt": "2026-11-15T00:00:00.000Z", "from": principal("c", "Cy"), "to": principal("me", "Ann") }],
                "outgoing": []
            })),
            ok(json!({ "blocks": [{ "blockerId": "me", "blockedId": "d",
                "createdAt": "2026-11-15T00:00:00.000Z", "blocked": principal("d", "Di") }] })),
            ok(json!({ "groups": [{ "id": "g1", "name": "Crew", "ownerId": "me",
                "createdAt": "2026-11-15T00:00:00.000Z", "role": "owner", "joinedAt": "2026-11-15T00:00:00.000Z" }] })),
            ok(json!({ "group": { "id": "g1", "name": "Crew", "ownerId": "me",
                "createdAt": "2026-11-15T00:00:00.000Z", "role": "owner", "canRecoverOwnership": false,
                "members": [{ "groupId": "g1", "principalId": "me", "role": "owner",
                    "joinedAt": "2026-11-15T00:00:00.000Z", "principal": principal("me", "Ann") }],
                "invites": [{ "id": "i1", "groupId": "g1", "createdById": "me", "maxUses": 3, "uses": 0,
                    "createdAt": "2026-11-15T00:00:00.000Z", "expiresAt": "2026-11-22T00:00:00.000Z", "revokedAt": null }],
                "bans": [] } })),
            Ok(Reply {
                status: 201,
                body: json!({ "invite": { "id": "i2", "groupId": "g1", "createdById": "me", "maxUses": 5,
                    "uses": 0, "createdAt": "2026-11-15T00:00:00.000Z",
                    "expiresAt": "2026-11-22T00:00:00.000Z", "revokedAt": null }, "code": "YKG-ABCD-2345" }),
            }),
            ok(json!({ "groupId": "g9" })),
        ]);
        let api = HttpSocial::new(&script);
        let me = api.me().unwrap();
        assert_eq!((me.display_name.as_str(), me.friend_code.as_str()), ("Ann", "YKF-ABCD-2345"));
        let friends = api.friends().unwrap();
        assert_eq!(friends.friends[0].principal.display_name, "Bo");
        assert_eq!(friends.incoming[0].from.display_name, "Cy");
        assert_eq!(api.blocks().unwrap()[0].blocked.id, "d");
        assert_eq!(api.groups().unwrap()[0].role, Role::Owner);
        let detail = api.group("g1").unwrap();
        assert_eq!((detail.members.len(), detail.invites[0].max_uses), (1, 3));
        let (invite, code) = api.create_invite("g1", 5, 7).unwrap();
        assert_eq!((invite.id.as_str(), code.as_str()), ("i2", "YKG-ABCD-2345"));
        assert_eq!(api.redeem_invite("YKG-ABCD-2345").unwrap(), "g9");
        let calls = script.calls.lock().unwrap();
        assert_eq!(
            calls[5].2,
            Some(json!({ "maxUses": 5, "expiresInDays": 7 })),
            "invite options"
        );
        assert_eq!(calls[0].1, ["me"]);
    }

    #[test]
    fn the_room_calls_are_the_portals() {
        use rooms::Audience;
        let hosted = json!({
            "session_id": "s1", "host_principal": "p1", "relay_only": true, "host_token": "T",
            "provision": { "ticket": { "n": 1 }, "signature": "S" },
            "relay": "relay.example:7000", "expires_at": 1_800_000_000
        });
        let script = Script::new(vec![
            Ok(Reply { status: 201, body: hosted.clone() }),
            Ok(Reply { status: 201, body: hosted }),
            ok(json!({ "provision": { "ticket": {}, "signature": "S2" }, "relay": "r:1" })),
            ok(json!({ "sessions": [{ "session_id": "a", "host": { "principal": "p", "name": "Sora" },
                "audience": { "group": "g", "name": "Crew" }, "name": null, "created_at": 1, "expires_at": 2, "mine": false }] })),
            ok(json!({ "grant": { "g": 1 }, "signature": "S3", "relay": "r:2" })),
            Ok(Reply { status: 204, body: Value::Null }),
        ]);
        let api = HttpSocial::new(&script);
        api.create_room(&Audience::Friends, None).unwrap();
        let group = Audience::Group { id: "g1".into(), name: Some("ignored".into()) };
        api.create_room(&group, Some("Night")).unwrap();
        assert_eq!(api.reprovision_room("s1").unwrap().relay, "r:1");
        let list = api.rooms().unwrap();
        assert_eq!(list[0].host.name, "Sora");
        assert_eq!(api.join_room("a").unwrap().relay, "r:2");
        api.close_room("s1").unwrap();
        let calls = script.calls.lock().unwrap();
        let line = |i: usize| (calls[i].0, calls[i].1.join("/"), calls[i].2.clone());
        assert_eq!(
            line(0),
            (Method::Post, "sessions".into(), Some(json!({ "audience": "friends" })))
        );
        assert_eq!(
            line(1).2,
            Some(json!({ "audience": { "group": "g1" }, "name": "Night" }))
        );
        assert_eq!(line(2), (Method::Post, "sessions/s1/provision".into(), None));
        assert_eq!(line(3), (Method::Get, "sessions".into(), None));
        assert_eq!(line(4), (Method::Post, "sessions/a/join".into(), None));
        assert_eq!(line(5), (Method::Delete, "sessions/s1".into(), None));
    }

    #[test]
    fn the_pending_and_ack_calls_are_the_portals() {
        let nonce = "a".repeat(64);
        let script = Script::new(vec![
            ok(json!({ "joins": [
                { "grant": { "nonce": nonce, "session_id": "s1" }, "signature": "ab".repeat(64),
                  "guest": { "principal": "g", "name": "Mika" }, "expires_at": 5, "new": 1 },
                { "grant": { "nonce": "short" }, "signature": "s",
                  "guest": { "principal": "g", "name": "Bad" }, "expires_at": 5 }
            ] })),
            Ok(Reply { status: 204, body: Value::Null }),
            Ok(Reply { status: 404, body: json!({ "error": { "code": "NOT_FOUND" } }) }),
            Ok(Reply { status: 404, body: json!({ "error": { "code": "NOT_FOUND" } }) }),
            Ok(Reply { status: 401, body: json!({ "error": { "code": "UNAUTHENTICATED" } }) }),
        ]);
        let api = HttpSocial::new(&script);
        let joins = api.pending_joins("s1").unwrap();
        // The one with a nonce that is not 64 lowercase hex digits is left out.
        assert_eq!(joins.len(), 1);
        assert_eq!(joins[0].guest.name, "Mika");
        api.ack_joins("s1", &[nonce.clone()]).unwrap();
        assert_eq!(api.pending_joins("gone").err(), Some(SocialError::Api(ApiCode::NotFound)));
        assert_eq!(api.ack_joins("gone", &[]).err(), Some(SocialError::Api(ApiCode::NotFound)));
        assert_eq!(api.pending_joins("s1").err(), Some(SocialError::SignedOut));
        let calls = script.calls.lock().unwrap();
        assert_eq!((calls[0].0, calls[0].1.join("/"), calls[0].2.clone()), (Method::Get, "sessions/s1/pending".into(), None));
        assert_eq!(
            (calls[1].0, calls[1].1.join("/"), calls[1].2.clone()),
            (Method::Post, "sessions/s1/pending/ack".into(), Some(json!({ "nonces": [nonce] })))
        );
    }

    #[test]
    fn room_answers_that_are_not_the_apis_are_invalid_and_ids_stay_in_the_path() {
        let script = Script::new(vec![
            ok(json!({ "session_id": "s1" })),
            ok(json!({ "sessions": [{ "session_id": "a" }] })),
            Ok(Reply { status: 404, body: json!({ "error": { "code": "NOT_FOUND" } }) }),
            Ok(Reply { status: 403, body: json!({ "error": { "code": "FORBIDDEN" } }) }),
        ]);
        let api = HttpSocial::new(&script);
        assert_eq!(api.create_room(&rooms::Audience::Friends, None).err(), Some(SocialError::Invalid));
        assert_eq!(api.rooms().err(), Some(SocialError::Invalid));
        assert_eq!(api.join_room("gone").err(), Some(SocialError::Api(ApiCode::NotFound)));
        assert_eq!(api.join_room("x").err(), Some(SocialError::Api(ApiCode::Forbidden)));
        for id in ["", ".", ".."] {
            assert_eq!(api.close_room(id), Err(SocialError::Invalid));
            assert_eq!(api.join_room(id).err(), Some(SocialError::Invalid));
        }
        // No answer with a room in it puts it in the error.
        assert!(format!("{:?}", SocialError::Invalid).len() < 20);
    }

    #[test]
    fn a_204_has_no_body_and_a_page_that_is_not_json_keeps_its_status() {
        let closed = reply_from(204, b"");
        assert_eq!((closed.status, closed.body), (204, Value::Null));
        assert_eq!(reply_from(200, br#"{"a":1}"#).body, json!({ "a": 1 }));
        assert_eq!(reply_from(404, b"<html>").status, 404);
        assert_eq!(reply_from(200, b"<html>").status, 502);
    }

    #[test]
    fn errors_say_what_the_portal_said() {
        let body = |code: &str| json!({ "error": { "code": code } });
        assert_eq!(error_from(401, &body("UNAUTHENTICATED")), SocialError::SignedOut);
        assert_eq!(error_from(400, &body("INVALID_INVITE")), SocialError::Api(ApiCode::InvalidInvite));
        assert_eq!(error_from(409, &body("GROUP_FULL")), SocialError::Api(ApiCode::GroupFull));
        assert_eq!(error_from(409, &body("LIMIT_REACHED")), SocialError::Api(ApiCode::LimitReached));
        assert_eq!(error_from(429, &body("RATE_LIMITED")), SocialError::Api(ApiCode::RateLimited));
        // The API's own NOT_FOUND is not "the API is not there".
        assert_eq!(error_from(404, &body("NOT_FOUND")), SocialError::Api(ApiCode::NotFound));
        // Anything else is by status: a 404 page means no API yet.
        assert_eq!(error_from(404, &Value::Null), SocialError::NotAvailable);
        assert_eq!(error_from(404, &json!({ "error": "not found" })), SocialError::NotAvailable);
        assert_eq!(error_from(401, &Value::Null), SocialError::SignedOut);
        assert_eq!(error_from(403, &json!({ "error": "origin" })), SocialError::Api(ApiCode::Forbidden));
        assert_eq!(error_from(502, &Value::Null), SocialError::Offline);
        assert_eq!(error_from(503, &Value::Null), SocialError::Offline);
        assert_eq!(error_from(500, &Value::Null), SocialError::Api(ApiCode::Internal));
        assert_eq!(error_from(418, &Value::Null), SocialError::Invalid);
    }

    #[test]
    fn transport_failures_and_odd_answers_become_states() {
        let script = Script::new(vec![
            Err(TransportError::Offline),
            Err(TransportError::SignedOut),
            Ok(Reply { status: 404, body: Value::Null }),
            ok(json!({ "nonsense": 1 })),
            ok(json!({ "principal": { "id": 7 } })),
        ]);
        let api = HttpSocial::new(&script);
        assert_eq!(api.me(), Err(SocialError::Offline));
        assert_eq!(api.friends(), Err(SocialError::SignedOut));
        assert_eq!(api.groups(), Err(SocialError::NotAvailable));
        assert_eq!(api.blocks(), Err(SocialError::Invalid));
        assert_eq!(api.me(), Err(SocialError::Invalid));
    }

    #[test]
    fn codes_are_read_loosely_and_written_the_portals_way() {
        assert_eq!(normalize_code("YKF-ABCD-2345", "YKF").as_deref(), Some("YKF-ABCD-2345"));
        assert_eq!(normalize_code("  ykf abcd 2345 ", "YKF").as_deref(), Some("YKF-ABCD-2345"));
        assert_eq!(normalize_code("YKFABCD2345", "YKF").as_deref(), Some("YKF-ABCD-2345"));
        assert_eq!(normalize_code("YKG-ABCD-2345", "YKG").as_deref(), Some("YKG-ABCD-2345"));
        // The wrong kind, too short, and letters the alphabet leaves out (0, 1, I, O).
        assert_eq!(normalize_code("YKG-ABCD-2345", "YKF"), None);
        assert_eq!(normalize_code("YKF-ABCD-234", "YKF"), None);
        assert_eq!(normalize_code("YKF-ABCD-23450", "YKF"), None);
        assert_eq!(normalize_code("YKF-ABCD-2O45", "YKF"), None);
        assert_eq!(normalize_code("YKF-ABCD-2145", "YKF"), None);
        assert_eq!(normalize_code("", "YKF"), None);
        assert_eq!(clean_group_name("  Crew  ").as_deref(), Some("Crew"));
        assert_eq!(clean_group_name("   "), None);
        assert_eq!(clean_group_name(&"あ".repeat(40)).map(|s| s.chars().count()), Some(40));
        assert_eq!(clean_group_name(&"あ".repeat(41)), None);
        assert_eq!(clean_group_name("a\u{7}b"), None);
    }

    #[test]
    fn the_url_escapes_ids_and_stays_on_the_portal() {
        struct Nobody;
        impl YoakeAuth for Nobody {
            fn authorization(&self) -> std::result::Result<Option<String>, AuthError> {
                Ok(None)
            }
        }
        let transport = UreqTransport::new("https://portal.example/base", Box::new(Nobody)).unwrap();
        assert_eq!(
            transport.url(&["groups", "a/b?c#d", "members"]).unwrap().as_str(),
            "https://portal.example/base/api/social/v1/groups/a%2Fb%3Fc%23d/members"
        );
        assert!(UreqTransport::new("http://portal.example", Box::new(Nobody)).is_err());
        assert!(UreqTransport::new("https://u:p@portal.example", Box::new(Nobody)).is_err());
        // Not signed in: no request is made.
        assert_eq!(
            transport.call(Method::Get, &["me"], None).err(),
            Some(TransportError::SignedOut)
        );
    }

    #[test]
    fn a_build_without_a_portal_has_no_social() {
        assert_eq!(NO_PORTAL.me(), Err(SocialError::NotAvailable));
        assert_eq!(NO_PORTAL.redeem_invite("x"), Err(SocialError::NotAvailable));
        assert_eq!(Failing(SocialError::SignedOut).friends(), Err(SocialError::SignedOut));
    }
}
