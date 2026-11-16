use std::collections::VecDeque;

use super::*;

#[derive(Clone, Default)]
struct QueueStream {
    incoming: Rc<RefCell<VecDeque<Vec<u8>>>>,
    outgoing: Rc<RefCell<Vec<Vec<u8>>>>,
}

impl Read for QueueStream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let Some(message) = self.incoming.borrow_mut().pop_front() else {
            return Err(io::ErrorKind::WouldBlock.into());
        };
        assert!(message.len() <= bytes.len());
        bytes[..message.len()].copy_from_slice(&message);
        Ok(message.len())
    }
}

impl Write for QueueStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.outgoing.borrow_mut().push(bytes.to_vec());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct Bridge {
    pair: PairedConnection<QueueStream, QueueStream>,
    guest: QueueStream,
    host: QueueStream,
}

impl Bridge {
    fn new(relay: &SocialRelay, nonce: u8, now: Instant) -> Self {
        let guest = QueueStream::default();
        let host = QueueStream::default();
        let grant = signed(nonce, 200);
        let mut pending = relay.begin_guest(&grant, guest.clone(), 100, now).unwrap();
        let pair = pending
            .pair(&grant, &"ab".repeat(32), host.clone(), 100, now)
            .unwrap();
        Self { pair, guest, host }
    }

    fn queue(&self, direction: Direction, bytes: &[u8], count: usize) {
        let stream = match direction {
            Direction::GuestToHost => &self.guest,
            Direction::HostToGuest => &self.host,
        };
        stream
            .incoming
            .borrow_mut()
            .extend(std::iter::repeat_n(bytes.to_vec(), count));
    }

    fn poll(&mut self, direction: Direction, now: Instant) -> ForwardProgress {
        self.pair.forward_once(direction, now).unwrap()
    }

    fn received(&self, direction: Direction) -> Vec<Vec<u8>> {
        let stream = match direction {
            Direction::GuestToHost => &self.host,
            Direction::HostToGuest => &self.guest,
        };
        stream.outgoing.borrow().clone()
    }
}

fn opposite(direction: Direction) -> Direction {
    match direction {
        Direction::GuestToHost => Direction::HostToGuest,
        Direction::HostToGuest => Direction::GuestToHost,
    }
}

#[test]
fn waiting_pairs_take_round_robin_turns_under_biased_polling() {
    let (relay, now) = setup(2, Duration::from_secs(30), BURST, BURST as u64);
    let mut bridges = [Bridge::new(&relay, 1, now), Bridge::new(&relay, 2, now)];
    for bridge in &bridges {
        bridge.queue(Direction::HostToGuest, &vec![6; BURST], 8);
    }
    assert_eq!(
        bridges[0].poll(Direction::HostToGuest, now),
        ForwardProgress::Transferred(BURST)
    );
    assert_eq!(
        bridges[1].poll(Direction::HostToGuest, now),
        ForwardProgress::Blocked
    );
    for second in 1..=6 {
        let chosen = second as usize % 2;
        let waiting = 1 - chosen;
        let tick = now + Duration::from_secs(second);
        for _ in 0..20 {
            assert_eq!(
                bridges[waiting].poll(Direction::HostToGuest, tick),
                ForwardProgress::Blocked
            );
        }
        assert_eq!(
            bridges[chosen].poll(Direction::HostToGuest, tick),
            ForwardProgress::Transferred(BURST)
        );
        assert_eq!(
            bridges[chosen].poll(Direction::HostToGuest, tick),
            ForwardProgress::Blocked
        );
    }
    assert_eq!(bridges[0].received(Direction::HostToGuest).len(), 4);
    assert_eq!(bridges[1].received(Direction::HostToGuest).len(), 3);
}

#[test]
fn idle_reverse_direction_lends_its_burst_and_rate() {
    for direction in [Direction::GuestToHost, Direction::HostToGuest] {
        let (relay, now) = setup(1, Duration::from_secs(30), 4 * BURST, 2 * BURST as u64);
        let mut bridge = Bridge::new(&relay, 1, now);
        bridge.queue(direction, &vec![4; BURST], 7);
        for _ in 0..4 {
            assert_eq!(
                bridge.poll(direction, now),
                ForwardProgress::Transferred(BURST)
            );
        }
        assert_eq!(bridge.poll(direction, now), ForwardProgress::Blocked);
        let refilled = now + Duration::from_secs(1);
        for _ in 0..2 {
            assert_eq!(
                bridge.poll(direction, refilled),
                ForwardProgress::Transferred(BURST)
            );
        }
        assert_eq!(bridge.poll(direction, refilled), ForwardProgress::Blocked);
    }
}

#[test]
fn session_cap_covers_every_pair_and_direction() {
    let burst = 2 * BURST + 3;
    let rate = BURST as u64 + 1;
    let (relay, now) = setup(2, Duration::from_secs(30), burst, rate);
    let mut bridges = [Bridge::new(&relay, 1, now), Bridge::new(&relay, 2, now)];
    for bridge in &bridges {
        bridge.queue(Direction::GuestToHost, &vec![5; BURST], 256);
        bridge.queue(Direction::HostToGuest, b"read while writing", 256);
    }
    let capacity = burst as u128 * 1000;
    let mut remaining = capacity;
    let mut previous_ms = 0;
    let mut transferred = 0;
    for elapsed_ms in 0..=2000 {
        remaining = (remaining + rate as u128 * (elapsed_ms - previous_ms) as u128).min(capacity);
        previous_ms = elapsed_ms;
        for bridge in &mut bridges {
            for direction in [Direction::GuestToHost, Direction::HostToGuest] {
                if let ForwardProgress::Transferred(count) =
                    bridge.poll(direction, now + Duration::from_millis(elapsed_ms))
                {
                    let charge = count as u128 * 1000;
                    assert!(
                        charge <= remaining,
                        "session cap exceeded at {elapsed_ms} ms"
                    );
                    remaining -= charge;
                    transferred += count;
                }
            }
        }
    }
    assert!(transferred > burst);
    for bridge in &bridges {
        assert!(!bridge.received(Direction::GuestToHost).is_empty());
        assert!(!bridge.received(Direction::HostToGuest).is_empty());
    }
}

#[test]
fn ready_directions_alternate_even_with_biased_polling() {
    for first in [Direction::GuestToHost, Direction::HostToGuest] {
        let (relay, now) = setup(1, Duration::from_secs(30), 8 * BURST, BURST as u64);
        let mut bridge = Bridge::new(&relay, 1, now);
        let second = opposite(first);
        bridge.queue(first, &vec![1; BURST], 3);
        bridge.queue(second, &vec![2; BURST], 3);
        assert_eq!(bridge.poll(first, now), ForwardProgress::Transferred(BURST));
        for _ in 0..20 {
            assert_eq!(bridge.poll(first, now), ForwardProgress::Blocked);
        }
        assert_eq!(bridge.poll(second, now), ForwardProgress::Transferred(BURST));
        assert_eq!(bridge.poll(second, now), ForwardProgress::Blocked);
        assert_eq!(bridge.poll(first, now), ForwardProgress::Transferred(BURST));
    }
}

#[test]
fn pending_reverse_message_prevents_borrowing_its_unused_share() {
    let (relay, now) = setup(1, Duration::from_secs(30), 4 * BURST, 2 * BURST as u64);
    let mut bridge = Bridge::new(&relay, 1, now);
    bridge.queue(Direction::GuestToHost, &vec![3; BURST], 3);
    bridge.queue(Direction::HostToGuest, b"read while writing", 4);
    for _ in 0..2 {
        assert_eq!(
            bridge.poll(Direction::GuestToHost, now),
            ForwardProgress::Transferred(BURST)
        );
        assert_eq!(
            bridge.poll(Direction::HostToGuest, now),
            ForwardProgress::Transferred(18)
        );
    }
    assert_eq!(
        bridge.poll(Direction::GuestToHost, now),
        ForwardProgress::Blocked
    );
    assert_eq!(
        bridge.poll(Direction::GuestToHost, now + Duration::from_millis(999)),
        ForwardProgress::Blocked
    );
    assert_eq!(
        bridge.poll(Direction::GuestToHost, now + Duration::from_secs(1)),
        ForwardProgress::Transferred(BURST)
    );
}

#[test]
fn saturated_direction_delivers_reverse_input_within_200_ms_of_budget() {
    for saturated in [Direction::GuestToHost, Direction::HostToGuest] {
        let rate = BURST as u64;
        let (relay, now) = setup(1, Duration::from_secs(30), BURST, rate);
        let mut bridge = Bridge::new(&relay, 1, now);
        let reverse = opposite(saturated);
        bridge.queue(saturated, &vec![57; BURST], 8);
        assert_eq!(
            bridge.poll(saturated, now),
            ForwardProgress::Transferred(BURST)
        );
        bridge.queue(reverse, b"read while writing", 1);
        let available_at = now + Duration::from_millis((18 * 1000_u64).div_ceil(rate));
        let deadline = available_at + Duration::from_millis(200);
        let mut delivered_at = None;
        for elapsed_ms in 1..=202 {
            let tick = now + Duration::from_millis(elapsed_ms);
            bridge.poll(saturated, tick);
            if bridge.poll(reverse, tick) == ForwardProgress::Transferred(18) {
                delivered_at = Some(tick);
                break;
            }
        }
        assert!(delivered_at.is_some_and(|tick| tick <= deadline));
        assert_eq!(bridge.received(reverse), [b"read while writing".to_vec()]);
    }
}
