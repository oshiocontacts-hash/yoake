use std::collections::BTreeMap;

const MAX_TICKETS: usize = 4096;
const MAX_OWNER_BYTES: usize = 128;
const MAX_ERROR_BYTES: usize = 1024;

struct Ticket<T> {
    owner: String,
    result: Option<Result<T, String>>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum TicketPoll<T> {
    Pending,
    Ready(Result<T, String>),
}

pub struct RequestTickets<T> {
    max_per_owner: usize,
    max_total: usize,
    next_id: u64,
    tickets: BTreeMap<u64, Ticket<T>>,
    owner_counts: BTreeMap<String, usize>,
}

impl<T> RequestTickets<T> {
    pub fn new(max_per_owner: usize, max_total: usize) -> Result<Self, &'static str> {
        if max_per_owner == 0 || max_total == 0 || max_per_owner > max_total || max_total > MAX_TICKETS {
            return Err("invalid request ticket limits");
        }
        Ok(Self {
            max_per_owner,
            max_total,
            next_id: 1,
            tickets: BTreeMap::new(),
            owner_counts: BTreeMap::new(),
        })
    }

    pub fn start(&mut self, owner: &str) -> Result<u64, &'static str> {
        if owner.is_empty() || owner.len() > MAX_OWNER_BYTES {
            return Err("invalid request ticket owner");
        }
        if self.tickets.len() >= self.max_total
            || self.owner_counts.get(owner).copied().unwrap_or(0) >= self.max_per_owner
        {
            return Err("request ticket quota exceeded");
        }
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or("request ticket ids exhausted")?;
        self.tickets.insert(id, Ticket { owner: owner.to_owned(), result: None });
        *self.owner_counts.entry(owner.to_owned()).or_insert(0) += 1;
        Ok(id)
    }

    pub fn complete(&mut self, owner: &str, id: u64, result: Result<T, String>) -> Result<(), &'static str> {
        if result.as_ref().err().is_some_and(|error| error.len() > MAX_ERROR_BYTES) {
            return Err("request error exceeds limit");
        }
        let ticket = self.tickets.get_mut(&id).filter(|ticket| ticket.owner == owner)
            .ok_or("unknown request ticket")?;
        if ticket.result.is_some() {
            return Err("request ticket already completed");
        }
        ticket.result = Some(result);
        Ok(())
    }

    pub fn poll(&mut self, owner: &str, id: u64) -> Result<TicketPoll<T>, &'static str> {
        let ticket = self.tickets.get(&id).filter(|ticket| ticket.owner == owner)
            .ok_or("unknown request ticket")?;
        if ticket.result.is_none() {
            return Ok(TicketPoll::Pending);
        }
        let ticket = self.remove(id).ok_or("unknown request ticket")?;
        Ok(TicketPoll::Ready(ticket.result.ok_or("request is pending")?))
    }

    pub fn cancel(&mut self, owner: &str, id: u64) -> bool {
        if self.tickets.get(&id).is_none_or(|ticket| ticket.owner != owner) {
            return false;
        }
        self.remove(id).is_some()
    }

    pub fn remove_owner(&mut self, owner: &str) {
        self.tickets.retain(|_, ticket| ticket.owner != owner);
        self.owner_counts.remove(owner);
    }

    fn remove(&mut self, id: u64) -> Option<Ticket<T>> {
        let ticket = self.tickets.remove(&id)?;
        if let Some(count) = self.owner_counts.get_mut(&ticket.owner) {
            *count -= 1;
            if *count == 0 {
                self.owner_counts.remove(&ticket.owner);
            }
        }
        Some(ticket)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tickets_are_owner_scoped_and_results_are_consumed_once() {
        let mut tickets = RequestTickets::new(1, 2).unwrap();
        let first = tickets.start("first").unwrap();
        assert_eq!(tickets.poll("first", first), Ok(TicketPoll::<u8>::Pending));
        assert!(tickets.poll("second", first).is_err());
        assert!(tickets.complete("second", first, Ok(1)).is_err());
        assert!(!tickets.cancel("second", first));
        tickets.complete("first", first, Ok(42)).unwrap();
        assert!(tickets.complete("first", first, Ok(43)).is_err());
        assert!(tickets.start("first").is_err());
        assert_eq!(tickets.poll("first", first), Ok(TicketPoll::Ready(Ok(42))));
        assert!(tickets.poll("first", first).is_err());
        assert!(tickets.start("first").is_ok());
    }

    #[test]
    fn limits_count_pending_and_uncollected_results() {
        let mut tickets = RequestTickets::<()>::new(2, 2).unwrap();
        let first = tickets.start("first").unwrap();
        tickets.complete("first", first, Err("failed".into())).unwrap();
        let second = tickets.start("first").unwrap();
        assert!(tickets.start("first").is_err());
        assert!(tickets.start("second").is_err());
        assert_eq!(tickets.poll("first", first), Ok(TicketPoll::Ready(Err("failed".into()))));
        let third = tickets.start("second").unwrap();
        assert!(tickets.cancel("first", second));
        assert!(tickets.cancel("second", third));
        assert!(!tickets.cancel("second", third));
    }

    #[test]
    fn cancellation_and_unload_prevent_late_completions() {
        let mut tickets = RequestTickets::<()>::new(2, 3).unwrap();
        let cancelled = tickets.start("first").unwrap();
        let removed = tickets.start("first").unwrap();
        let retained = tickets.start("second").unwrap();
        assert!(tickets.cancel("first", cancelled));
        tickets.remove_owner("first");
        assert!(tickets.complete("first", cancelled, Ok(())).is_err());
        assert!(tickets.complete("first", removed, Ok(())).is_err());
        assert_eq!(tickets.poll("second", retained), Ok(TicketPoll::Pending));
        assert_ne!(tickets.start("first").unwrap(), removed);
    }

    #[test]
    fn rejects_invalid_limits_owners_and_oversize_errors() {
        assert!(RequestTickets::<()>::new(0, 1).is_err());
        assert!(RequestTickets::<()>::new(2, 1).is_err());
        assert!(RequestTickets::<()>::new(1, MAX_TICKETS + 1).is_err());
        let mut tickets = RequestTickets::<()>::new(1, 1).unwrap();
        assert!(tickets.start("").is_err());
        assert!(tickets.start(&"x".repeat(MAX_OWNER_BYTES + 1)).is_err());
        let id = tickets.start("first").unwrap();
        assert!(tickets.complete("first", id, Err("x".repeat(MAX_ERROR_BYTES + 1))).is_err());
        assert_eq!(tickets.poll("first", id), Ok(TicketPoll::Pending));
        tickets.complete("first", id, Ok(())).unwrap();
    }
}
