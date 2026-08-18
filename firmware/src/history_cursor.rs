pub(crate) const FIRST_CURSOR: u32 = u32::MAX;

#[derive(Clone, Copy, Default)]
pub(crate) struct HistoryWalk {
    pub(crate) active: bool,
    pub(crate) next_slot: u32,
    pub(crate) visited_slots: u32,
    pub(crate) next_cursor: u32,
    pub(crate) end_sequence: u32,
    pub(crate) revision: u32,
}

impl HistoryWalk {
    pub(crate) fn restart(&mut self, start_slot: u32, end_sequence: u32, revision: u32) {
        self.active = true;
        self.next_slot = start_slot;
        self.visited_slots = 0;
        self.next_cursor = FIRST_CURSOR;
        self.end_sequence = end_sequence;
        self.revision = revision;
    }

    pub(crate) fn can_continue(&self, cursor: u32, revision: u32) -> bool {
        self.active && self.revision == revision && self.next_cursor == cursor
    }

    pub(crate) fn advance_slot(&mut self, capacity: u32) {
        self.next_slot = (self.next_slot + 1) % capacity;
        self.visited_slots += 1;
    }

    pub(crate) fn set_next_cursor(&mut self, cursor: u32) {
        self.next_cursor = cursor;
        if cursor == 0 {
            self.active = false;
        }
    }

    pub(crate) fn stop(&mut self) {
        self.active = false;
        self.next_cursor = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::{FIRST_CURSOR, HistoryWalk};

    fn page(
        walk: &mut HistoryWalk,
        slots: &[Option<u32>],
        start_slot: u32,
        cursor: u32,
        end_sequence: u32,
        revision: u32,
        limit: usize,
    ) -> (Vec<u32>, u32) {
        if cursor == FIRST_CURSOR || !walk.can_continue(cursor, revision) {
            walk.restart(start_slot, end_sequence, revision);
        }
        let start = if cursor == FIRST_CURSOR {
            let mut slot = start_slot as usize;
            let mut oldest = end_sequence;
            for _ in 0..slots.len() {
                if let Some(sequence) = slots[slot] {
                    oldest = sequence;
                    break;
                }
                slot = (slot + 1) % slots.len();
            }
            oldest
        } else {
            cursor
        };
        let mut records = Vec::new();
        let mut next_cursor = 0;
        while walk.visited_slots < slots.len() as u32 && records.len() < limit {
            let slot = walk.next_slot as usize;
            let record = slots[slot];
            walk.advance_slot(slots.len() as u32);
            let Some(sequence) = record else { continue };
            if sequence < start || sequence > walk.end_sequence {
                continue;
            }
            records.push(sequence);
            next_cursor = if sequence == walk.end_sequence {
                0
            } else {
                sequence + 1
            };
            if next_cursor == 0 {
                break;
            }
        }
        walk.set_next_cursor(next_cursor);
        (records, next_cursor)
    }

    #[test]
    fn walks_slots_across_ring_wrap() {
        let mut walk = HistoryWalk::default();
        walk.restart(6, 20, 1);
        assert_eq!(walk.next_slot, 6);
        walk.advance_slot(8);
        assert_eq!(walk.next_slot, 7);
        walk.advance_slot(8);
        assert_eq!(walk.next_slot, 0);
        walk.advance_slot(8);
        assert_eq!(walk.next_slot, 1);
        assert_eq!(walk.visited_slots, 3);
    }

    #[test]
    fn only_continues_for_the_expected_cursor_and_revision() {
        let mut walk = HistoryWalk::default();
        walk.restart(0, 5, 9);
        walk.set_next_cursor(3);
        assert!(walk.can_continue(3, 9));
        assert!(!walk.can_continue(FIRST_CURSOR, 9));
        assert!(!walk.can_continue(3, 10));
    }

    #[test]
    fn zero_cursor_stops_the_walk() {
        let mut walk = HistoryWalk::default();
        walk.restart(0, 5, 1);
        walk.set_next_cursor(0);
        assert!(!walk.active);
        assert!(!walk.can_continue(0, 1));
    }

    #[test]
    fn empty_history_returns_no_cursor() {
        let mut walk = HistoryWalk::default();
        let (records, next) = page(&mut walk, &[None, None, None], 0, FIRST_CURSOR, 0, 1, 5);
        assert!(records.is_empty());
        assert_eq!(next, 0);
    }

    #[test]
    fn multiple_pages_continue_from_the_previous_physical_slot() {
        let slots = [
            Some(1),
            Some(2),
            Some(3),
            Some(4),
            Some(5),
            Some(6),
            Some(7),
        ];
        let mut walk = HistoryWalk::default();
        let (first, cursor) = page(&mut walk, &slots, 0, FIRST_CURSOR, 7, 1, 3);
        let (second, cursor) = page(&mut walk, &slots, 0, cursor, 7, 1, 3);
        let (third, cursor) = page(&mut walk, &slots, 0, cursor, 7, 1, 3);
        assert_eq!(first, vec![1, 2, 3]);
        assert_eq!(second, vec![4, 5, 6]);
        assert_eq!(third, vec![7]);
        assert_eq!(cursor, 0);
    }

    #[test]
    fn partial_and_wraparound_pages_skip_invalid_slots() {
        let slots = [
            Some(3),
            None,
            Some(4),
            Some(5),
            None,
            None,
            Some(1),
            Some(2),
        ];
        let mut walk = HistoryWalk::default();
        let (first, cursor) = page(&mut walk, &slots, 6, FIRST_CURSOR, 5, 1, 3);
        let (second, cursor) = page(&mut walk, &slots, 6, cursor, 5, 1, 3);
        assert_eq!(first, vec![1, 2, 3]);
        assert_eq!(second, vec![4, 5]);
        assert_eq!(cursor, 0);
    }

    #[test]
    fn cursor_beyond_newest_returns_empty() {
        let slots = [Some(10), Some(11), Some(12)];
        let mut walk = HistoryWalk::default();
        let (records, next) = page(&mut walk, &slots, 0, 20, 12, 1, 5);
        assert!(records.is_empty());
        assert_eq!(next, 0);
    }
}
