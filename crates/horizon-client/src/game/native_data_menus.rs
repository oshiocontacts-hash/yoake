use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use horizon_client::{
    containers::{self, BlockEntity, ModBlockEntity, ModBlockEntityFields, ModMenuType},
    lan::data::component_nbt,
    menu::{Kind, Menu},
    mod_api::{Host, ItemHandler, ItemStack, Text},
    native_inventory::{self, MAX_MENU_EXTRA},
};
use horizon_protocol::packets::play;

use super::{Game, lan::LanHost, native_block_behaviors::valid_id};

pub trait MenuBehavior: Send + Sync {
    fn create(
        &self,
        host: &mut dyn Host,
        player: u128,
        extra: &[u8],
        client_side: bool,
    ) -> Option<Arc<dyn ItemHandler>>;

    fn still_valid(&self, _host: &mut dyn Host, _player: u128, _inventory: &dyn ItemHandler) -> bool {
        true
    }

    fn closed(&self, _host: &mut dyn Host, _player: u128, _inventory: &dyn ItemHandler) {}
}

pub struct NativeMenuBehavior {
    pub id: String,
    pub behavior: Arc<dyn MenuBehavior>,
}

struct Behaviors(HashMap<String, Arc<dyn MenuBehavior>>);

impl Behaviors {
    fn new(entries: Vec<NativeMenuBehavior>) -> Result<Self, String> {
        let mut behaviors = HashMap::new();
        for entry in entries {
            if !valid_id(&entry.id) || behaviors.insert(entry.id.clone(), entry.behavior).is_some() {
                return Err(format!("invalid or duplicate menu behavior: {}", entry.id));
            }
        }
        Ok(Self(behaviors))
    }
}

static BEHAVIORS: OnceLock<Behaviors> = OnceLock::new();

pub(super) fn register() -> Result<(), String> {
    let mut entries = Vec::new();
    Game::native_menu_behaviors(&mut entries);
    let behaviors = Behaviors::new(entries)?;
    if behaviors.0.is_empty() {
        return Ok(());
    }
    BEHAVIORS
        .set(behaviors)
        .map_err(|_| "menu behaviors already registered".into())
}

pub(super) fn enabled() -> bool {
    BEHAVIORS.get().is_some()
}

pub(super) fn lookup(id: &str) -> Option<&'static Arc<dyn MenuBehavior>> {
    BEHAVIORS.get()?.0.get(id)
}

fn create_inventory(
    behavior: &dyn MenuBehavior,
    host: &mut dyn Host,
    player: u128,
    extra: &[u8],
    client_side: bool,
    slots: usize,
) -> Option<Arc<dyn ItemHandler>> {
    if slots > 256 || extra.len() > MAX_MENU_EXTRA {
        return None;
    }
    let inventory = behavior.create(host, player, extra, client_side)?;
    if inventory.slots() != slots {
        behavior.closed(host, player, inventory.as_ref());
        return None;
    }
    Some(inventory)
}

pub(super) struct OpenRequest {
    player: u128,
    kind: &'static ModMenuType,
    title: Text,
    extra: Vec<u8>,
}

pub(super) struct Session {
    pub(super) window: i32,
    player: u128,
    behavior: Arc<dyn MenuBehavior>,
    inventory: Arc<dyn ItemHandler>,
    engine: Box<Menu>,
}

pub(super) const LOCAL_MENU_KEY: (i32, i32, i32) = (i32::MIN, 0, 0);

pub(super) struct LocalSession {
    pub(super) kind: &'static ModMenuType,
    pub(super) title: Text,
    pub(super) entity: BlockEntity,
    extra: Vec<u8>,
    player: u128,
    behavior: Arc<dyn MenuBehavior>,
    inventory: Arc<dyn ItemHandler>,
}

impl Session {
    pub(super) fn inventory_for_update(&self) -> &dyn ItemHandler {
        self.inventory.as_ref()
    }
}

impl Game<'_> {
    pub(super) fn local_data_menu(&self) -> Option<&LocalSession> {
        if !enabled()
            || self.remote.is_some()
            || self.lan_acting.is_some()
            || self.container != Some(LOCAL_MENU_KEY)
        {
            return None;
        }
        self.native_local_data_menu.as_ref()
    }

    pub(super) fn data_menu_handler(&self) -> Option<Arc<dyn ItemHandler>> {
        if !enabled() {
            return None;
        }
        self.remote
            .as_ref()
            .and_then(|remote| remote.menu_handler.clone())
            .or_else(|| self.local_data_menu().map(|menu| menu.inventory.clone()))
    }

    pub(super) fn sync_local_data_menu(&mut self) {
        if self.local_data_menu().is_some() {
            let session = self.native_local_data_menu.as_mut().unwrap();
            if let BlockEntity::Mod(entity) = &mut session.entity {
                entity.items = native_inventory::snapshot(session.inventory.as_ref());
            }
        }
    }

    pub(super) fn close_local_data_menu(&mut self) {
        if self.local_data_menu().is_none() {
            return;
        }
        let session = self.native_local_data_menu.take().unwrap();
        session
            .behavior
            .closed(self, session.player, session.inventory.as_ref());
        self.container = None;
        self.mod_screen = None;
    }

    pub(super) fn tick_local_data_menu(&mut self) {
        if self.local_data_menu().is_none() {
            return;
        }
        let session = self.native_local_data_menu.take().unwrap();
        let valid = self
            .player
            .as_ref()
            .is_some_and(|player| player.health.health() > 0.0)
            && session
                .behavior
                .still_valid(self, session.player, session.inventory.as_ref());
        self.native_local_data_menu = Some(session);
        if valid {
            self.sync_local_data_menu();
        } else {
            self.close_container();
            self.screen = super::Screen::Playing;
            self.capture(true);
        }
    }

    fn open_local_data_menu(&mut self, request: OpenRequest) {
        if !self
            .player
            .as_ref()
            .is_some_and(|player| player.health.health() > 0.0)
        {
            return;
        }
        let Some(behavior) = lookup(request.kind.id).cloned() else {
            return;
        };
        self.close_container();
        let Some(inventory) = create_inventory(
            behavior.as_ref(),
            self,
            request.player,
            &request.extra,
            false,
            request.kind.slots,
        ) else {
            self.screen = super::Screen::Playing;
            self.capture(true);
            return;
        };
        let entity = BlockEntity::Mod(Box::new(ModBlockEntity::from(ModBlockEntityFields {
            id: request.kind.id.into(),
            items: native_inventory::snapshot(inventory.as_ref()),
            ..Default::default()
        })));
        self.native_local_data_menu = Some(LocalSession {
            kind: request.kind,
            title: request.title,
            entity,
            extra: request.extra,
            player: request.player,
            behavior,
            inventory,
        });
        self.container = Some(LOCAL_MENU_KEY);
        self.open_native_screen(LOCAL_MENU_KEY, None, 1, request.kind.id);
        if self.mod_screen.is_none() {
            self.close_local_data_menu();
            self.screen = super::Screen::Playing;
            self.capture(true);
            return;
        }
        self.screen = super::Screen::Inventory;
        self.capture(false);
    }

    pub(super) fn prepare_remote_data_menu(&mut self, window: i32, menu_type: &str) -> bool {
        if !enabled() {
            return true;
        }
        self.close_remote_data_menu();
        let opening = self
            .remote
            .as_mut()
            .and_then(|remote| remote.pending_menu.take());
        let Some(opening) = opening
            .filter(|opening| opening.window == window && opening.menu_type == menu_type)
        else {
            return lookup(menu_type).is_none();
        };
        let Some(behavior) = lookup(menu_type) else {
            return false;
        };
        let Some(kind) = containers::mod_menu_type(menu_type) else {
            return false;
        };
        let player = self.native_acting_id();
        let Some(inventory) = create_inventory(
            behavior.as_ref(),
            self,
            player,
            &opening.extra,
            true,
            kind.slots,
        ) else {
            return false;
        };
        if let Some(remote) = self.remote.as_mut() {
            remote.menu_extra = opening.extra;
            remote.menu_handler = Some(inventory);
            remote.menu_handler_type = Some(menu_type.into());
        }
        true
    }

    pub(super) fn close_remote_data_menu(&mut self) {
        if !enabled() {
            return;
        }
        let closing = self.remote.as_mut().and_then(|remote| {
            remote.menu_extra.clear();
            let inventory = remote.menu_handler.take()?;
            Some((remote.menu_handler_type.take()?, inventory))
        });
        if let Some((menu_type, inventory)) = closing {
            if let Some(behavior) = lookup(&menu_type) {
                let player = self.native_acting_id();
                behavior.closed(self, player, inventory.as_ref());
            }
        }
    }

    pub(super) fn remote_menu_extra(&self) -> &[u8] {
        if !enabled() {
            return &[];
        }
        self.remote
            .as_ref()
            .map(|remote| remote.menu_extra.as_slice())
            .or_else(|| self.local_data_menu().map(|menu| menu.extra.as_slice()))
            .unwrap_or(&[])
    }

    pub(super) fn request_data_menu(
        &mut self,
        player: u128,
        menu_type: &str,
        title: Text,
        extra: &[u8],
    ) -> bool {
        if !enabled() || !self.native_authoritative() || extra.len() > MAX_MENU_EXTRA {
            return false;
        }
        let Some(kind) = containers::mod_menu_type(menu_type).filter(|kind| kind.slots <= 256) else {
            return false;
        };
        if lookup(menu_type).is_none() || self.native_data_menu_requests.len() >= 64 {
            return false;
        }
        let local = self.lan_acting.is_none()
            && self.options.dedicated.is_none()
            && self.native_acting_id() == player
            && self
                .player
                .as_ref()
                .is_some_and(|player| player.health.health() > 0.0);
        if !local {
            if let Some(host) = self.lan.as_ref() {
                if !host.hub.guests.iter().any(|guest| {
                    guest.placed
                        && !guest.dead
                        && guest.uuid == player
                        && containers::menu_reaches(&guest.brand, menu_type)
                }) {
                    return false;
                }
            } else if self.lan_acting.is_none() || self.native_acting_id() != player {
                return false;
            }
        }
        self.native_data_menu_requests.push(OpenRequest {
            player,
            kind,
            title,
            extra: extra.to_vec(),
        });
        true
    }

    pub(super) fn flush_data_menus(&mut self) {
        if !enabled() {
            return;
        }
        let requests = std::mem::take(&mut self.native_data_menu_requests);
        let local_player = self.native_acting_id();
        let mut remote_requests = Vec::new();
        for request in requests {
            if self.lan_acting.is_none()
                && self.options.dedicated.is_none()
                && request.player == local_player
            {
                self.open_local_data_menu(request);
            } else {
                remote_requests.push(request);
            }
        }
        if remote_requests.is_empty() {
            return;
        }
        let Some(mut host) = self.lan.take() else {
            self.native_data_menu_requests = remote_requests;
            return;
        };
        for request in remote_requests {
            let Some(conn) = host
                .hub
                .guests
                .iter()
                .find(|guest| guest.placed && !guest.dead && guest.uuid == request.player)
                .map(|guest| guest.conn)
            else {
                continue;
            };
            let Some(behavior) = lookup(request.kind.id).cloned() else {
                continue;
            };
            self.guest_close_all(&mut host, conn);
            let inventory = self
                .as_guest(&mut host, conn, |game| {
                    create_inventory(
                        behavior.as_ref(),
                        game,
                        request.player,
                        &request.extra,
                        false,
                        request.kind.slots,
                    )
                })
                .flatten();
            let Some(inventory) = inventory else {
                continue;
            };
            let items = native_inventory::snapshot(inventory.as_ref());
            let window = host.hub.open_data_menu(
                conn,
                request.kind,
                component_nbt(&request.title.component()),
                items,
                &request.extra,
            );
            if let Some(window) = window {
                let mut engine = Box::new(Menu::new(Kind::Mod(request.kind)));
                engine.item_handler = Some(inventory.clone());
                host.play.entry(conn).or_default().data_menu = Some(Session {
                    window,
                    player: request.player,
                    behavior,
                    inventory,
                    engine,
                });
            } else {
                self.player_hook(&mut host, conn, |game| {
                    behavior.closed(game, request.player, inventory.as_ref());
                });
            }
        }
        self.lan = Some(host);
    }

    pub(super) fn close_data_menu(&mut self, host: &mut LanHost, conn: u32) {
        if !enabled() {
            return;
        }
        if let Some(session) = host
            .play
            .get_mut(&conn)
            .and_then(|play| play.data_menu.take())
        {
            self.player_hook(host, conn, |game| {
                session
                    .behavior
                    .closed(game, session.player, session.inventory.as_ref());
            });
        }
    }

    pub(super) fn tick_data_menus(&mut self, host: &mut LanHost) {
        if !enabled() {
            return;
        }
        let connections: Vec<u32> = host
            .play
            .iter()
            .filter_map(|(conn, play)| play.data_menu.as_ref().map(|_| *conn))
            .collect();
        for conn in connections {
            let Some(session) = host
                .play
                .get_mut(&conn)
                .and_then(|play| play.data_menu.take())
            else {
                continue;
            };
            let open = host.hub.guest(conn).is_some_and(|guest| {
                guest.menu.as_ref().is_some_and(|menu| menu.window == session.window)
            });
            let valid = open
                && host.hub.guest(conn).is_some_and(|guest| !guest.dead)
                && self
                    .as_guest(host, conn, |game| {
                        session.behavior.still_valid(
                            game,
                            session.player,
                            session.inventory.as_ref(),
                        )
                    })
                    .unwrap_or(false);
            if let Some(play) = host.play.get_mut(&conn) {
                play.data_menu = Some(session);
            }
            if valid {
                let session = host.play[&conn].data_menu.as_ref().unwrap();
                host.hub.set_native_menu(
                    conn,
                    native_inventory::snapshot(session.inventory.as_ref()),
                );
            } else {
                if open {
                    host.hub.close_container(conn);
                    self.guest_close_all(host, conn);
                } else {
                    self.close_data_menu(host, conn);
                }
            }
        }
    }

    pub(super) fn lan_data_menu_click(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        packet: &play::ContainerClick,
    ) -> bool {
        if !enabled() {
            return false;
        }
        let Some(index) = host.hub.guests.iter().position(|guest| guest.conn == conn) else {
            return false;
        };
        let Some(mut session) = host
            .play
            .get_mut(&conn)
            .and_then(|play| play.data_menu.take())
        else {
            return false;
        };
        if session.window != packet.container {
            host.play.get_mut(&conn).unwrap().data_menu = Some(session);
            return false;
        }
        let valid = !host.hub.guests[index].dead
            && self
                .as_guest(host, conn, |game| {
                    session.behavior.still_valid(
                        game,
                        session.player,
                        session.inventory.as_ref(),
                    )
                })
                .unwrap_or(false);
        if !valid {
            host.play.get_mut(&conn).unwrap().data_menu = Some(session);
            host.hub.close_container(conn);
            self.guest_close_all(host, conn);
            return true;
        }
        if host.hub.guests[index].spectator() {
            host.play.get_mut(&conn).unwrap().data_menu = Some(session);
            host.hub.resend_menu(conn, packet.container, &self.world);
            return true;
        }
        let inventory = session.inventory.clone();
        let before = native_inventory::snapshot(inventory.as_ref());
        let quick_move = !session.engine.quick_crafting()
            && self.data_menu_quick_move(
                host,
                conn,
                packet,
                session.engine.kind(),
                &before,
                inventory.as_ref(),
            );
        if !quick_move {
            self.guest_menu_op(host, index, &mut session.engine, |menu, env| {
                native_inventory::load(menu, env, inventory.as_ref());
                let _ = menu.click(
                    env,
                    i32::from(packet.slot),
                    i32::from(packet.button),
                    super::lan_crafting::input(packet.input),
                );
            });
        }
        host.hub
            .set_native_menu(conn, native_inventory::snapshot(inventory.as_ref()));
        host.play.get_mut(&conn).unwrap().data_menu = Some(session);
        host.hub.resend_menu(conn, packet.container, &self.world);
        true
    }

    fn data_menu_quick_move(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        packet: &play::ContainerClick,
        kind: Kind,
        before: &[Option<ItemStack>],
        inventory: &dyn ItemHandler,
    ) -> bool {
        use super::native_quick_move::quick_move_slot;
        let Some(at) = quick_move_slot(
            packet.input,
            i32::from(packet.button),
            i32::from(packet.slot),
            before.len(),
            &[],
        ) else {
            return false;
        };
        self.as_guest(host, conn, |game| {
            game.apply_data_quick_move(at, packet.slot as usize, kind, before, inventory)
        })
        .unwrap_or(false)
    }

    pub(super) fn local_data_quick_move(&mut self, internal: usize, button: i32) -> bool {
        let Some(session) = self.local_data_menu() else {
            return false;
        };
        let kind = session.kind;
        let inventory = session.inventory.clone();
        let Some(slot) = Self::menu_slot_index(kind.id, internal) else {
            return false;
        };
        let before = native_inventory::snapshot(inventory.as_ref());
        let Some(at) = super::native_quick_move::quick_move_slot(
            1,
            button,
            slot as i32,
            before.len(),
            &[],
        ) else {
            return false;
        };
        let handled = self.apply_data_quick_move(
            at,
            slot,
            Kind::Mod(kind),
            &before,
            inventory.as_ref(),
        );
        self.sync_local_data_menu();
        handled
    }

    pub(super) fn apply_data_quick_move(
        &mut self,
        at: super::native_quick_move::MenuSlot,
        slot: usize,
        kind: Kind,
        before: &[Option<ItemStack>],
        inventory: &dyn ItemHandler,
    ) -> bool {
        use super::native_quick_move::{MenuSlot, settle};
        let stack = match at {
            MenuSlot::Container(slot) => before.get(slot).cloned().flatten(),
            MenuSlot::Inventory(slot) => self.inventory.get(slot).cloned().flatten(),
        };
        let Some(mut stack) = stack else {
            return false;
        };
        let Kind::Mod(menu_type) = kind else {
            return false;
        };
        let mut entity = ModBlockEntity::from(ModBlockEntityFields {
            id: menu_type.id.into(),
            items: before.to_vec(),
            ..Default::default()
        });
        let mut handled = false;
        let player_before = self.inventory.clone();
        self.native_menu_quick_move(
            [i32::MIN, 0, 0],
            &mut entity,
            before.len(),
            slot,
            &mut stack,
            &mut handled,
        );
        settle(
            at,
            handled,
            stack,
            before,
            &mut entity.items,
            &mut self.inventory,
        );
        if !handled || !native_inventory::replace(inventory, &entity.items) {
            self.inventory = player_before;
        }
        handled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    struct Closed;

    impl MenuBehavior for Closed {
        fn create(
            &self,
            _host: &mut dyn Host,
            _player: u128,
            _extra: &[u8],
            _client_side: bool,
        ) -> Option<Arc<dyn ItemHandler>> {
            None
        }
    }

    struct TestHost;

    impl Host for TestHost {}

    struct Storage(Mutex<Vec<Option<ItemStack>>>);

    impl ItemHandler for Storage {
        fn slots(&self) -> usize {
            self.0.lock().unwrap().len()
        }

        fn get_stack(&self, slot: usize) -> Option<ItemStack> {
            self.0.lock().unwrap().get(slot).cloned().flatten()
        }

        fn set_stack(&self, slot: usize, stack: Option<ItemStack>) {
            self.0.lock().unwrap()[slot] = stack;
        }

        fn stack_limit(&self, _stack: &ItemStack) -> u32 {
            64
        }
    }

    struct Factory {
        calls: Mutex<Vec<(u128, Vec<u8>, bool)>>,
        closed: AtomicUsize,
        valid: AtomicBool,
        inventory: Arc<Storage>,
    }

    impl Factory {
        fn new(slots: usize) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                closed: AtomicUsize::new(0),
                valid: AtomicBool::new(true),
                inventory: Arc::new(Storage(Mutex::new(vec![None; slots]))),
            }
        }
    }

    impl MenuBehavior for Factory {
        fn create(
            &self,
            _host: &mut dyn Host,
            player: u128,
            extra: &[u8],
            client_side: bool,
        ) -> Option<Arc<dyn ItemHandler>> {
            self.calls
                .lock()
                .unwrap()
                .push((player, extra.to_vec(), client_side));
            if client_side {
                Some(Arc::new(Storage(Mutex::new(native_inventory::snapshot(
                    self.inventory.as_ref(),
                )))))
            } else {
                Some(self.inventory.clone())
            }
        }

        fn still_valid(&self, _host: &mut dyn Host, _player: u128, _inventory: &dyn ItemHandler) -> bool {
            self.valid.load(Ordering::Relaxed)
        }

        fn closed(&self, _host: &mut dyn Host, _player: u128, _inventory: &dyn ItemHandler) {
            self.closed.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn empty_registry_does_not_install_dispatch_and_claims_are_unique() {
        assert!(Behaviors::new(Vec::new()).unwrap().0.is_empty());
        assert!(!enabled());
        assert!(lookup("test:backpack").is_none());
        let behavior: Arc<dyn MenuBehavior> = Arc::new(Closed);
        let entries = || {
            vec![NativeMenuBehavior {
                id: "test:backpack".into(),
                behavior: behavior.clone(),
            }]
        };
        let registry = Behaviors::new(entries()).unwrap();
        assert!(Arc::ptr_eq(&registry.0["test:backpack"], &behavior));
        let mut duplicate = entries();
        duplicate.extend(entries());
        assert!(Behaviors::new(duplicate).is_err());
        assert!(Behaviors::new(vec![NativeMenuBehavior {
            id: "minecraft:chest".into(),
            behavior,
        }])
        .is_err());
    }

    #[test]
    fn opening_buffer_reaches_both_factories_and_closing_preserves_inventory() {
        let factory = Factory::new(9);
        let mut host = TestHost;
        let opening = native_inventory::MenuOpening {
            window: 3,
            menu_type: "test:backpack".into(),
            extra: vec![0, 255, 3, 9],
        };
        let received = native_inventory::MenuOpening::decode(&opening.encode().unwrap()).unwrap();
        let server = create_inventory(&factory, &mut host, 42, &opening.extra, false, 9).unwrap();
        let client = create_inventory(&factory, &mut host, 42, &received.extra, true, 9).unwrap();
        assert_eq!(
            *factory.calls.lock().unwrap(),
            vec![(42, opening.extra.clone(), false), (42, opening.extra, true)],
        );
        let stack = ItemStack::new("minecraft:stone", 12);
        assert!(server.insert(0, stack.clone(), false).is_none());
        assert!(client.insert(0, stack, false).is_none());
        assert_eq!(
            native_inventory::snapshot(server.as_ref()),
            native_inventory::snapshot(client.as_ref()),
        );
        assert!(factory.still_valid(&mut host, 42, server.as_ref()));
        factory.valid.store(false, Ordering::Relaxed);
        assert!(!factory.still_valid(&mut host, 42, server.as_ref()));
        factory.closed(&mut host, 42, server.as_ref());
        factory.closed(&mut host, 42, client.as_ref());
        let reopened = create_inventory(&factory, &mut host, 42, &received.extra, false, 9).unwrap();
        assert_eq!(reopened.get_stack(0).unwrap().count, 12);
        assert_eq!(factory.closed.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn malformed_opening_is_rejected_before_factory_and_wrong_slots_close_once() {
        let factory = Factory::new(8);
        let mut host = TestHost;
        assert!(create_inventory(&factory, &mut host, 42, &[], false, 257).is_none());
        assert!(
            create_inventory(
                &factory,
                &mut host,
                42,
                &vec![0; MAX_MENU_EXTRA + 1],
                true,
                9,
            )
            .is_none()
        );
        assert!(factory.calls.lock().unwrap().is_empty());
        assert!(create_inventory(&factory, &mut host, 42, &[], false, 9).is_none());
        assert_eq!(factory.calls.lock().unwrap().len(), 1);
        assert_eq!(factory.closed.load(Ordering::Relaxed), 1);
        assert!(create_inventory(&Closed, &mut host, 42, &[], true, 9).is_none());
    }
}
