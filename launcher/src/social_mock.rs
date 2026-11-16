//! An in-memory portal for the Friends page: [`MockSocial`] answers the social API as the
//! portal's `social-api.md` says (the same codes, caps and block rules), with people to find by
//! code. The tests use it, and `HORIZON_LAUNCHER_SOCIAL=mock` shows the page with sample data
//! while the real portal has no API. It never touches the network or the sign-in.
use crate::social::rooms::{
    Audience, Grant, HostedRoom, JoinGrant, PendingJoin, Provision, Redacted, Reprovision, RoomHost,
    RoomInfo,
};
use crate::social::*;
use std::collections::HashMap;
use std::sync::Mutex;

const ME: &str = "me";

struct MockInvite {
    invite: Invite,
    code: String,
}

struct MockGroup {
    id: String,
    name: String,
    owner: String,
    created_at: String,
    members: Vec<(String, Role, String)>,
    invites: Vec<MockInvite>,
    bans: Vec<String>,
    deleted: bool,
}

struct MockRequest {
    id: String,
    from: String,
    to: String,
    status: &'static str,
    /// A request to someone who blocked the sender: the sender sees it, the recipient does not.
    hidden: bool,
    created_at: String,
}

struct MockRoom {
    id: String,
    host: String,
    audience: Audience,
    name: Option<String>,
    created: i64,
}

#[derive(Default)]
struct State {
    rooms: Vec<MockRoom>,
    /// Guests waiting at a room of mine, not acknowledged yet: (room id, the join).
    pending: Vec<(String, PendingJoin)>,
    people: Vec<Principal>,
    codes: HashMap<String, String>,
    my_code: String,
    friends: Vec<(String, String)>,
    requests: Vec<MockRequest>,
    blocks: Vec<(String, String)>,
    /// Who blocked me.
    blocked_me: Vec<String>,
    groups: Vec<MockGroup>,
    next: u32,
    fail_next: Option<SocialError>,
}

pub struct MockSocial {
    state: Mutex<State>,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Now, plus `days`, as the portal writes timestamps.
fn stamp(days: i64) -> String {
    format!("{}T00:00:00.000Z", day_from_now(days))
}

impl State {
    fn id(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{}", self.next)
    }

    /// A code with the portal's alphabet, different each time.
    fn code(&mut self, prefix: &str) -> String {
        self.next += 1;
        let mut n = u64::from(self.next).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 8;
        let alphabet: Vec<char> = ALPHABET.chars().collect();
        let letters: String = (0..8)
            .map(|_| {
                let c = alphabet[(n % 32) as usize];
                n /= 32;
                c
            })
            .collect();
        format!("{prefix}-{}-{}", &letters[..4], &letters[4..])
    }

    fn person(&self, id: &str) -> Option<Principal> {
        self.people.iter().find(|p| p.id == id).cloned()
    }

    fn is_friend(&self, id: &str) -> bool {
        self.friends.iter().any(|(f, _)| f == id)
    }

    fn i_blocked(&self, id: &str) -> bool {
        self.blocks.iter().any(|(by, blocked)| by == ME && blocked == id)
    }

    /// Whether `id` shares a live group with me.
    fn co_member(&self, id: &str) -> bool {
        self.groups.iter().any(|g| {
            !g.deleted
                && g.members.iter().any(|m| m.0 == ME)
                && g.members.iter().any(|m| m.0 == id)
        })
    }

    fn my_groups(&self) -> usize {
        self.groups
            .iter()
            .filter(|g| !g.deleted && g.members.iter().any(|m| m.0 == ME))
            .count()
    }

    /// Whether the player may join `room`: its host is a friend (not blocked), or the player is
    /// in the group it is for.
    fn may_join(&self, room: &MockRoom) -> bool {
        if self.i_blocked(&room.host) || self.blocked_me.contains(&room.host) {
            return false;
        }
        match &room.audience {
            Audience::Friends => self.is_friend(&room.host),
            Audience::Group { id, .. } => self
                .groups
                .iter()
                .any(|g| &g.id == id && !g.deleted && g.members.iter().any(|m| m.0 == ME)),
        }
    }

    fn group_name(&self, id: &str) -> Option<String> {
        self.groups.iter().find(|g| g.id == id).map(|g| g.name.clone())
    }

    fn group_mut(&mut self, id: &str) -> Result<(&mut MockGroup, Role)> {
        let group = self
            .groups
            .iter_mut()
            .find(|g| g.id == id && !g.deleted)
            .ok_or(SocialError::Api(ApiCode::NotFound))?;
        let role = group
            .members
            .iter()
            .find(|m| m.0 == ME)
            .map(|m| m.1)
            .ok_or(SocialError::Api(ApiCode::NotFound))?;
        Ok((group, role))
    }
}

fn api(code: ApiCode) -> SocialError {
    SocialError::Api(code)
}

impl MockSocial {
    /// A portal with only the player on it.
    pub fn blank() -> Self {
        let mut state = State::default();
        state.people.push(Principal {
            id: ME.into(),
            kind: "member".into(),
            display_name: "Yuki".into(),
        });
        state.my_code = state.code(FRIEND_PREFIX);
        Self { state: Mutex::new(state) }
    }

    /// Adds someone to find by `code`; their id is `id`.
    pub fn with_person(self, id: &str, name: &str, kind: &str, code: &str) -> Self {
        {
            let mut s = self.state.lock().unwrap();
            s.people.push(Principal {
                id: id.into(),
                kind: kind.into(),
                display_name: name.into(),
            });
            s.codes.insert(code.into(), id.into());
        }
        self
    }

    /// The next call fails with `error` (once).
    #[cfg(test)]
    pub fn fail_next(&self, error: SocialError) {
        self.state.lock().unwrap().fail_next = Some(error);
    }

    /// Someone who blocked the player: a request to them is a shadow.
    #[cfg(test)]
    pub fn with_blocker(self, id: &str) -> Self {
        self.state.lock().unwrap().blocked_me.push(id.into());
        self
    }

    /// People, friends, requests, blocks and groups to look at.
    pub fn sample() -> Self {
        let mock = Self::blank()
            .with_person("sora", "Sora", "member", "YKF-SORA-2345")
            .with_person("mika", "Mika", "member", "YKF-MIKA-2345")
            .with_person("haru", "Haru", "member", "YKF-HARU-2345")
            .with_person("ren", "Ren", "member", "YKF-RENN-2345")
            .with_person("kai", "Kai (guest)", "guest", "YKF-KAIK-2345")
            .with_person("troll", "Troll", "member", "YKF-TROL-2345");
        {
            let mut s = mock.state.lock().unwrap();
            s.friends.push(("sora".into(), stamp(-30)));
            s.friends.push(("mika".into(), stamp(-3)));
            let id = s.id("r");
            s.requests.push(MockRequest {
                id,
                from: "haru".into(),
                to: ME.into(),
                status: "pending",
                hidden: false,
                created_at: stamp(-1),
            });
            let id = s.id("r");
            s.requests.push(MockRequest {
                id,
                from: ME.into(),
                to: "ren".into(),
                status: "pending",
                hidden: false,
                created_at: stamp(-2),
            });
            s.blocks.push((ME.into(), "troll".into()));
            let members = |list: &[(&str, Role)]| {
                list.iter()
                    .map(|(p, r)| (p.to_string(), *r, stamp(-10)))
                    .collect::<Vec<_>>()
            };
            let id = s.id("g");
            s.groups.push(MockGroup {
                id,
                name: "ゆにぽっぷ".into(),
                owner: ME.into(),
                created_at: stamp(-20),
                members: members(&[
                    (ME, Role::Owner),
                    ("sora", Role::Admin),
                    ("mika", Role::Member),
                    ("haru", Role::Member),
                ]),
                invites: Vec::new(),
                bans: Vec::new(),
                deleted: false,
            });
            let id = s.id("g");
            s.groups.push(MockGroup {
                id,
                name: "Build Club".into(),
                owner: "sora".into(),
                created_at: stamp(-9),
                members: members(&[("sora", Role::Owner), (ME, Role::Member), ("kai", Role::Member)]),
                invites: Vec::new(),
                bans: Vec::new(),
                deleted: false,
            });
            // Rooms to join: a friend's, one of a group of mine, and one of the other group.
            let (g1, g2) = (s.groups[0].id.clone(), s.groups[1].id.clone());
            let now = now_secs();
            for (id, host, audience, name, ago) in [
                ("room-sora", "sora", Audience::Friends, Some("夜のサバイバル"), 1500),
                (
                    "room-mika",
                    "mika",
                    Audience::Group { id: g1, name: None },
                    Some("Build night"),
                    600,
                ),
                ("room-kai", "kai", Audience::Group { id: g2, name: None }, None, 120),
            ] {
                s.rooms.push(MockRoom {
                    id: id.into(),
                    host: host.into(),
                    audience,
                    name: name.map(str::to_string),
                    created: now - ago,
                });
            }
        }
        mock
    }

    /// The player already has a room open (`HORIZON_LAUNCHER_SOCIAL_ROOM=open`), for friends.
    pub fn with_my_room_open(self) -> Self {
        {
            let mut s = self.state.lock().unwrap();
            let created = now_secs() - 420;
            s.rooms.push(MockRoom {
                id: "room-me".into(),
                host: ME.into(),
                audience: Audience::Friends,
                name: Some("Yuki room".into()),
                created,
            });
        }
        self
    }

    /// A guest asks to join my room `room`: the portal now has a grant for the host to pick up.
    /// Returns the grant's nonce. (Tests and the mock's own sample.)
    #[allow(dead_code)]
    pub fn guest_joins(&self, room: &str, principal: &str, name: &str) -> String {
        let mut s = self.state.lock().unwrap();
        s.next += 1;
        let nonce = format!("{:064x}", u128::from(s.next) * 0x1_0000_0001);
        let grant: Grant = serde_json::from_value(serde_json::json!({
            "nonce": nonce, "mock": "grant", "session": room
        }))
        .unwrap();
        s.pending.push((
            room.into(),
            PendingJoin {
                grant: Redacted::new(grant),
                signature: Redacted::new("mock-signature".into()),
                guest: RoomHost { principal: principal.into(), name: name.into() },
                expires_at: now_secs() + 240,
            },
        ));
        nonce
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, State>> {
        let mut state = self.state.lock().map_err(|_| SocialError::Invalid)?;
        match state.fail_next.take() {
            Some(error) => Err(error),
            None => Ok(state),
        }
    }
}

fn principal(s: &State, id: &str) -> Principal {
    s.person(id).unwrap_or(Principal {
        id: id.into(),
        kind: "member".into(),
        display_name: "?".into(),
    })
}

fn request_view(s: &State, r: &MockRequest) -> FriendRequest {
    FriendRequest {
        id: r.id.clone(),
        from_id: r.from.clone(),
        to_id: r.to.clone(),
        status: r.status.into(),
        created_at: r.created_at.clone(),
        from: principal(s, &r.from),
        to: principal(s, &r.to),
    }
}

impl SocialBackend for MockSocial {
    fn me(&self) -> Result<Me> {
        let s = self.lock()?;
        Ok(Me {
            id: ME.into(),
            kind: "member".into(),
            display_name: "Yuki".into(),
            friend_code: s.my_code.clone(),
            has_friend_code: true,
        })
    }

    fn rotate_friend_code(&self) -> Result<Me> {
        let mut s = self.lock()?;
        s.my_code = s.code(FRIEND_PREFIX);
        drop(s);
        self.me()
    }

    fn friends(&self) -> Result<FriendsList> {
        let s = self.lock()?;
        let pending = |f: &dyn Fn(&MockRequest) -> bool| -> Vec<FriendRequest> {
            s.requests
                .iter()
                .filter(|r| r.status == "pending" && f(r))
                .map(|r| request_view(&s, r))
                .collect()
        };
        Ok(FriendsList {
            friends: s
                .friends
                .iter()
                .map(|(id, since)| Friend {
                    principal: principal(&s, id),
                    created_at: since.clone(),
                })
                .collect(),
            incoming: pending(&|r| r.to == ME && !r.hidden),
            outgoing: pending(&|r| r.from == ME),
        })
    }

    fn send_request(&self, target: &Target) -> Result<()> {
        let mut s = self.lock()?;
        let to = match target {
            Target::Code(code) => {
                let code = normalize_code(code, FRIEND_PREFIX).ok_or(api(ApiCode::InvalidInput))?;
                if code == s.my_code {
                    return Err(api(ApiCode::InvalidInput));
                }
                s.codes.get(&code).cloned().ok_or(api(ApiCode::NotFound))?
            }
            Target::Principal(id) => {
                if id == ME {
                    return Err(api(ApiCode::InvalidInput));
                }
                if !s.co_member(id) {
                    return Err(api(ApiCode::NotFound));
                }
                id.clone()
            }
        };
        if s.i_blocked(&to) {
            return Err(api(ApiCode::NotFound));
        }
        let sent_today = s.requests.iter().filter(|r| r.from == ME).count();
        if sent_today >= 20 {
            return Err(api(ApiCode::RateLimited));
        }
        if s.is_friend(&to)
            || s.requests.iter().any(|r| {
                r.status == "pending"
                    && ((r.from == ME && r.to == to) || (r.from == to && r.to == ME && !r.hidden))
            })
        {
            return Err(api(ApiCode::Conflict));
        }
        let id = s.id("r");
        let hidden = s.blocked_me.contains(&to);
        s.requests.push(MockRequest {
            id,
            from: ME.into(),
            to,
            status: "pending",
            hidden,
            created_at: stamp(0),
        });
        Ok(())
    }

    fn answer_request(&self, id: &str, answer: Answer) -> Result<()> {
        let mut s = self.lock()?;
        let index = s
            .requests
            .iter()
            .position(|r| {
                r.id == id
                    && r.status == "pending"
                    && match answer {
                        Answer::Cancel => r.from == ME,
                        _ => r.to == ME && !r.hidden,
                    }
            })
            .ok_or(api(ApiCode::NotFound))?;
        s.requests[index].status = match answer {
            Answer::Accept => "accepted",
            Answer::Decline => "declined",
            Answer::Cancel => "cancelled",
        };
        if answer == Answer::Accept {
            let from = s.requests[index].from.clone();
            s.friends.push((from, stamp(0)));
        }
        Ok(())
    }

    fn remove_friend(&self, principal_id: &str) -> Result<()> {
        let mut s = self.lock()?;
        let before = s.friends.len();
        s.friends.retain(|(id, _)| id != principal_id);
        if s.friends.len() == before {
            return Err(api(ApiCode::NotFound));
        }
        Ok(())
    }

    fn blocks(&self) -> Result<Vec<Block>> {
        let s = self.lock()?;
        Ok(s.blocks
            .iter()
            .map(|(by, id)| Block {
                blocker_id: by.clone(),
                blocked_id: id.clone(),
                created_at: stamp(0),
                blocked: principal(&s, id),
            })
            .collect())
    }

    fn block(&self, principal_id: &str) -> Result<()> {
        let mut s = self.lock()?;
        if principal_id == ME || s.person(principal_id).is_none() {
            return Err(api(ApiCode::NotFound));
        }
        let pair = (ME.to_string(), principal_id.to_string());
        if !s.blocks.contains(&pair) {
            s.blocks.push(pair);
        }
        // Blocking ends the friendship and every request the other could see, both ways.
        s.friends.retain(|(id, _)| id != principal_id);
        for r in &mut s.requests {
            let between = (r.from == ME && r.to == principal_id) || (r.from == principal_id && r.to == ME);
            if between && r.status == "pending" && !r.hidden {
                r.status = "cancelled";
            }
        }
        Ok(())
    }

    fn unblock(&self, principal_id: &str) -> Result<()> {
        let mut s = self.lock()?;
        s.blocks.retain(|(_, id)| id != principal_id);
        Ok(())
    }

    fn groups(&self) -> Result<Vec<GroupSummary>> {
        let s = self.lock()?;
        Ok(s.groups
            .iter()
            .filter(|g| !g.deleted)
            .filter_map(|g| {
                let (_, role, joined) = g.members.iter().find(|m| m.0 == ME)?;
                Some(GroupSummary {
                    id: g.id.clone(),
                    name: g.name.clone(),
                    owner_id: g.owner.clone(),
                    created_at: g.created_at.clone(),
                    role: *role,
                    joined_at: joined.clone(),
                })
            })
            .collect())
    }

    fn create_group(&self, name: &str) -> Result<Group> {
        let mut s = self.lock()?;
        let name = clean_group_name(name).ok_or(api(ApiCode::InvalidInput))?;
        // Ten a day, and the 50-group cap.
        if s.groups.iter().filter(|g| g.created_at == stamp(0)).count() >= 10 {
            return Err(api(ApiCode::RateLimited));
        }
        if s.my_groups() >= MAX_GROUPS {
            return Err(api(ApiCode::LimitReached));
        }
        let id = s.id("g");
        let created_at = stamp(0);
        s.groups.push(MockGroup {
            id: id.clone(),
            name: name.clone(),
            owner: ME.into(),
            created_at: created_at.clone(),
            members: vec![(ME.into(), Role::Owner, created_at.clone())],
            invites: Vec::new(),
            bans: Vec::new(),
            deleted: false,
        });
        Ok(Group {
            id,
            name,
            owner_id: ME.into(),
            created_by_id: ME.into(),
            created_at,
            deleted_at: None,
        })
    }

    fn group(&self, id: &str) -> Result<GroupDetail> {
        let mut s = self.lock()?;
        let (group, role) = s.group_mut(id)?;
        let group = &*group;
        let (name, owner, created_at) = (group.name.clone(), group.owner.clone(), group.created_at.clone());
        let members: Vec<(String, Role, String)> = group.members.clone();
        let invites: Vec<Invite> = group.invites.iter().map(|i| i.invite.clone()).collect();
        let bans = group.bans.clone();
        let gid = group.id.clone();
        let mut detail = GroupDetail {
            id: gid.clone(),
            name,
            owner_id: owner,
            created_at,
            role,
            can_recover_ownership: false,
            members: members
                .iter()
                .map(|(p, r, joined)| Member {
                    group_id: gid.clone(),
                    principal_id: p.clone(),
                    role: *r,
                    joined_at: joined.clone(),
                    principal: principal(&s, p),
                })
                .collect(),
            invites: Vec::new(),
            bans: Vec::new(),
        };
        if role.manages() {
            detail.invites = invites;
            detail.bans = bans
                .iter()
                .map(|p| Ban {
                    group_id: gid.clone(),
                    principal_id: p.clone(),
                    removed_by_id: ME.into(),
                    created_at: stamp(0),
                    principal: principal(&s, p),
                })
                .collect();
        }
        Ok(detail)
    }

    fn delete_group(&self, id: &str) -> Result<()> {
        let mut s = self.lock()?;
        let (group, role) = s.group_mut(id)?;
        if role != Role::Owner {
            return Err(api(ApiCode::Forbidden));
        }
        group.deleted = true;
        Ok(())
    }

    fn create_invite(
        &self,
        group_id: &str,
        max_uses: u32,
        expires_in_days: u32,
    ) -> Result<(Invite, String)> {
        let mut s = self.lock()?;
        if !(1..=50).contains(&max_uses) || !(1..=30).contains(&expires_in_days) {
            return Err(api(ApiCode::InvalidInput));
        }
        let code = s.code(GROUP_PREFIX);
        let invite_id = s.id("i");
        let (group, role) = s.group_mut(group_id)?;
        if !role.manages() {
            return Err(api(ApiCode::Forbidden));
        }
        let now = stamp(0);
        let active = group
            .invites
            .iter()
            .filter(|i| i.invite.revoked_at.is_none() && i.invite.uses < i.invite.max_uses && i.invite.expires_at > now)
            .count();
        if active >= MAX_ACTIVE_INVITES {
            return Err(api(ApiCode::LimitReached));
        }
        let invite = Invite {
            id: invite_id,
            group_id: group.id.clone(),
            created_by_id: ME.into(),
            max_uses,
            uses: 0,
            created_at: now,
            expires_at: stamp(i64::from(expires_in_days)),
            revoked_at: None,
        };
        group.invites.push(MockInvite { invite: invite.clone(), code: code.clone() });
        Ok((invite, code))
    }

    fn revoke_invite(&self, group_id: &str, invite_id: &str) -> Result<()> {
        let mut s = self.lock()?;
        let (group, role) = s.group_mut(group_id)?;
        if !role.manages() {
            return Err(api(ApiCode::Forbidden));
        }
        let invite = group
            .invites
            .iter_mut()
            .find(|i| i.invite.id == invite_id)
            .ok_or(api(ApiCode::NotFound))?;
        invite.invite.revoked_at = Some(stamp(0));
        Ok(())
    }

    fn redeem_invite(&self, code: &str) -> Result<String> {
        let mut s = self.lock()?;
        let code = normalize_code(code, GROUP_PREFIX).ok_or(api(ApiCode::InvalidInput))?;
        let now = stamp(0);
        let mine = s.my_groups();
        let found = s.groups.iter_mut().find_map(|g| {
            let at = g.invites.iter().position(|i| i.code == code)?;
            Some((g, at))
        });
        let Some((group, at)) = found else {
            return Err(api(ApiCode::InvalidInvite));
        };
        let invite = &group.invites[at].invite;
        if group.deleted
            || invite.revoked_at.is_some()
            || invite.uses >= invite.max_uses
            || invite.expires_at <= now
            || group.bans.iter().any(|b| b == ME)
        {
            return Err(api(ApiCode::InvalidInvite));
        }
        if group.members.iter().any(|m| m.0 == ME) {
            return Err(api(ApiCode::Conflict));
        }
        if group.members.len() >= MAX_GROUP_MEMBERS {
            return Err(api(ApiCode::GroupFull));
        }
        if mine >= MAX_GROUPS {
            return Err(api(ApiCode::LimitReached));
        }
        group.invites[at].invite.uses += 1;
        group.members.push((ME.into(), Role::Member, now));
        Ok(group.id.clone())
    }

    fn set_role(&self, group_id: &str, principal_id: &str, role: Role) -> Result<()> {
        let mut s = self.lock()?;
        let (group, mine) = s.group_mut(group_id)?;
        if mine != Role::Owner {
            return Err(api(ApiCode::Forbidden));
        }
        if role == Role::Owner {
            return Err(api(ApiCode::InvalidInput));
        }
        let member = group
            .members
            .iter_mut()
            .find(|m| m.0 == principal_id)
            .ok_or(api(ApiCode::NotFound))?;
        if member.1 == Role::Owner {
            return Err(api(ApiCode::Forbidden));
        }
        member.1 = role;
        Ok(())
    }

    fn remove_member(&self, group_id: &str, principal_id: &str) -> Result<()> {
        let mut s = self.lock()?;
        let (group, mine) = s.group_mut(group_id)?;
        let theirs = group
            .members
            .iter()
            .find(|m| m.0 == principal_id)
            .map(|m| m.1)
            .ok_or(api(ApiCode::NotFound))?;
        if principal_id == ME {
            if mine == Role::Owner {
                return Err(api(ApiCode::Forbidden));
            }
        } else {
            if !mine.manages() || theirs == Role::Owner || (mine == Role::Admin && theirs != Role::Member) {
                return Err(api(ApiCode::Forbidden));
            }
            group.bans.push(principal_id.into());
        }
        group.members.retain(|m| m.0 != principal_id);
        Ok(())
    }

    fn lift_ban(&self, group_id: &str, principal_id: &str) -> Result<()> {
        let mut s = self.lock()?;
        let (group, mine) = s.group_mut(group_id)?;
        if !mine.manages() {
            return Err(api(ApiCode::Forbidden));
        }
        let before = group.bans.len();
        group.bans.retain(|b| b != principal_id);
        if group.bans.len() == before {
            return Err(api(ApiCode::NotFound));
        }
        Ok(())
    }

    fn transfer_ownership(&self, group_id: &str, principal_id: &str) -> Result<()> {
        let mut s = self.lock()?;
        let guest = s.person(principal_id).is_some_and(|p| p.is_guest());
        let (group, mine) = s.group_mut(group_id)?;
        if mine != Role::Owner {
            return Err(api(ApiCode::Forbidden));
        }
        if principal_id == ME || guest {
            return Err(api(ApiCode::Forbidden));
        }
        if !group.members.iter().any(|m| m.0 == principal_id) {
            return Err(api(ApiCode::NotFound));
        }
        for m in &mut group.members {
            if m.0 == principal_id {
                m.1 = Role::Owner;
            } else if m.0 == ME {
                m.1 = Role::Member;
            }
        }
        group.owner = principal_id.into();
        Ok(())
    }
    fn create_room(&self, audience: &Audience, name: Option<&str>) -> Result<HostedRoom> {
        let mut s = self.lock()?;
        let name = match name {
            Some(n) => Some(clean_group_name(n).ok_or(api(ApiCode::InvalidInput))?),
            None => None,
        };
        if s.rooms.iter().any(|r| r.host == ME) {
            return Err(api(ApiCode::Conflict));
        }
        let audience = match audience {
            Audience::Friends => Audience::Friends,
            Audience::Group { id, .. } => {
                let mine = s
                    .groups
                    .iter()
                    .any(|g| &g.id == id && !g.deleted && g.members.iter().any(|m| m.0 == ME));
                if !mine {
                    return Err(api(ApiCode::Forbidden));
                }
                Audience::Group { id: id.clone(), name: s.group_name(id) }
            }
        };
        let id = s.id("room-");
        let created = now_secs();
        s.rooms.push(MockRoom { id: id.clone(), host: ME.into(), audience, name, created });
        Ok(HostedRoom {
            session_id: id,
            host_principal: ME.into(),
            relay_only: true,
            host_token: Redacted::new("mock-host-token".into()),
            provision: Provision {
                ticket: Redacted::new(serde_json::json!({ "mock": true })),
                signature: Redacted::new("mock-signature".into()),
            },
            relay: "relay.mock.invalid:7000".into(),
            expires_at: created + 4 * 3600,
        })
    }

    fn reprovision_room(&self, session_id: &str) -> Result<Reprovision> {
        let s = self.lock()?;
        s.rooms
            .iter()
            .find(|r| r.id == session_id && r.host == ME)
            .ok_or(api(ApiCode::NotFound))?;
        Ok(Reprovision {
            provision: Provision {
                ticket: Redacted::new(serde_json::json!({ "mock": true })),
                signature: Redacted::new("mock-signature".into()),
            },
            relay: "relay.mock.invalid:7000".into(),
            expires_at: s.rooms.iter().find(|r| r.id == session_id).map(|r| r.created + 4 * 3600),
        })
    }

    fn rooms(&self) -> Result<Vec<RoomInfo>> {
        let s = self.lock()?;
        Ok(s.rooms
            .iter()
            .filter(|r| r.host == ME || s.may_join(r))
            .map(|r| RoomInfo {
                session_id: r.id.clone(),
                host: RoomHost {
                    principal: r.host.clone(),
                    name: principal(&s, &r.host).display_name,
                },
                audience: match &r.audience {
                    Audience::Group { id, .. } => {
                        Audience::Group { id: id.clone(), name: s.group_name(id) }
                    }
                    other => other.clone(),
                },
                name: r.name.clone(),
                created_at: r.created,
                expires_at: Some(r.created + 4 * 3600),
                mine: r.host == ME,
            })
            .collect())
    }

    fn join_room(&self, session_id: &str) -> Result<JoinGrant> {
        let s = self.lock()?;
        let room = s
            .rooms
            .iter()
            .find(|r| r.id == session_id)
            .ok_or(api(ApiCode::NotFound))?;
        if room.host == ME || !s.may_join(room) {
            return Err(api(ApiCode::Forbidden));
        }
        Ok(JoinGrant {
            grant: Redacted::new(serde_json::json!({ "mock": "grant", "session": session_id })),
            signature: Redacted::new("mock-signature".into()),
            relay: "relay.mock.invalid:7000".into(),
        })
    }

    fn close_room(&self, session_id: &str) -> Result<()> {
        let mut s = self.lock()?;
        let before = s.rooms.len();
        s.rooms.retain(|r| !(r.id == session_id && r.host == ME));
        if s.rooms.len() == before {
            return Err(api(ApiCode::NotFound));
        }
        s.pending.retain(|(room, _)| room != session_id);
        Ok(())
    }

    fn pending_joins(&self, session_id: &str) -> Result<Vec<PendingJoin>> {
        let s = self.lock()?;
        s.rooms
            .iter()
            .find(|r| r.id == session_id && r.host == ME)
            .ok_or(api(ApiCode::NotFound))?;
        Ok(s.pending
            .iter()
            .filter(|(room, _)| room == session_id)
            .map(|(_, join)| join.clone())
            .collect())
    }

    fn ack_joins(&self, session_id: &str, nonces: &[String]) -> Result<()> {
        let mut s = self.lock()?;
        s.rooms
            .iter()
            .find(|r| r.id == session_id && r.host == ME)
            .ok_or(api(ApiCode::NotFound))?;
        s.pending
            .retain(|(room, join)| !(room == session_id && nonces.iter().any(|n| n == join.grant.expose().nonce())));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> MockSocial {
        MockSocial::blank()
            .with_person("a", "Aki", "member", "YKF-AAAA-2222")
            .with_person("b", "Bea", "member", "YKF-BBBB-2222")
            .with_person("g", "Guest", "guest", "YKF-GGGG-2222")
            .with_blocker("b")
    }

    fn err<T>(r: Result<T>) -> SocialError {
        r.err().expect("an error")
    }

    #[test]
    fn requests_follow_the_portals_rules() {
        let m = world();
        let code = |c: &str| Target::Code(c.into());
        assert_eq!(err(m.send_request(&code("nonsense"))), api(ApiCode::InvalidInput));
        assert_eq!(err(m.send_request(&code("YKF-ZZZZ-2222"))), api(ApiCode::NotFound));
        let mine = m.me().unwrap().friend_code;
        assert_eq!(err(m.send_request(&code(&mine))), api(ApiCode::InvalidInput));
        m.send_request(&code("ykf-aaaa-2222")).unwrap();
        assert_eq!(err(m.send_request(&code("YKF-AAAA-2222"))), api(ApiCode::Conflict));
        let list = m.friends().unwrap();
        assert_eq!(list.outgoing.len(), 1);
        assert_eq!(list.outgoing[0].to.display_name, "Aki");
        m.answer_request(&list.outgoing[0].id, Answer::Cancel).unwrap();
        assert!(m.friends().unwrap().outgoing.is_empty());
        // Someone who blocked me: it looks sent, nobody sees it, and it stays until cancelled.
        m.send_request(&code("YKF-BBBB-2222")).unwrap();
        assert_eq!(m.friends().unwrap().outgoing.len(), 1);
        // A direct principal id works only inside a group.
        assert_eq!(err(m.send_request(&Target::Principal("a".into()))), api(ApiCode::NotFound));
    }

    #[test]
    fn accepting_makes_friends_and_blocking_ends_it() {
        let m = MockSocial::sample();
        let list = m.friends().unwrap();
        assert_eq!(list.friends.len(), 2);
        let incoming = &list.incoming[0];
        assert_eq!(incoming.from.display_name, "Haru");
        // Only the recipient accepts; only the sender cancels.
        assert_eq!(err(m.answer_request(&list.outgoing[0].id, Answer::Accept)), api(ApiCode::NotFound));
        assert_eq!(err(m.answer_request(&incoming.id, Answer::Cancel)), api(ApiCode::NotFound));
        m.answer_request(&incoming.id, Answer::Accept).unwrap();
        assert_eq!(m.friends().unwrap().friends.len(), 3);
        m.block("haru").unwrap();
        let after = m.friends().unwrap();
        assert!(after.friends.iter().all(|f| f.principal.id != "haru"));
        assert_eq!(m.blocks().unwrap().len(), 2);
        m.unblock("haru").unwrap();
        assert_eq!(m.blocks().unwrap().len(), 1);
        m.remove_friend("sora").unwrap();
        assert_eq!(err(m.remove_friend("sora")), api(ApiCode::NotFound));
    }

    #[test]
    fn groups_have_roles_invites_bans_and_caps() {
        let m = MockSocial::sample();
        let groups = m.groups().unwrap();
        let mine = groups.iter().find(|g| g.role == Role::Owner).unwrap().id.clone();
        let theirs = groups.iter().find(|g| g.role == Role::Member).unwrap().id.clone();
        // A member sees no invites or bans and cannot invite.
        assert!(m.group(&theirs).unwrap().invites.is_empty());
        assert_eq!(err(m.create_invite(&theirs, 5, 7)), api(ApiCode::Forbidden));
        assert_eq!(err(m.create_invite(&mine, 0, 7)), api(ApiCode::InvalidInput));
        assert_eq!(err(m.create_invite(&mine, 5, 31)), api(ApiCode::InvalidInput));
        let (invite, code) = m.create_invite(&mine, 1, 7).unwrap();
        assert!(code.starts_with("YKG-"));
        assert_eq!(m.group(&mine).unwrap().invites[0].id, invite.id);
        // Someone leaves and rejoins by the code; a kick bans.
        m.remove_member(&mine, "mika").unwrap();
        assert_eq!(m.group(&mine).unwrap().bans.len(), 1);
        m.lift_ban(&mine, "mika").unwrap();
        assert_eq!(err(m.lift_ban(&mine, "mika")), api(ApiCode::NotFound));
        // The owner makes an admin, then hands the group over.
        m.set_role(&mine, "haru", Role::Admin).unwrap();
        m.transfer_ownership(&mine, "haru").unwrap();
        assert_eq!(m.group(&mine).unwrap().role, Role::Member);
        assert_eq!(err(m.delete_group(&mine)), api(ApiCode::Forbidden));
        // Leaving: a member can; the owner has to hand over or delete.
        m.remove_member(&mine, "me").unwrap();
        assert!(m.groups().unwrap().iter().all(|g| g.id != mine));
        let own = m.create_group("Own").unwrap();
        assert_eq!(err(m.remove_member(&own.id, "me")), api(ApiCode::Forbidden));
        m.delete_group(&own.id).unwrap();
    }

    #[test]
    fn invite_codes_work_once_in_the_ways_the_portal_says() {
        let m = MockSocial::blank();
        let g = m.create_group("  Crew  ").unwrap();
        assert_eq!(g.name, "Crew");
        assert_eq!(err(m.create_group("   ")), api(ApiCode::InvalidInput));
        let (_, code) = m.create_invite(&g.id, 1, 1).unwrap();
        assert_eq!(err(m.redeem_invite("nope")), api(ApiCode::InvalidInput));
        assert_eq!(err(m.redeem_invite("YKG-ZZZZ-2222")), api(ApiCode::InvalidInvite));
        // Already a member: a conflict, and the use is kept.
        assert_eq!(err(m.redeem_invite(&code)), api(ApiCode::Conflict));
        let (invite, other) = {
            let (i, c) = m.create_invite(&g.id, 3, 7).unwrap();
            (i, c)
        };
        m.revoke_invite(&g.id, &invite.id).unwrap();
        assert_eq!(err(m.redeem_invite(&other)), api(ApiCode::InvalidInvite));
        assert_eq!(err(m.revoke_invite(&g.id, "nope")), api(ApiCode::NotFound));
    }

    #[test]
    fn the_caps_are_the_portals() {
        let m = MockSocial::blank();
        let mut first = None;
        for i in 0..10 {
            let g = m.create_group(&format!("G{i}")).unwrap();
            first.get_or_insert(g.id);
        }
        // Ten creations a day.
        assert_eq!(err(m.create_group("one more")), api(ApiCode::RateLimited));
        let id = first.unwrap();
        for _ in 0..MAX_ACTIVE_INVITES {
            m.create_invite(&id, 5, 7).unwrap();
        }
        assert_eq!(err(m.create_invite(&id, 5, 7)), api(ApiCode::LimitReached));
        // Twenty requests a day.
        let people = (0..21).fold(MockSocial::blank(), |m, i| {
            let n = ALPHABET.chars().nth(i % 32).unwrap();
            m.with_person(&format!("p{i}"), &format!("P{i}"), "member", &format!("YKF-{n}{n}{n}{n}-2222"))
        });
        for i in 0..20 {
            let n = ALPHABET.chars().nth(i % 32).unwrap();
            people.send_request(&Target::Code(format!("YKF-{n}{n}{n}{n}-2222"))).unwrap();
        }
        assert_eq!(
            err(people.send_request(&Target::Code("YKF-LLLL-2222".into()))),
            api(ApiCode::RateLimited)
        );
    }

    #[test]
    fn a_new_friend_code_replaces_the_old_and_failures_can_be_scripted() {
        let m = MockSocial::blank().with_person("a", "Aki", "member", "YKF-AAAA-2222");
        let first = m.me().unwrap().friend_code;
        assert_eq!(m.me().unwrap().friend_code, first);
        let second = m.rotate_friend_code().unwrap().friend_code;
        assert_ne!(first, second);
        assert!(normalize_code(&second, FRIEND_PREFIX).is_some());
        m.fail_next(SocialError::Offline);
        assert_eq!(err(m.friends()), SocialError::Offline);
        assert!(m.friends().is_ok());
    }

    #[test]
    fn rooms_open_list_join_and_close_as_the_portal_says() {
        let m = MockSocial::sample();
        // Three rooms to join (a friend's, and two groups'), none of mine.
        let list = m.rooms().unwrap();
        assert_eq!(list.len(), 3);
        assert!(list.iter().all(|r| r.host.principal != "me"));
        assert!(list.iter().any(|r| r.audience == Audience::Friends));
        let grouped: Vec<_> = list
            .iter()
            .filter_map(|r| match &r.audience {
                Audience::Group { name, .. } => name.clone(),
                Audience::Friends => None,
            })
            .collect();
        assert_eq!(grouped.len(), 2);

        // Joining gives a grant and the relay; an unknown room is not found.
        let join = m.join_room("room-sora").unwrap();
        assert_eq!(join.relay, "relay.mock.invalid:7000");
        assert_eq!(err(m.join_room("nope")), api(ApiCode::NotFound));
        // Blocking the host takes their room away from the list, and the join is refused.
        m.block("sora").unwrap();
        assert_eq!(m.rooms().unwrap().len(), 2);
        assert_eq!(err(m.join_room("room-sora")), api(ApiCode::Forbidden));
        // Leaving a group closes its rooms to me.
        let kai_group = list
            .iter()
            .find(|r| r.session_id == "room-kai")
            .map(|r| match &r.audience {
                Audience::Group { id, .. } => id.clone(),
                Audience::Friends => unreachable!(),
            })
            .unwrap();
        m.remove_member(&kai_group, "me").unwrap();
        assert_eq!(err(m.join_room("room-kai")), api(ApiCode::Forbidden));

        // My own room: one at a time, only for friends or a group I am in.
        let groups = m.groups().unwrap();
        let mine = groups.iter().find(|g| g.role == Role::Owner).unwrap().id.clone();
        let room = m
            .create_room(&Audience::Group { id: mine.clone(), name: None }, Some("Night run"))
            .unwrap();
        assert_eq!(room.host_principal, "me");
        assert_eq!(err(m.create_room(&Audience::Friends, None)), api(ApiCode::Conflict));
        let own = m.rooms().unwrap().into_iter().find(|r| r.host.principal == "me").unwrap();
        assert_eq!(own.name.as_deref(), Some("Night run"));
        assert_eq!(err(m.join_room(&room.session_id)), api(ApiCode::Forbidden));
        assert_eq!(m.reprovision_room(&room.session_id).unwrap().relay, room.relay);
        // Only the host closes it, and once.
        assert_eq!(err(m.close_room("room-mika")), api(ApiCode::NotFound));
        m.close_room(&room.session_id).unwrap();
        assert_eq!(err(m.close_room(&room.session_id)), api(ApiCode::NotFound));
        assert_eq!(err(m.create_room(&Audience::Group { id: "nope".into(), name: None }, None)), api(ApiCode::Forbidden));
        assert_eq!(err(m.create_room(&Audience::Friends, Some("a\u{7}b"))), api(ApiCode::InvalidInput));
        m.create_room(&Audience::Friends, None).unwrap();
    }

    #[test]
    fn the_player_can_start_with_a_room_already_open() {
        let m = MockSocial::sample().with_my_room_open();
        let own: Vec<_> = m.rooms().unwrap().into_iter().filter(|r| r.host.principal == "me").collect();
        assert_eq!(own.len(), 1);
        assert_eq!(err(m.create_room(&Audience::Friends, None)), api(ApiCode::Conflict));
    }
}
