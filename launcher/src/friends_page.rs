//! The Friends page's state and what it does (the view is `friends_view.rs`): friend code,
//! friends, requests, blocks and groups from the portal's social API (`social.rs`).
//!
//! Every call runs on its own thread against a [`SocialBackend`] and reports over a channel that
//! `drain_friends` empties every 80 ms with the rest of the launcher's background work, so the
//! window never waits for the network. After a write the page reads everything again: the portal
//! is the authority, and what it says is what the page shows.
use super::*;
use crate::line_edit::LineEdit;
use crate::social::{
    self, Answer, Block, Friend, FriendRequest, FriendsList, GroupDetail, GroupSummary,
    Me, Role, SocialBackend, SocialError, Target,
};
use gpui::{Pixels, Size};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::Arc;

/// The page is read again this often while it is open.
const REFRESH: Duration = Duration::from_secs(30);
/// How long "Copied" stays on a button.
const COPIED: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum FriendsTab {
    Friends,
    Requests,
    Groups,
    Blocked,
}

/// What the page can show.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum FriendsStatus {
    /// Not asked yet.
    Idle,
    Loading,
    Ready,
    /// The portal has no social API (yet), or this build has no portal.
    NotAvailable,
    /// The portal cannot be reached, or answered something odd, and there is nothing to show.
    Offline,
    /// No sign-in saved, or not accepted.
    SignedOut,
}

/// A button that asks twice (what ends something, or cannot be taken back).
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum Confirm {
    NewCode,
    Remove(String),
    Block(String),
    Kick(String),
    Leave,
    DeleteGroup,
}

/// The line the Groups tab has open above its list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum GroupPanel {
    None,
    Create,
    Join,
}

/// What was copied, for "Copied" on its button.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Copied {
    FriendCode,
    InviteCode,
}

/// A text field of the page.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum InputKind {
    AddFriend,
    GroupName,
    JoinCode,
}

pub(super) struct Input {
    pub(super) edit: LineEdit,
    /// Made by the view the first time it draws the field.
    focus: Option<FocusHandle>,
    pub(super) focused: bool,
}

impl Input {
    fn new(max: usize) -> Self {
        Self {
            edit: LineEdit::new(String::new(), max, 32),
            focus: None,
            focused: false,
        }
    }

    /// Whether the field has the keyboard, as of this frame.
    fn note_focus(&mut self, window: &Window) {
        self.focused = self.focus.as_ref().is_some_and(|h| h.is_focused(window));
    }

    /// The field's focus handle, made on first use.
    pub(super) fn handle(&mut self, cx: &mut App) -> FocusHandle {
        self.focus
            .get_or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone()
    }
}

/// What a write was, for its message and for what to do after it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum FriendAction {
    /// Reading the page (the errors of the reads).
    Load,
    SendRequest,
    Accept,
    Decline,
    Cancel,
    RemoveFriend,
    Block,
    Unblock,
    NewCode,
    CreateGroup,
    Join,
    DeleteGroup,
    Leave,
    CreateInvite,
    RevokeInvite,
    SetRole,
    Kick,
    LiftBan,
    Transfer,
}

pub(super) enum Outcome {
    Done,
    Me(Me),
    /// A group to open: the one just made or joined.
    Group(String),
    /// A new invite and its code, which the portal shows only once.
    Invite(String),
}

enum Event {
    Me(social::Result<Me>),
    Friends(social::Result<FriendsList>),
    Blocks(social::Result<Vec<Block>>),
    Groups(social::Result<Vec<GroupSummary>>),
    Detail(String, social::Result<GroupDetail>),
    /// One full read is over.
    Refreshed,
    Done(FriendAction, social::Result<Outcome>),
}

/// What the page does next, after an event changed its state.
#[derive(Debug, PartialEq, Eq)]
enum Follow {
    /// Read everything again.
    Refresh,
    OpenGroup(String),
}

/// What the page keeps between frames.
pub(super) struct Friends {
    backend: Option<Arc<dyn SocialBackend>>,
    tx: mpsc::Sender<Event>,
    rx: mpsc::Receiver<Event>,
    pub(super) status: FriendsStatus,
    pub(super) me: Option<Me>,
    pub(super) list: FriendsList,
    pub(super) blocks: Vec<Block>,
    pub(super) groups: Vec<GroupSummary>,
    /// The group whose page is open, and what the portal said of it.
    pub(super) open_group: Option<String>,
    pub(super) detail: Option<GroupDetail>,
    pub(super) tab: FriendsTab,
    /// The last result in words: good or not.
    pub(super) note: Option<(bool, String)>,
    pub(super) confirm: Option<Confirm>,
    pub(super) panel: GroupPanel,
    /// A new invite's code and its group, shown until the group page closes.
    pub(super) new_invite: Option<(String, String)>,
    /// Which of [`social::INVITE_USES`] and [`social::INVITE_DAYS`] the next invite uses.
    pub(super) invite_uses: usize,
    pub(super) invite_days: usize,
    pub(super) copied: Option<(Copied, Instant)>,
    pub(super) add: Input,
    pub(super) name: Input,
    pub(super) join: Input,
    /// The tab strip's size, for the panel's plate.
    pub(super) strip: Rc<Cell<Option<Size<Pixels>>>>,
    /// A field to put the keyboard in on the next frame.
    pub(super) focus_soon: Option<InputKind>,
    pub(super) busy: usize,
    refreshing: bool,
    refresh_again: bool,
    loaded_at: Option<Instant>,
    /// `HORIZON_LAUNCHER_FRIENDS_TAB=group:<id, role or name>`: a group to open once the list is in.
    preview_group: Option<String>,
    /// `HORIZON_LAUNCHER_FRIENDS_NOTE`, said once the language is set.
    preview_note: Option<bool>,
    /// `HORIZON_LAUNCHER_FRIENDS_MENU=1`: the first member's menu open on the group page.
    preview_menu: bool,
}

impl Friends {
    pub(super) fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            backend: None,
            tx,
            rx,
            status: FriendsStatus::Idle,
            me: None,
            list: FriendsList::default(),
            blocks: Vec::new(),
            groups: Vec::new(),
            open_group: None,
            detail: None,
            tab: FriendsTab::Friends,
            note: None,
            confirm: None,
            panel: GroupPanel::None,
            new_invite: None,
            invite_uses: 1,
            invite_days: 1,
            copied: None,
            add: Input::new(32),
            name: Input::new(social::MAX_GROUP_NAME),
            join: Input::new(32),
            strip: Rc::new(Cell::new(None)),
            focus_soon: None,
            busy: 0,
            refreshing: false,
            refresh_again: false,
            loaded_at: None,
            preview_group: None,
            preview_note: None,
            preview_menu: false,
        }
    }

    /// `HORIZON_LAUNCHER_FRIENDS_TAB=friends|requests|groups|blocked|group:<id, role or name>` and
    /// `HORIZON_LAUNCHER_FRIENDS_NOTE=error|ok` start the page on a tab, a group or a message, for
    /// screenshots. Nothing happens without them.
    pub(super) fn from_env() -> Self {
        let mut page = Self::new();
        match std::env::var("HORIZON_LAUNCHER_FRIENDS_TAB").as_deref() {
            Ok("friends") => page.tab = FriendsTab::Friends,
            Ok("requests") => page.tab = FriendsTab::Requests,
            Ok("groups") => page.tab = FriendsTab::Groups,
            Ok("blocked") => page.tab = FriendsTab::Blocked,
            Ok(other) => {
                if let Some(group) = other.strip_prefix("group:") {
                    page.tab = FriendsTab::Groups;
                    page.preview_group = Some(group.to_string());
                }
            }
            Err(_) => {}
        }
        page.preview_menu = std::env::var_os("HORIZON_LAUNCHER_FRIENDS_MENU").is_some();
        page.preview_note = match std::env::var("HORIZON_LAUNCHER_FRIENDS_NOTE").as_deref() {
            Ok("error") => Some(false),
            Ok("ok") => Some(true),
            _ => None,
        };
        page
    }

    /// A page over a backend of the caller's, for tests.
    #[cfg(test)]
    fn with_backend(backend: Arc<dyn SocialBackend>) -> Self {
        let mut friends = Self::new();
        friends.backend = Some(backend);
        friends
    }

    /// Reads which field has the keyboard, and gives it to the one asked for.
    pub(super) fn note_focus(&mut self, window: &mut Window, cx: &mut App) {
        if let Some(kind) = self.focus_soon.take() {
            let handle = self.input(kind).handle(cx);
            window.focus(&handle, cx);
        }
        for kind in [InputKind::AddFriend, InputKind::GroupName, InputKind::JoinCode] {
            self.input(kind).note_focus(window);
        }
    }

    pub(super) fn input(&mut self, kind: InputKind) -> &mut Input {
        match kind {
            InputKind::AddFriend => &mut self.add,
            InputKind::GroupName => &mut self.name,
            InputKind::JoinCode => &mut self.join,
        }
    }

    fn backend(&mut self) -> Arc<dyn SocialBackend> {
        self.backend
            .get_or_insert_with(|| social::configured(&backend::data_dir()))
            .clone()
    }

    /// The player's own principal id, for "you" and for leaving.
    pub(super) fn my_id(&self) -> &str {
        self.me.as_ref().map_or("", |me| me.id.as_str())
    }

    fn note_error(&mut self, action: FriendAction, error: SocialError) {
        self.note = Some((false, error_text(action, error)));
    }

    /// A failure that says where the page stands (no API, no sign-in) changes its state.
    fn note_state(&mut self, error: SocialError) {
        match error {
            SocialError::NotAvailable => self.status = FriendsStatus::NotAvailable,
            SocialError::SignedOut => self.status = FriendsStatus::SignedOut,
            SocialError::Offline | SocialError::Invalid | SocialError::Api(_)
                if self.me.is_none() =>
            {
                self.status = FriendsStatus::Offline
            }
            _ => {}
        }
    }

    /// Takes in what a background call said. What to do next is the caller's ([`Follow`]).
    fn apply(&mut self, event: Event) -> Vec<Follow> {
        let mut follow = Vec::new();
        match event {
            Event::Me(Ok(me)) => {
                self.me = Some(me);
                self.status = FriendsStatus::Ready;
            }
            Event::Me(Err(error)) => {
                self.note_state(error);
                if self.status == FriendsStatus::Ready {
                    self.note_error(FriendAction::Load, error);
                }
            }
            Event::Friends(Ok(list)) => self.list = list,
            Event::Blocks(Ok(blocks)) => self.blocks = blocks,
            Event::Groups(Ok(groups)) => self.groups = groups,
            Event::Friends(Err(error)) | Event::Blocks(Err(error)) | Event::Groups(Err(error)) => {
                self.note_state(error);
                if self.note.is_none() {
                    self.note_error(FriendAction::Load, error);
                }
            }
            Event::Detail(id, result) => {
                if self.open_group.as_deref() != Some(id.as_str()) {
                    return follow;
                }
                match result {
                    Ok(detail) => self.detail = Some(detail),
                    Err(error) => {
                        // A group that is gone (deleted, or one is no longer in it) closes.
                        if error == SocialError::Api(social::ApiCode::NotFound) {
                            self.open_group = None;
                            self.detail = None;
                            self.new_invite = None;
                        }
                        self.note_state(error);
                        self.note_error(FriendAction::Load, error);
                    }
                }
            }
            Event::Refreshed => {
                self.refreshing = false;
                self.loaded_at = Some(Instant::now());
                // Asked, and not even `me` came back: say so rather than load for ever.
                if self.status == FriendsStatus::Loading {
                    self.status = FriendsStatus::Offline;
                }
                if std::mem::take(&mut self.refresh_again) {
                    follow.push(Follow::Refresh);
                }
            }
            Event::Done(action, result) => {
                self.busy = self.busy.saturating_sub(1);
                match result {
                    Ok(outcome) => follow.extend(self.done(action, outcome)),
                    Err(error) => {
                        self.note_state(error);
                        self.note_error(action, error);
                        // What the page showed may be stale (a request answered elsewhere, a
                        // member already gone): read it again.
                        if matches!(
                            error,
                            SocialError::Api(
                                social::ApiCode::NotFound
                                    | social::ApiCode::Conflict
                                    | social::ApiCode::Forbidden
                            )
                        ) {
                            follow.push(Follow::Refresh);
                        }
                    }
                }
            }
        }
        follow
    }

    fn done(&mut self, action: FriendAction, outcome: Outcome) -> Vec<Follow> {
        use FriendAction as A;
        let mut follow = Vec::new();
        self.note = Some((true, tr(done_en(action)).to_string()));
        match (action, outcome) {
            (A::SendRequest, _) => self.add.edit.set(String::new()),
            (A::CreateGroup, Outcome::Group(id)) => {
                self.name.edit.set(String::new());
                self.panel = GroupPanel::None;
                self.tab = FriendsTab::Groups;
                follow.push(Follow::OpenGroup(id));
            }
            (A::Join, Outcome::Group(id)) => {
                self.join.edit.set(String::new());
                self.panel = GroupPanel::None;
                self.tab = FriendsTab::Groups;
                follow.push(Follow::OpenGroup(id));
            }
            (A::NewCode, Outcome::Me(me)) => self.me = Some(me),
            (A::CreateInvite, Outcome::Invite(code)) => {
                if let Some(id) = self.open_group.clone() {
                    self.new_invite = Some((id, code));
                }
            }
            (A::DeleteGroup | A::Leave, _) => {
                self.open_group = None;
                self.detail = None;
                self.new_invite = None;
            }
            _ => {}
        }
        follow.push(Follow::Refresh);
        follow
    }
}

// ----- Words ------------------------------------------------------------------------------

type Words = (&'static str, &'static [(&'static str, &'static str)]);

/// What went wrong, in English with its `{holes}` and their values: the page says it in the
/// player's language (`error_text`), the tests check each has a Japanese one.
pub(super) fn error_en(action: FriendAction, error: SocialError) -> Words {
    use social::ApiCode as C;
    use FriendAction as A;
    match error {
        SocialError::NotAvailable => (
            "Friends are not available yet: the portal has not turned them on.",
            &[],
        ),
        SocialError::Offline => ("Cannot reach the portal. Check the connection.", &[]),
        SocialError::SignedOut => ("Sign in on the Yoake page first.", &[]),
        SocialError::Invalid => ("The portal answered something unexpected.", &[]),
        SocialError::Api(code) => match (code, action) {
            (C::InvalidInput, A::SendRequest) => (
                "That friend code does not look right. A friend code looks like YKF-ABCD-2345.",
                &[],
            ),
            (C::InvalidInput, A::CreateGroup) => (
                "A group name is 1 to {max} characters, with no control characters.",
                &[("max", "40")],
            ),
            (C::InvalidInput, A::Join) => (
                "That is not a group invite code. It looks like YKG-ABCD-2345.",
                &[],
            ),
            (C::InvalidInput, _) => ("The portal did not accept that.", &[]),
            (C::InvalidInvite, _) => (
                "That invite does not work: it may be wrong, expired, used up or revoked.",
                &[],
            ),
            (C::Forbidden, A::Leave) => (
                "The owner cannot leave. Hand the group over or delete it.",
                &[],
            ),
            (C::Forbidden, A::Transfer) => (
                "Ownership can only go to a member of the group you have not blocked.",
                &[],
            ),
            (
                C::Forbidden,
                A::DeleteGroup
                | A::CreateInvite
                | A::RevokeInvite
                | A::SetRole
                | A::Kick
                | A::LiftBan,
            ) => ("You are not allowed to do that in this group.", &[]),
            (C::Forbidden, _) => ("You are not allowed to do that.", &[]),
            (C::NotFound, A::SendRequest) => ("No one has that friend code.", &[]),
            (C::NotFound, A::Accept | A::Decline | A::Cancel) => {
                ("That request is not there any more.", &[])
            }
            (C::NotFound, _) => ("That is not there any more.", &[]),
            (C::Conflict, A::SendRequest) => (
                "You are already friends, or a request is already waiting.",
                &[],
            ),
            (C::Conflict, A::Join) => ("You are already in that group.", &[]),
            (C::Conflict, _) => ("That already exists.", &[]),
            (C::GroupFull, _) => ("That group already has {max} members.", &[("max", "100")]),
            (C::LimitReached, A::CreateInvite) => (
                "This group already has {max} active invites. Revoke one first.",
                &[("max", "20")],
            ),
            (C::LimitReached, A::CreateGroup | A::Join) => (
                "You can be in at most {max} groups. Leave one first.",
                &[("max", "50")],
            ),
            (C::LimitReached, _) => ("A limit of the portal is reached.", &[]),
            (C::RateLimited, A::SendRequest) => (
                "You have sent as many friend requests as the portal allows for now ({member} a day, {guest} for guests). Try again later.",
                &[("member", "20"), ("guest", "5")],
            ),
            (C::RateLimited, A::CreateGroup) => (
                "You can create {max} groups a day. Try again later.",
                &[("max", "10")],
            ),
            (C::RateLimited, A::Join) => (
                "Too many invite tries for now. Try again in an hour.",
                &[],
            ),
            (C::RateLimited, _) => (
                "Too many requests for now. Try again in a little while.",
                &[],
            ),
            (C::Internal, _) => ("The portal had a problem. Try again later.", &[]),
        },
    }
}

pub(super) fn error_text(action: FriendAction, error: SocialError) -> String {
    let (en, args) = error_en(action, error);
    trf(en, args)
}

/// What a write that worked says.
pub(super) fn done_en(action: FriendAction) -> &'static str {
    use FriendAction as A;
    match action {
        A::Load => "",
        A::SendRequest => "Friend request sent.",
        A::Accept => "You are friends now.",
        A::Decline => "Request declined.",
        A::Cancel => "Request cancelled.",
        A::RemoveFriend => "Friend removed.",
        A::Block => "Blocked.",
        A::Unblock => "Unblocked.",
        A::NewCode => "Your friend code was replaced. The old one no longer works.",
        A::CreateGroup => "Group created.",
        A::Join => "You joined the group.",
        A::DeleteGroup => "Group deleted.",
        A::Leave => "You left the group.",
        A::CreateInvite => "Invite created. Copy its code now: the portal shows it only once.",
        A::RevokeInvite => "Invite revoked.",
        A::SetRole => "Role changed.",
        A::Kick => "Member kicked and banned. They cannot rejoin until the ban is lifted.",
        A::LiftBan => "Ban lifted.",
        A::Transfer => "You handed the group over.",
    }
}

/// How a date from the portal reads: its day.
pub(super) fn day(stamp: &str) -> &str {
    stamp.get(..10).unwrap_or(stamp)
}

// ----- What the page does -----------------------------------------------------------------

impl Launcher {
    /// The page was opened: read it again.
    pub(super) fn friends_opened(&mut self) {
        self.friends.note = None;
        self.friends.confirm = None;
        self.friends.loaded_at = None;
        if self.friends.status != FriendsStatus::Ready {
            self.friends.status = FriendsStatus::Idle;
        }
    }

    /// Reads everything in the background (the page's first draw, Refresh, after a write).
    pub(super) fn friends_refresh(&mut self) {
        if self.friends.refreshing {
            self.friends.refresh_again = true;
            return;
        }
        self.friends.refreshing = true;
        if self.friends.status == FriendsStatus::Idle {
            self.friends.status = FriendsStatus::Loading;
        }
        let backend = self.friends.backend();
        let tx = self.friends.tx.clone();
        let open = self.friends.open_group.clone();
        std::thread::spawn(move || {
            let me = backend.me();
            let stop = me.is_err();
            let _ = tx.send(Event::Me(me));
            if !stop {
                let _ = tx.send(Event::Friends(backend.friends()));
                let _ = tx.send(Event::Blocks(backend.blocks()));
                let _ = tx.send(Event::Groups(backend.groups()));
                if let Some(id) = open {
                    let detail = backend.group(&id);
                    let _ = tx.send(Event::Detail(id, detail));
                }
            }
            let _ = tx.send(Event::Refreshed);
        });
    }

    /// Runs a write in the background; its outcome comes back as [`Event::Done`].
    pub(super) fn friends_act(
        &mut self,
        action: FriendAction,
        run: impl FnOnce(&dyn SocialBackend) -> social::Result<Outcome> + Send + 'static,
    ) {
        let backend = self.friends.backend();
        let tx = self.friends.tx.clone();
        self.friends.busy += 1;
        self.friends.note = None;
        self.friends.confirm = None;
        std::thread::spawn(move || {
            let _ = tx.send(Event::Done(action, run(backend.as_ref())));
        });
    }

    /// Opens a group's page, reading it.
    pub(super) fn friends_open_group(&mut self, id: String) {
        self.friends.open_group = Some(id.clone());
        self.friends.detail = None;
        self.friends.new_invite = None;
        self.friends.confirm = None;
        let backend = self.friends.backend();
        let tx = self.friends.tx.clone();
        std::thread::spawn(move || {
            let detail = backend.group(&id);
            let _ = tx.send(Event::Detail(id, detail));
        });
    }

    pub(super) fn friends_close_group(&mut self) {
        self.friends.open_group = None;
        self.friends.detail = None;
        self.friends.new_invite = None;
        self.friends.confirm = None;
        self.friends.note = None;
    }

    pub(super) fn friends_set_tab(&mut self, tab: FriendsTab) {
        self.friends.tab = tab;
        self.friends.note = None;
        self.friends.confirm = None;
    }

    /// Enter in one of the page's fields.
    pub(super) fn friends_enter(&mut self, kind: InputKind) {
        match kind {
            InputKind::AddFriend => self.friends_send_request(),
            InputKind::GroupName => self.friends_create_group(),
            InputKind::JoinCode => self.friends_join(),
        }
    }

    /// Whether a field holds something its button could send.
    pub(super) fn friends_input_ok(&self, kind: InputKind) -> bool {
        match kind {
            InputKind::AddFriend => {
                social::normalize_code(self.friends.add.edit.text(), social::FRIEND_PREFIX)
                    .is_some()
            }
            InputKind::GroupName => {
                social::clean_group_name(self.friends.name.edit.text()).is_some()
            }
            InputKind::JoinCode => {
                social::normalize_code(self.friends.join.edit.text(), social::GROUP_PREFIX)
                    .is_some()
            }
        }
    }

    fn friends_send_request(&mut self) {
        let Some(code) =
            social::normalize_code(self.friends.add.edit.text(), social::FRIEND_PREFIX)
        else {
            self.friends.note = Some((
                false,
                error_text(
                    FriendAction::SendRequest,
                    SocialError::Api(social::ApiCode::InvalidInput),
                ),
            ));
            return;
        };
        self.friends_act(FriendAction::SendRequest, move |api| {
            api.send_request(&Target::Code(code))
                .map(|()| Outcome::Done)
        });
    }

    /// A friend request to someone of one of the player's groups, by id.
    pub(super) fn friends_send_to_member(&mut self, principal: String) {
        self.friends_act(FriendAction::SendRequest, move |api| {
            api.send_request(&Target::Principal(principal))
                .map(|()| Outcome::Done)
        });
    }

    fn friends_create_group(&mut self) {
        let Some(name) = social::clean_group_name(self.friends.name.edit.text()) else {
            self.friends.note = Some((
                false,
                error_text(
                    FriendAction::CreateGroup,
                    SocialError::Api(social::ApiCode::InvalidInput),
                ),
            ));
            return;
        };
        self.friends_act(FriendAction::CreateGroup, move |api| {
            api.create_group(&name)
                .map(|group| Outcome::Group(group.id))
        });
    }

    fn friends_join(&mut self) {
        let Some(code) =
            social::normalize_code(self.friends.join.edit.text(), social::GROUP_PREFIX)
        else {
            self.friends.note = Some((
                false,
                error_text(
                    FriendAction::Join,
                    SocialError::Api(social::ApiCode::InvalidInput),
                ),
            ));
            return;
        };
        self.friends_act(FriendAction::Join, move |api| {
            api.redeem_invite(&code).map(Outcome::Group)
        });
    }

    pub(super) fn friends_answer(&mut self, id: String, answer: Answer) {
        let action = match answer {
            Answer::Accept => FriendAction::Accept,
            Answer::Decline => FriendAction::Decline,
            Answer::Cancel => FriendAction::Cancel,
        };
        self.friends_act(action, move |api| {
            api.answer_request(&id, answer).map(|()| Outcome::Done)
        });
    }

    pub(super) fn friends_remove(&mut self, principal: String) {
        self.friends_act(FriendAction::RemoveFriend, move |api| {
            api.remove_friend(&principal).map(|()| Outcome::Done)
        });
    }

    pub(super) fn friends_block(&mut self, principal: String) {
        self.friends_act(FriendAction::Block, move |api| {
            api.block(&principal).map(|()| Outcome::Done)
        });
    }

    pub(super) fn friends_unblock(&mut self, principal: String) {
        self.friends_act(FriendAction::Unblock, move |api| {
            api.unblock(&principal).map(|()| Outcome::Done)
        });
    }

    pub(super) fn friends_new_code(&mut self) {
        self.friends_act(FriendAction::NewCode, |api| {
            api.rotate_friend_code().map(Outcome::Me)
        });
    }

    /// An action on the open group (`run` gets its id).
    pub(super) fn friends_group_act(
        &mut self,
        action: FriendAction,
        run: impl FnOnce(&dyn SocialBackend, &str) -> social::Result<Outcome> + Send + 'static,
    ) {
        let Some(id) = self.friends.open_group.clone() else {
            return;
        };
        self.friends_act(action, move |api| run(api, &id));
    }

    pub(super) fn friends_create_invite(&mut self) {
        let uses = social::INVITE_USES[self.friends.invite_uses % social::INVITE_USES.len()];
        let days = social::INVITE_DAYS[self.friends.invite_days % social::INVITE_DAYS.len()];
        self.friends_group_act(FriendAction::CreateInvite, move |api, group| {
            api.create_invite(group, uses, days)
                .map(|(_, code)| Outcome::Invite(code))
        });
    }

    pub(super) fn friends_leave(&mut self) {
        let me = self.friends.my_id().to_string();
        self.friends_group_act(FriendAction::Leave, move |api, group| {
            api.remove_member(group, &me).map(|()| Outcome::Done)
        });
    }

    /// Copies `text` and says so on the button.
    pub(super) fn friends_copy(&mut self, what: Copied, text: String, cx: &mut App) {
        copy_to_clipboard(cx, gpui::ClipboardItem::new_string(text));
        self.friends.copied = Some((what, Instant::now()));
    }

    /// The page's background work: what the portal answered, the 30 s refresh and "Copied".
    /// True when something changed.
    pub(super) fn drain_friends(&mut self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.friends.rx.try_recv() {
            changed = true;
            for follow in self.friends.apply(event) {
                match follow {
                    Follow::Refresh => self.friends_refresh(),
                    Follow::OpenGroup(id) => self.friends_open_group(id),
                }
            }
        }
        if self.friends.preview_menu {
            if let Some(detail) = &self.friends.detail {
                let me = self.friends.my_id().to_string();
                let first = detail
                    .members
                    .iter()
                    .find(|m| m.principal_id != me && m.role != social::Role::Owner);
                if let Some(member) = first {
                    self.list_menu = Some((
                        SharedString::from(format!("fr-member-{}-menu", member.principal_id)),
                        false,
                    ));
                    self.friends.preview_menu = false;
                    changed = true;
                }
            }
        }
        if let Some(good) = self.friends.preview_note.take() {
            self.friends.note = Some(if good {
                (true, tr(done_en(FriendAction::SendRequest)).to_string())
            } else {
                (
                    false,
                    error_text(
                        FriendAction::SendRequest,
                        SocialError::Api(social::ApiCode::NotFound),
                    ),
                )
            });
            changed = true;
        }
        if self.friends.open_group.is_none() && !self.friends.groups.is_empty() {
            if let Some(want) = self.friends.preview_group.take() {
                let found = self.friends.groups.iter().find(|g| {
                    g.id == want || g.name == want || role_word(g.role).eq_ignore_ascii_case(&want)
                });
                if let Some(id) = found.map(|g| g.id.clone()) {
                    self.friends_open_group(id);
                    changed = true;
                }
            }
        }
        if let Some((_, at)) = self.friends.copied {
            if at.elapsed() >= COPIED {
                self.friends.copied = None;
                changed = true;
            }
        }
        if self.page == Page::Friends
            && self.friends.status == FriendsStatus::Ready
            && self.friends.busy == 0
            && !self.friends.refreshing
            && self
                .friends
                .loaded_at
                .is_some_and(|at| at.elapsed() >= REFRESH)
        {
            self.friends_refresh();
        }
        changed
    }
}

// ----- Small views of the lists -----------------------------------------------------------

/// A person's line under their name in a list.
fn kind_note(is_guest: bool) -> &'static str {
    if is_guest {
        "Guest"
    } else {
        "Member"
    }
}

pub(super) fn friend_note(friend: &Friend) -> String {
    trf(
        "{kind} · friends since {date}",
        &[
            ("kind", tr(kind_note(friend.principal.is_guest()))),
            ("date", day(&friend.created_at)),
        ],
    )
}

pub(super) fn request_note(request: &FriendRequest, incoming: bool) -> String {
    let who = if incoming { &request.from } else { &request.to };
    trf(
        "{kind} · sent {date}",
        &[
            ("kind", tr(kind_note(who.is_guest()))),
            ("date", day(&request.created_at)),
        ],
    )
}

pub(super) fn role_word(role: Role) -> &'static str {
    match role {
        Role::Owner => "Owner",
        Role::Admin => "Admin",
        Role::Member => "Member",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::social_mock::MockSocial;
    use horizon_launcher::i18n::{tr_in, Language};
    use social::ApiCode as C;

    const ACTIONS: [FriendAction; 19] = [
        FriendAction::Load,
        FriendAction::SendRequest,
        FriendAction::Accept,
        FriendAction::Decline,
        FriendAction::Cancel,
        FriendAction::RemoveFriend,
        FriendAction::Block,
        FriendAction::Unblock,
        FriendAction::NewCode,
        FriendAction::CreateGroup,
        FriendAction::Join,
        FriendAction::DeleteGroup,
        FriendAction::Leave,
        FriendAction::CreateInvite,
        FriendAction::RevokeInvite,
        FriendAction::SetRole,
        FriendAction::Kick,
        FriendAction::LiftBan,
        FriendAction::Transfer,
    ];

    fn errors() -> Vec<SocialError> {
        let mut all = vec![
            SocialError::NotAvailable,
            SocialError::Offline,
            SocialError::SignedOut,
            SocialError::Invalid,
        ];
        for code in [
            C::InvalidInput,
            C::InvalidInvite,
            C::Forbidden,
            C::NotFound,
            C::Conflict,
            C::GroupFull,
            C::LimitReached,
            C::RateLimited,
            C::Internal,
        ] {
            all.push(SocialError::Api(code));
        }
        all
    }

    fn holes(s: &str) -> Vec<&str> {
        let mut found: Vec<_> = s
            .split('{')
            .skip(1)
            .filter_map(|p| p.split_once('}').map(|x| x.0))
            .collect();
        found.sort();
        found
    }

    #[test]
    fn every_message_has_a_japanese_one_and_fills_its_holes() {
        for action in ACTIONS {
            for error in errors() {
                let (en, args) = error_en(action, error);
                assert!(!en.is_empty());
                let ja = tr_in(Language::Japanese, en);
                assert_ne!(ja, en, "no Japanese for {en:?}");
                let mut given: Vec<_> = args.iter().map(|a| a.0).collect();
                given.sort();
                assert_eq!(holes(en), given, "{en:?}");
                assert_eq!(holes(ja), given, "{ja:?}");
            }
            let done = done_en(action);
            if action != FriendAction::Load {
                assert_ne!(tr_in(Language::Japanese, done), done, "no Japanese for {done:?}");
            }
        }
        for en in [
            "Guest",
            "Member",
            "Owner",
            "Admin",
            "{kind} · friends since {date}",
            "{kind} · sent {date}",
            "(You)",
            "Kick",
            "Click again to kick and ban",
        ] {
            assert_ne!(tr_in(Language::Japanese, en), en, "{en:?}");
        }
    }

    #[test]
    fn the_caps_are_in_the_messages() {
        let say = error_text;
        assert!(say(FriendAction::Join, SocialError::Api(C::GroupFull)).contains("100"));
        assert!(say(FriendAction::CreateInvite, SocialError::Api(C::LimitReached)).contains("20"));
        assert!(say(FriendAction::CreateGroup, SocialError::Api(C::LimitReached)).contains("50"));
        assert!(say(FriendAction::CreateGroup, SocialError::Api(C::RateLimited)).contains("10"));
        assert!(say(FriendAction::SendRequest, SocialError::Api(C::RateLimited)).contains("20"));
    }

    /// Feeds what the backend says into the page's state, as the page's threads do.
    fn read_all(page: &mut Friends, api: &dyn SocialBackend) {
        page.refreshing = true;
        page.apply(Event::Me(api.me()));
        page.apply(Event::Friends(api.friends()));
        page.apply(Event::Blocks(api.blocks()));
        page.apply(Event::Groups(api.groups()));
        page.apply(Event::Refreshed);
    }

    fn done(
        page: &mut Friends,
        action: FriendAction,
        result: social::Result<Outcome>,
    ) -> Vec<Follow> {
        page.busy += 1;
        page.apply(Event::Done(action, result))
    }

    #[test]
    fn the_page_takes_in_what_the_portal_says() {
        let api = MockSocial::sample();
        let mut page = Friends::with_backend(Arc::new(MockSocial::blank()));
        assert_eq!(page.status, FriendsStatus::Idle);
        read_all(&mut page, &api);
        assert_eq!(page.status, FriendsStatus::Ready);
        assert_eq!(page.me.as_ref().unwrap().display_name, "Yuki");
        assert_eq!(
            (
                page.list.friends.len(),
                page.list.incoming.len(),
                page.list.outgoing.len()
            ),
            (2, 1, 1)
        );
        assert_eq!((page.blocks.len(), page.groups.len()), (1, 2));
        assert_eq!(page.my_id(), "me");

        // A request goes out: the field empties, the page reads again.
        page.add.edit.set("YKF-RENN-2345".into());
        let follow = done(&mut page, FriendAction::SendRequest, Ok(Outcome::Done));
        assert_eq!(follow, [Follow::Refresh]);
        assert_eq!(page.add.edit.text(), "");
        assert_eq!(page.note.as_ref().map(|n| n.0), Some(true));

        // A group made: its page opens on the Groups tab, the name field empties.
        page.name.edit.set("Crew".into());
        page.panel = GroupPanel::Create;
        let follow = done(
            &mut page,
            FriendAction::CreateGroup,
            Ok(Outcome::Group("g9".into())),
        );
        assert_eq!(follow, [Follow::OpenGroup("g9".into()), Follow::Refresh]);
        assert_eq!(
            (page.tab, page.panel, page.name.edit.text()),
            (FriendsTab::Groups, GroupPanel::None, "")
        );
    }

    #[test]
    fn a_new_invite_code_is_kept_for_its_group_until_the_page_closes() {
        let api = MockSocial::sample();
        let mut page = Friends::with_backend(Arc::new(MockSocial::blank()));
        read_all(&mut page, &api);
        let mine = page
            .groups
            .iter()
            .find(|g| g.role == Role::Owner)
            .unwrap()
            .id
            .clone();
        page.open_group = Some(mine.clone());
        let (_, code) = api.create_invite(&mine, 5, 7).unwrap();
        done(
            &mut page,
            FriendAction::CreateInvite,
            Ok(Outcome::Invite(code.clone())),
        );
        assert_eq!(page.new_invite, Some((mine.clone(), code)));
        // The detail arrives for the open group only.
        page.apply(Event::Detail("other".into(), api.group(&mine)));
        assert!(page.detail.is_none());
        page.apply(Event::Detail(mine.clone(), api.group(&mine)));
        assert_eq!(page.detail.as_ref().unwrap().invites.len(), 1);
        // Deleting or leaving closes it and drops the code.
        done(&mut page, FriendAction::DeleteGroup, Ok(Outcome::Done));
        assert!(page.open_group.is_none() && page.new_invite.is_none() && page.detail.is_none());
    }

    #[test]
    fn errors_set_the_state_or_a_note() {
        let mut page = Friends::with_backend(Arc::new(MockSocial::blank()));
        page.status = FriendsStatus::Loading;
        page.refreshing = true;
        page.apply(Event::Me(Err(SocialError::NotAvailable)));
        page.apply(Event::Refreshed);
        assert_eq!(page.status, FriendsStatus::NotAvailable);

        let mut page = Friends::with_backend(Arc::new(MockSocial::blank()));
        page.status = FriendsStatus::Loading;
        page.apply(Event::Me(Err(SocialError::Offline)));
        assert_eq!(page.status, FriendsStatus::Offline);
        page.apply(Event::Me(Err(SocialError::SignedOut)));
        assert_eq!(page.status, FriendsStatus::SignedOut);

        // Once the page has data, a failed write is a note and the data stays.
        let mut page = Friends::with_backend(Arc::new(MockSocial::blank()));
        read_all(&mut page, &MockSocial::sample());
        let follow = done(
            &mut page,
            FriendAction::SendRequest,
            Err(SocialError::Api(C::NotFound)),
        );
        assert_eq!(follow, [Follow::Refresh]);
        assert_eq!(page.status, FriendsStatus::Ready);
        assert_eq!(
            page.note,
            Some((false, tr("No one has that friend code.").to_string()))
        );
        assert_eq!(page.list.friends.len(), 2);
        // A gone group closes its page.
        page.open_group = Some("g1".into());
        page.apply(Event::Detail("g1".into(), Err(SocialError::Api(C::NotFound))));
        assert!(page.open_group.is_none());
        // A rate limit is a note and no re-read.
        let follow = done(
            &mut page,
            FriendAction::Join,
            Err(SocialError::Api(C::RateLimited)),
        );
        assert!(follow.is_empty());
        // A second read asked for while one runs runs after it.
        page.refreshing = true;
        page.refresh_again = true;
        assert_eq!(page.apply(Event::Refreshed), [Follow::Refresh]);
    }
}
