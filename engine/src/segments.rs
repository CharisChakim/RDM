use serde::{Deserialize, Serialize};

/// A byte range of the file owned by at most one connection.
/// Invariant: start <= done <= claimed <= end.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Segment {
    /// Where this segment began (only used for display).
    pub start: u64,
    /// Exclusive end. Shrinks when another connection steals the tail.
    pub end: u64,
    /// Bytes up to here are written to disk.
    pub done: u64,
    /// Bytes up to here are received (may still sit in a worker's buffer).
    #[serde(skip)]
    pub claimed: u64,
    #[serde(skip)]
    pub owned: bool,
}

impl Segment {
    pub fn remaining(&self) -> u64 {
        self.end - self.claimed
    }
}

#[derive(Debug)]
pub struct Segments {
    pub list: Vec<Segment>,
    pub min_split: u64,
}

impl Segments {
    pub fn new(total: u64, min_split: u64) -> Self {
        let first = Segment { start: 0, end: total, done: 0, claimed: 0, owned: false };
        Self { list: vec![first], min_split }
    }

    pub fn restore(mut list: Vec<Segment>, min_split: u64) -> Self {
        for s in &mut list {
            s.claimed = s.done;
        }
        Self { list, min_split }
    }

    /// Hand work to an idle connection: an orphaned unfinished segment if any,
    /// otherwise split the segment with the most bytes left (IDM-style dynamic segmentation).
    pub fn acquire(&mut self) -> Option<usize> {
        if let Some(i) = self.list.iter().position(|s| !s.owned && s.remaining() > 0) {
            self.list[i].owned = true;
            return Some(i);
        }
        let (i, rem) = self
            .list
            .iter()
            .enumerate()
            .map(|(i, s)| (i, s.remaining()))
            .max_by_key(|&(_, r)| r)?;
        if rem < 2 * self.min_split {
            return None;
        }
        let mid = self.list[i].claimed + rem / 2;
        let end = self.list[i].end;
        self.list[i].end = mid;
        self.list.push(Segment { start: mid, end, done: mid, claimed: mid, owned: true });
        Some(self.list.len() - 1)
    }

    pub fn finished(&self) -> bool {
        self.list.iter().all(|s| s.done == s.end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_covers_file_without_overlap() {
        let mut segs = Segments::new(100, 10);
        assert_eq!(segs.acquire(), Some(0));
        segs.list[0].claimed = 20;
        assert_eq!(segs.acquire(), Some(1)); // splits [20,100) at 60
        assert_eq!((segs.list[0].end, segs.list[1].start, segs.list[1].end), (60, 60, 100));
        assert_eq!(segs.acquire(), Some(2)); // splits [60,100) at 80
        assert_eq!((segs.list[1].end, segs.list[2].start), (80, 80));
    }

    #[test]
    fn refuses_tiny_splits_and_reuses_orphans() {
        let mut segs = Segments::new(15, 10);
        assert_eq!(segs.acquire(), Some(0));
        assert_eq!(segs.acquire(), None);
        segs.list[0].owned = false;
        assert_eq!(segs.acquire(), Some(0));
    }
}
