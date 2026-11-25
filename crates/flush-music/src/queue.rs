//! `TrackQueue`: bounded FIFO with index-based edits, mirroring yuzuha's.

use std::collections::VecDeque;

use crate::error::MusicError;

/// FIFO with index-based insertion, removal, moves and shuffles, capped at
/// `capacity` live entries.
pub struct TrackQueue<T> {
    entries: VecDeque<T>,
    capacity: usize,
}

impl<T> TrackQueue<T> {
    pub fn new(capacity: usize) -> Result<Self, MusicError> {
        if !(1..=100_000).contains(&capacity) {
            return Err(MusicError::new(
                "MUSIC_QUEUE_LIMIT",
                "The queue accepts between 1 and 100000 items.",
            ));
        }
        Ok(Self {
            entries: VecDeque::new(),
            capacity,
        })
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn check_capacity(&self, extra: usize) -> Result<(), MusicError> {
        if extra > self.capacity - self.entries.len() {
            return Err(MusicError::new(
                "MUSIC_QUEUE_LIMIT",
                "The music queue is full.",
            ));
        }
        Ok(())
    }

    fn index(&self, index: usize, end: bool) -> Result<(), MusicError> {
        let max = self.entries.len().saturating_sub(if end { 0 } else { 1 });
        if index > max || (!end && self.entries.is_empty()) {
            return Err(MusicError::new("MUSIC_QUEUE_INDEX", "Index out of queue."));
        }
        Ok(())
    }

    fn count(&self, count: usize, max: usize) -> Result<(), MusicError> {
        if count > max {
            return Err(MusicError::new("MUSIC_QUEUE_INDEX", "Count out of queue."));
        }
        Ok(())
    }

    pub fn enqueue(&mut self, item: T) -> Result<usize, MusicError> {
        self.add([item])
    }

    pub fn add(&mut self, items: impl IntoIterator<Item = T>) -> Result<usize, MusicError> {
        let items: Vec<T> = items.into_iter().collect();
        self.check_capacity(items.len())?;
        self.entries.extend(items);
        Ok(self.entries.len())
    }

    pub fn peek(&self) -> Option<&T> {
        self.entries.front()
    }

    pub fn at(&self, index: usize) -> Result<Option<&T>, MusicError> {
        self.index(index, false)?;
        Ok(self.entries.get(index))
    }

    pub fn slice(&self, start: usize, count: usize) -> Result<Vec<T>, MusicError>
    where
        T: Clone,
    {
        self.index(start, true)?;
        self.count(count, self.entries.len() - start)?;
        Ok(self
            .entries
            .iter()
            .skip(start)
            .take(count)
            .cloned()
            .collect())
    }

    pub fn index_of(&self, item: &T) -> Option<usize>
    where
        T: PartialEq,
    {
        self.entries.iter().position(|e| e == item)
    }

    pub fn insert(
        &mut self,
        index: usize,
        items: impl IntoIterator<Item = T>,
    ) -> Result<usize, MusicError> {
        let items: Vec<T> = items.into_iter().collect();
        self.index(index, true)?;
        self.check_capacity(items.len())?;
        for (offset, item) in items.into_iter().enumerate() {
            self.entries.insert(index + offset, item);
        }
        Ok(self.entries.len())
    }

    pub fn remove(&mut self, index: usize, count: usize) -> Result<Vec<T>, MusicError> {
        self.index(index, count == 0)?;
        self.count(count, self.entries.len() - index)?;
        let mut removed = Vec::with_capacity(count);
        for _ in 0..count {
            if let Some(item) = self.entries.remove(index) {
                removed.push(item);
            }
        }
        Ok(removed)
    }

    pub fn move_item(&mut self, from: usize, to: usize) -> Result<(), MusicError> {
        self.index(from, false)?;
        self.index(to, false)?;
        if from == to {
            return Ok(());
        }
        let item = self.entries.remove(from).unwrap();
        self.entries.insert(to, item);
        Ok(())
    }

    /// Fisher–Yates shuffle; `random` must return a value in [0, 1).
    pub fn shuffle<F>(&mut self, mut random: F) -> Result<(), MusicError>
    where
        F: FnMut() -> f64,
    {
        let mut entries: Vec<T> = self.entries.drain(..).collect();
        for index in (1..entries.len()).rev() {
            let value = random();
            if !(value.is_finite() && (0.0..1.0).contains(&value)) {
                self.entries = entries.into();
                return Err(MusicError::new(
                    "MUSIC_QUEUE_RANDOM",
                    "random must return a number in [0, 1).",
                ));
            }
            let other = (value * (index + 1) as f64) as usize;
            entries.swap(index, other);
        }
        self.entries = entries.into();
        Ok(())
    }

    pub fn dequeue(&mut self) -> Option<T> {
        self.entries.pop_front()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn to_vec(&self) -> Vec<T>
    where
        T: Clone,
    {
        self.entries.iter().cloned().collect()
    }
}
