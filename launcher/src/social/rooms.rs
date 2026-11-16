//! Rooms: a friend joins the host's game through the portal's relay (`/api/social/v1/sessions`).
//!
//! The portal's shapes, in one place (agreed with the portal's design). Unknown fields are
//! ignored (the portal may add some); a required field that is missing makes the whole answer
//! invalid.
//!
//! Secrets (`host_token`, tickets, grants, signatures) are wrapped in [`Redacted`], whose `Debug`
//! prints a mask, so no `{:?}` of a room, an answer or an error carries them. They go to the
//! game by a private file only (`handoff.rs`).
use serde::de::{self, Deserializer};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

/// A value that must not reach a log, an error message or a `Debug` print.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Redacted<T>(T);

impl<T> Redacted<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// The secret itself: only for writing the hand-off file.
    pub fn expose(&self) -> &T {
        &self.0
    }
}

impl<T> fmt::Debug for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Who may join a room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Audience {
    Friends,
    /// A group of the player's, by id; the name comes with the portal's list.
    Group { id: String, name: Option<String> },
}

impl Serialize for Audience {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Audience::Friends => serializer.serialize_str("friends"),
            Audience::Group { id, .. } => {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("group", id)?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Audience {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Word(String),
            Group {
                group: String,
                #[serde(default)]
                name: Option<String>,
            },
        }
        match Raw::deserialize(deserializer)? {
            Raw::Word(word) if word == "friends" => Ok(Audience::Friends),
            Raw::Word(_) => Err(de::Error::custom("unknown audience")),
            Raw::Group { group, name } if !group.is_empty() => {
                Ok(Audience::Group { id: group, name })
            }
            Raw::Group { .. } => Err(de::Error::custom("empty group")),
        }
    }
}

/// `POST /sessions`' body.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CreateRequest<'a> {
    pub audience: &'a Audience,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<&'a str>,
}

/// What the relay is given to open the room: the portal's ticket and its signature.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Provision {
    pub ticket: Redacted<Value>,
    pub signature: Redacted<String>,
}

/// `host:port`, as the portal writes the relay's address.
fn relay<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let relay = String::deserialize(deserializer)?;
    let ok = !relay.is_empty()
        && relay.len() <= 300
        && !relay.chars().any(|c| c.is_whitespace() || c.is_control());
    if ok {
        Ok(relay)
    } else {
        Err(de::Error::custom("bad relay"))
    }
}

fn object<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Redacted<Value>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    if value.is_object() {
        Ok(Redacted(value))
    } else {
        Err(de::Error::custom("not an object"))
    }
}

/// `POST /sessions`' answer. The host token comes once: it is not on any later answer.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct HostedRoom {
    pub session_id: String,
    pub host_principal: String,
    #[serde(default)]
    pub relay_only: bool,
    pub host_token: Redacted<String>,
    pub provision: Provision,
    #[serde(deserialize_with = "relay")]
    pub relay: String,
    /// Unix seconds.
    pub expires_at: i64,
}

/// `POST /sessions/:id/provision`' answer: a new ticket, for a relay that restarted.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Reprovision {
    pub provision: Provision,
    #[serde(deserialize_with = "relay")]
    pub relay: String,
}

/// `POST /sessions/:id/join`' answer: the grant (good for about 120 seconds) and its signature.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct JoinGrant {
    #[serde(deserialize_with = "object")]
    pub grant: Redacted<Value>,
    pub signature: Redacted<String>,
    #[serde(deserialize_with = "relay")]
    pub relay: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct RoomHost {
    pub principal: String,
    pub name: String,
}

/// One room of `GET /sessions`: one the player may join.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct RoomInfo {
    pub session_id: String,
    pub host: RoomHost,
    pub audience: Audience,
    #[serde(default)]
    pub name: Option<String>,
    /// Unix seconds (a number: text is refused).
    pub created_at: i64,
    #[serde(default)]
    pub expires_at: Option<i64>,
    /// The portal marks the player's own room.
    #[serde(default)]
    pub mine: bool,
}

/// The player's own open room, as the page keeps it: what to show and what to close.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MyRoom {
    pub session_id: String,
    pub audience: Audience,
    pub name: Option<String>,
    /// Unix seconds the room was opened (the launcher's clock, or the portal's `created_at`).
    pub opened_at: i64,
    /// Unix seconds the portal closes it by, when it said.
    pub expires_at: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hosted() -> Value {
        json!({
            "session_id": "s1", "host_principal": "p1", "relay_only": true,
            "host_token": "TOP-SECRET-TOKEN",
            "provision": { "ticket": { "n": "TICKET-SECRET" }, "signature": "SIG-SECRET" },
            "relay": "relay.example:7000", "expires_at": 1_800_000_000
        })
    }

    #[test]
    fn the_create_request_is_the_portals() {
        let friends = CreateRequest { audience: &Audience::Friends, name: None };
        assert_eq!(serde_json::to_value(friends).unwrap(), json!({ "audience": "friends" }));
        let group = Audience::Group { id: "g1".into(), name: Some("Crew".into()) };
        let named = CreateRequest { audience: &group, name: Some("Night run") };
        assert_eq!(
            serde_json::to_value(named).unwrap(),
            json!({ "audience": { "group": "g1" }, "name": "Night run" })
        );
    }

    #[test]
    fn answers_are_read_and_unknown_fields_are_ignored() {
        let mut body = hosted();
        body["something_new"] = json!({ "x": 1 });
        body["provision"]["extra"] = json!(1);
        let room: HostedRoom = serde_json::from_value(body).unwrap();
        assert_eq!((room.session_id.as_str(), room.relay.as_str()), ("s1", "relay.example:7000"));
        assert_eq!(room.expires_at, 1_800_000_000);

        let join: JoinGrant = serde_json::from_value(json!({
            "grant": { "g": 1 }, "signature": "s", "relay": "r.example:1", "more": true
        }))
        .unwrap();
        assert_eq!(join.relay, "r.example:1");
        let again: Reprovision = serde_json::from_value(json!({
            "provision": { "ticket": {}, "signature": "s" }, "relay": "r:1"
        }))
        .unwrap();
        assert_eq!(again.relay, "r:1");

        let list: Vec<RoomInfo> = serde_json::from_value(json!([
            { "session_id": "a", "host": { "principal": "p", "name": "Sora", "x": 1 },
              "audience": "friends", "name": null, "created_at": 1_800_000_000, "expires_at": 5, "mine": true },
            { "session_id": "b", "host": { "principal": "q", "name": "Mika" },
              "audience": { "group": "g", "name": "Crew" }, "created_at": 7 }
        ]))
        .unwrap();
        assert_eq!(list[0].audience, Audience::Friends);
        assert_eq!(list[1].audience, Audience::Group { id: "g".into(), name: Some("Crew".into()) });
        assert_eq!(list[0].created_at, 1_800_000_000);
        assert!(list[0].mine && !list[1].mine);
        // Times are numbers: text is refused.
        assert!(serde_json::from_value::<RoomInfo>(json!({
            "session_id": "a", "host": { "principal": "p", "name": "n" },
            "audience": "friends", "created_at": "2026-11-16T01:02:03Z"
        }))
        .is_err());
        let mut text = hosted();
        text["expires_at"] = json!("2026-11-16T01:02:03Z");
        assert!(serde_json::from_value::<HostedRoom>(text).is_err());
    }

    #[test]
    fn a_missing_required_field_is_an_error() {
        for field in ["session_id", "host_principal", "host_token", "provision", "relay", "expires_at"] {
            let mut body = hosted();
            body.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<HostedRoom>(body).is_err(), "{field}");
        }
        for field in ["ticket", "signature"] {
            let mut body = hosted();
            body["provision"].as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<HostedRoom>(body).is_err(), "{field}");
        }
        assert!(serde_json::from_value::<JoinGrant>(json!({ "grant": {}, "relay": "r:1" })).is_err());
        assert!(serde_json::from_value::<JoinGrant>(json!({ "signature": "s", "relay": "r:1" })).is_err());
        // A grant that is not an object, a relay that is not host:port text, an audience unknown.
        assert!(serde_json::from_value::<JoinGrant>(
            json!({ "grant": "x", "signature": "s", "relay": "r:1" })
        )
        .is_err());
        assert!(serde_json::from_value::<JoinGrant>(
            json!({ "grant": {}, "signature": "s", "relay": "a b" })
        )
        .is_err());
        assert!(serde_json::from_value::<RoomInfo>(json!({
            "session_id": "a", "host": { "principal": "p", "name": "n" },
            "audience": "everyone", "created_at": 1
        }))
        .is_err());
        assert!(serde_json::from_value::<RoomInfo>(json!({
            "session_id": "a", "host": { "name": "n" }, "audience": "friends", "created_at": 1
        }))
        .is_err());
    }

    #[test]
    fn debug_never_shows_a_secret() {
        let room: HostedRoom = serde_json::from_value(hosted()).unwrap();
        let join: JoinGrant = serde_json::from_value(json!({
            "grant": { "k": "GRANT-SECRET" }, "signature": "SIG-SECRET", "relay": "r:1"
        }))
        .unwrap();
        let shown = format!("{room:?} {join:?} {:#?} {:?}", room.provision, room.host_token);
        for secret in ["TOP-SECRET-TOKEN", "TICKET-SECRET", "SIG-SECRET", "GRANT-SECRET"] {
            assert!(!shown.contains(secret), "{secret} leaked: {shown}");
        }
        assert!(shown.contains("<redacted>"));
    }
}
