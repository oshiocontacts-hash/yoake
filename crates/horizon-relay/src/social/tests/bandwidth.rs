use std::cell::Cell;
use std::collections::VecDeque;

use super::*;

#[derive(Clone, Default)]
struct QueueStream {
    incoming: Rc<RefCell<VecDeque<Vec<u8>>>>,
    outgoing: Rc<RefCell<Vec<Vec<u8>>>>,
    write_limit: Rc<Cell<Option<usize>>>,
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
        let count = self.write_limit.get().unwrap_or(bytes.len()).min(bytes.len());
        if count == 0 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        self.outgoing.borrow_mut().push(bytes[..count].to_vec());
        Ok(count)
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
fn stopped_or_write_stalled_pair_never_delays_another_pair() {
    for write_limit in [None, Some(0), Some(1)] {
        let (relay, now) = setup(2, Duration::from_secs(30), 4 * BURST, 2 * BURST as u64);
        let mut stopped = Bridge::new(&relay, 1, now);
        let mut running = Bridge::new(&relay, 2, now);
        stopped.guest.write_limit.set(write_limit);
        stopped.queue(Direction::HostToGuest, &vec![6; BURST], 2);
        stopped.poll(Direction::HostToGuest, now);
        running.queue(Direction::HostToGuest, &vec![7; BURST], 10);
        for _ in 0..3 {
            assert_eq!(
                running.poll(Direction::HostToGuest, now),
                ForwardProgress::Transferred(BURST)
            );
        }
        for second in 1..=3 {
            let tick = now + Duration::from_secs(second);
            for _ in 0..2 {
                assert_eq!(
                    running.poll(Direction::HostToGuest, tick),
                    ForwardProgress::Transferred(BURST)
                );
            }
        }
        assert_eq!(running.received(Direction::HostToGuest).len(), 9);
    }
}

#[test]
fn full_session_bucket_reaches_portal_rate_without_inter_pair_waits() {
    let rate = 256 * 1024;
    for pair_count in [1, 2, 8] {
        let (relay, now) = setup(pair_count, Duration::from_secs(30), rate, rate as u64);
        let mut bridges: Vec<_> = (1..=pair_count)
            .map(|nonce| Bridge::new(&relay, nonce as u8, now))
            .collect();
        bridges[0].queue(Direction::HostToGuest, &vec![6; BURST], rate / BURST);
        for _ in 0..rate / BURST {
            assert_eq!(
                bridges[0].poll(Direction::HostToGuest, now),
                ForwardProgress::Transferred(BURST)
            );
        }
        let messages_per_pair = rate / BURST / pair_count;
        for bridge in &mut bridges {
            bridge.queue(Direction::HostToGuest, &vec![7; BURST], 3 * messages_per_pair);
            assert_eq!(
                bridge.poll(Direction::HostToGuest, now),
                ForwardProgress::Blocked
            );
        }
        for second in 1..=3 {
            let tick = now + Duration::from_secs(second);
            let mut transferred = 0;
            for bridge in &mut bridges {
                for _ in 0..messages_per_pair {
                    assert_eq!(
                        bridge.poll(Direction::HostToGuest, tick),
                        ForwardProgress::Transferred(BURST)
                    );
                    transferred += BURST;
                }
            }
            assert_eq!(transferred, rate);
        }
    }
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
        for direction in [Direction::GuestToHost, Direction::HostToGuest] {
            for bridge in &mut bridges {
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
fn funded_directions_do_not_wait_for_alternation() {
    for first in [Direction::GuestToHost, Direction::HostToGuest] {
        let (relay, now) = setup(1, Duration::from_secs(30), 8 * BURST, BURST as u64);
        let mut bridge = Bridge::new(&relay, 1, now);
        let second = opposite(first);
        bridge.queue(first, &vec![1; BURST], 3);
        bridge.queue(second, &vec![2; BURST], 3);
        assert_eq!(bridge.poll(first, now), ForwardProgress::Transferred(BURST));
        for _ in 0..2 {
            assert_eq!(bridge.poll(first, now), ForwardProgress::Transferred(BURST));
        }
        for _ in 0..3 {
            assert_eq!(bridge.poll(second, now), ForwardProgress::Transferred(BURST));
        }
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
fn unpaid_unfunded_reverse_message_preserves_its_remaining_tokens() {
    let (relay, now) = setup(1, Duration::from_secs(30), 4 * BURST, 2 * BURST as u64);
    let mut bridge = Bridge::new(&relay, 1, now);
    bridge.queue(Direction::HostToGuest, &vec![1; BURST], 1);
    bridge.queue(Direction::HostToGuest, &vec![2; BURST - 100], 1);
    bridge.queue(Direction::HostToGuest, &vec![3; BURST], 1);
    bridge.queue(Direction::GuestToHost, &vec![4; BURST], 2);
    bridge.queue(Direction::GuestToHost, &[5; 100], 1);
    assert_eq!(
        bridge.poll(Direction::HostToGuest, now),
        ForwardProgress::Transferred(BURST)
    );
    for _ in 0..2 {
        assert_eq!(
            bridge.poll(Direction::GuestToHost, now),
            ForwardProgress::Transferred(BURST)
        );
    }
    assert_eq!(
        bridge.poll(Direction::HostToGuest, now),
        ForwardProgress::Transferred(BURST - 100)
    );
    assert_eq!(
        bridge.poll(Direction::GuestToHost, now),
        ForwardProgress::Blocked
    );
    let refilled = now + Duration::from_secs(1);
    assert_eq!(
        bridge.poll(Direction::HostToGuest, refilled),
        ForwardProgress::Transferred(BURST)
    );
    assert_eq!(
        bridge.poll(Direction::GuestToHost, refilled),
        ForwardProgress::Transferred(100)
    );
}

#[test]
fn large_reverse_messages_pass_a_sustained_small_message_flood() {
    for flooded in [Direction::GuestToHost, Direction::HostToGuest] {
        let (relay, now) = setup(1, Duration::from_secs(30), 4 * BURST, 256 * 1024);
        let mut bridge = Bridge::new(&relay, 1, now);
        let reverse = opposite(flooded);
        for direction in [flooded, reverse] {
            bridge.queue(direction, &vec![1; BURST], 2);
        }
        for _ in 0..2 {
            for direction in [flooded, reverse] {
                assert_eq!(bridge.poll(direction, now), ForwardProgress::Transferred(BURST));
            }
        }
        bridge.queue(flooded, &[2; 1024], 1024);
        bridge.queue(reverse, &vec![3; BURST], 8);
        let mut delivered = 0;
        let mut last_delivery_ms = 0;
        let mut flooded_bytes = 0;
        for elapsed_ms in 0..=2000 {
            let tick = now + Duration::from_millis(elapsed_ms);
            while let ForwardProgress::Transferred(count) = bridge.poll(flooded, tick) {
                flooded_bytes += count;
            }
            if bridge.poll(reverse, tick) == ForwardProgress::Transferred(BURST) {
                assert!(elapsed_ms - last_delivery_ms <= 150);
                last_delivery_ms = elapsed_ms;
                delivered += 1;
            }
        }
        assert_eq!(delivered, 8);
        assert!(flooded_bytes >= 256 * 1024);
        let expected: Vec<_> = std::iter::repeat_n(vec![1; BURST], 2)
            .chain(std::iter::repeat_n(vec![3; BURST], 8)).collect();
        assert_eq!(bridge.received(reverse), expected);
    }
}

#[test]
fn large_under_share_pair_passes_a_small_message_flood_within_one_second() {
    for rate in [1_000_000, 256 * 1024] {
        let burst = 65_536;
        let (relay, now) = setup(2, Duration::from_secs(30), burst, rate);
        let mut flooded = Bridge::new(&relay, 1, now);
        let mut waiting = Bridge::new(&relay, 2, now);
        flooded.queue(Direction::GuestToHost, &[1; 1024], 4096);
        waiting.queue(Direction::HostToGuest, &vec![2; BURST], 8);
        let mut total_bytes = 0;
        while let ForwardProgress::Transferred(count) = flooded.poll(Direction::GuestToHost, now) {
            total_bytes += count;
        }
        assert_eq!(total_bytes, burst);
        assert_eq!(waiting.poll(Direction::HostToGuest, now), ForwardProgress::Blocked);
        let mut delivered = 0;
        let mut last_delivery_ms = 0;
        let mut flooded_bytes = 0;
        for elapsed_ms in 1..=2000 {
            let tick = now + Duration::from_millis(elapsed_ms);
            while let ForwardProgress::Transferred(count) = flooded.poll(Direction::GuestToHost, tick) {
                total_bytes += count;
                flooded_bytes += count;
            }
            if waiting.poll(Direction::HostToGuest, tick) == ForwardProgress::Transferred(BURST) {
                assert!(elapsed_ms - last_delivery_ms <= 1000);
                last_delivery_ms = elapsed_ms;
                total_bytes += BURST;
                delivered += 1;
            }
            assert!(total_bytes as u64 <= burst as u64 + rate * elapsed_ms / 1000);
        }
        assert_eq!(delivered, 8);
        assert!(flooded_bytes >= rate as usize);
        assert_eq!(waiting.received(Direction::HostToGuest), vec![vec![2; BURST]; 8]);
    }
}

#[test]
fn stopped_waiter_reservation_expires_without_an_inter_pair_turn() {
    let (relay, now) = setup(2, Duration::from_secs(30), 4 * BURST, 256 * 1024);
    let mut flooded = Bridge::new(&relay, 1, now);
    let mut stopped = Bridge::new(&relay, 2, now);
    flooded.queue(Direction::GuestToHost, &[1; 1024], 256);
    stopped.queue(Direction::HostToGuest, &vec![2; BURST], 1);
    while flooded.poll(Direction::GuestToHost, now) == ForwardProgress::Transferred(1024) {}
    assert_eq!(stopped.poll(Direction::HostToGuest, now), ForwardProgress::Blocked);
    for elapsed_ms in 1..100 {
        let tick = now + Duration::from_millis(elapsed_ms);
        while flooded.poll(Direction::GuestToHost, tick) == ForwardProgress::Transferred(1024) {}
    }
    let mut released = 0;
    let expired = now + Duration::from_millis(100);
    while let ForwardProgress::Transferred(count) = flooded.poll(Direction::GuestToHost, expired) {
        released += count;
    }
    assert!(released >= BURST);
    assert!(stopped.received(Direction::HostToGuest).is_empty());
}

#[test]
fn paid_partial_write_does_not_reserve_reverse_tokens_or_charge_twice() {
    let (relay, now) = setup(1, Duration::from_secs(30), 4 * BURST, 2 * BURST as u64);
    let mut bridge = Bridge::new(&relay, 1, now);
    bridge.queue(Direction::HostToGuest, &vec![1; BURST], 1);
    bridge.queue(Direction::GuestToHost, &vec![2; BURST], 3);
    bridge.guest.write_limit.set(Some(1));
    assert_eq!(
        bridge.poll(Direction::HostToGuest, now),
        ForwardProgress::Transferred(1)
    );
    for _ in 0..3 {
        assert_eq!(
            bridge.poll(Direction::GuestToHost, now),
            ForwardProgress::Transferred(BURST)
        );
    }
    bridge.guest.write_limit.set(None);
    assert_eq!(
        bridge.poll(Direction::HostToGuest, now),
        ForwardProgress::Transferred(BURST - 1)
    );
    assert_eq!(
        bridge.received(Direction::HostToGuest).concat(),
        vec![1; BURST]
    );
}

#[test]
fn contention_yield_uses_recent_usage_without_blocking_a_full_bucket() {
    let (relay, now) = setup(2, Duration::from_secs(30), BURST, BURST as u64);
    let mut busy = Bridge::new(&relay, 1, now);
    let mut quiet = Bridge::new(&relay, 2, now);
    busy.queue(Direction::HostToGuest, &vec![1; BURST], 3);
    quiet.queue(Direction::HostToGuest, &[2; 100], 1);
    assert_eq!(
        busy.poll(Direction::HostToGuest, now),
        ForwardProgress::Transferred(BURST)
    );
    assert_eq!(
        busy.poll(Direction::HostToGuest, now),
        ForwardProgress::Blocked
    );
    assert!(!busy.pair.take_contention_yield());
    assert_eq!(
        quiet.poll(Direction::HostToGuest, now),
        ForwardProgress::Blocked
    );
    assert!(!quiet.pair.take_contention_yield());
    assert_eq!(
        quiet.poll(Direction::HostToGuest, now + Duration::from_millis(10)),
        ForwardProgress::Transferred(100)
    );
    assert_eq!(
        busy.poll(Direction::HostToGuest, now + Duration::from_millis(10)),
        ForwardProgress::Blocked
    );
    assert!(busy.pair.take_contention_yield());
    assert_eq!(
        busy.poll(Direction::HostToGuest, now + Duration::from_secs(1)),
        ForwardProgress::Blocked
    );
    assert!(!busy.pair.take_contention_yield());
    let refilled = now + Duration::from_millis(1010);
    assert_eq!(
        busy.poll(Direction::HostToGuest, refilled),
        ForwardProgress::Transferred(BURST)
    );
    assert_eq!(
        busy.poll(Direction::HostToGuest, refilled),
        ForwardProgress::Blocked
    );
    assert!(!busy.pair.take_contention_yield());
    assert_eq!(
        busy.poll(Direction::HostToGuest, refilled + Duration::from_secs(1)),
        ForwardProgress::Transferred(BURST)
    );
}

#[test]
fn unpaired_slots_do_not_reduce_active_pairs_share() {
    let (relay, now) = setup(3, Duration::from_secs(30), BURST, BURST as u64);
    let mut bridge = Bridge::new(&relay, 1, now);
    let pending: Vec<_> = (2..=3)
        .map(|nonce| {
            relay
                .begin_guest(&signed(nonce, 200), QueueStream::default(), 100, now)
                .unwrap()
        })
        .collect();
    bridge.queue(Direction::HostToGuest, &vec![1; BURST], 2);
    assert_eq!(
        bridge.poll(Direction::HostToGuest, now),
        ForwardProgress::Transferred(BURST)
    );
    assert_eq!(bridge.poll(Direction::HostToGuest, now), ForwardProgress::Blocked);
    assert!(!bridge.pair.take_contention_yield());
    drop(pending);
}

#[test]
fn idle_expired_pair_does_not_reduce_active_pairs_share() {
    let (relay, now) = setup(3, Duration::from_secs(1), BURST, BURST as u64);
    let mut expired = Bridge::new(&relay, 1, now);
    let active_at = now + Duration::from_millis(500);
    let mut active = Bridge::new(&relay, 2, active_at);
    let pending = relay
        .begin_guest(&signed(3, 200), QueueStream::default(), 100, active_at)
        .unwrap();
    active.queue(Direction::HostToGuest, &vec![1; BURST], 2);
    assert_eq!(
        active.poll(Direction::HostToGuest, active_at),
        ForwardProgress::Transferred(BURST)
    );
    let expired_at = now + Duration::from_millis(1001);
    assert_eq!(
        active.poll(Direction::HostToGuest, expired_at),
        ForwardProgress::Blocked
    );
    assert!(!active.pair.take_contention_yield());
    assert!(matches!(
        expired.pair.forward_once(Direction::HostToGuest, expired_at),
            Err(ForwardError::Relay(RelayError::TimedOut))
    ));
    drop(pending);
}

#[test]
fn saturated_direction_keeps_sustained_reverse_input_below_200_ms() {
    for saturated in [Direction::GuestToHost, Direction::HostToGuest] {
        let rate = 256 * 1024;
        let (relay, now) = setup(1, Duration::from_secs(30), BURST, rate);
        let mut bridge = Bridge::new(&relay, 1, now);
        let reverse = opposite(saturated);
        bridge.queue(saturated, &vec![57; BURST], 128);
        let mut waiting = VecDeque::new();
        let mut sent = 0;
        let mut delivered = 0;
        let mut saturated_bytes = 0;
        for elapsed_ms in 0..=3200 {
            if sent < 90 && elapsed_ms >= sent * 1000 / 30 {
                bridge.queue(reverse, &[sent as u8; 100], 1);
                waiting.push_back(elapsed_ms);
                sent += 1;
            }
            let tick = now + Duration::from_millis(elapsed_ms);
            if let ForwardProgress::Transferred(count) = bridge.poll(saturated, tick) {
                saturated_bytes += count;
            }
            if bridge.poll(reverse, tick) == ForwardProgress::Transferred(100) {
                let queued_at = waiting.pop_front().unwrap();
                assert!(elapsed_ms - queued_at < 200, "input delayed at {elapsed_ms} ms");
                delivered += 1;
            }
        }
        assert_eq!(delivered, 90);
        assert!(waiting.is_empty());
        assert!(saturated_bytes >= 3 * rate as usize);
        let expected: Vec<_> = (0..90).map(|message| vec![message; 100]).collect();
        assert_eq!(bridge.received(reverse), expected);
    }
}
