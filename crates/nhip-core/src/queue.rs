//! Playback order: what plays now, next and before. No audio or UI in here.

use std::collections::VecDeque;

use rand::seq::SliceRandom;
use rand::SeedableRng;
use serde::{Deserialize, Serialize};

use crate::library::{Session, TrackId};
use crate::mechanics::PREVIOUS_RESTART_MS;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum Repeat {
    #[default]
    Off,
    Context,
    Track,
}

impl Repeat {
    pub fn cycled(self) -> Self {
        match self {
            Repeat::Off => Repeat::Context,
            Repeat::Context => Repeat::Track,
            Repeat::Track => Repeat::Off,
        }
    }
}

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub enum ContextRef {
    #[default]
    None,
    Liked,
    Playlist(u64),
    Album(String),
    Artist(String),
    All,
    Autoplay,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueueSnapshot {
    context_ref: ContextRef,
    order: Vec<usize>,
    pos: usize,
    from_queue: bool,
    manual: VecDeque<TrackId>,
}

#[derive(Debug, PartialEq)]
pub enum Previous {
    Restart,
    Track(TrackId),
}

pub struct Player {
    pub context: Vec<TrackId>,
    pub context_ref: ContextRef,
    pub context_name: String,
    order: Vec<usize>,
    pos: usize,
    pub queue: VecDeque<TrackId>,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub current: Option<TrackId>,
    /// The current song came out of the user queue, so `pos` still points at the context song.
    from_queue: bool,
    pub history: Vec<TrackId>,
    rng: rand::rngs::StdRng,
}

impl Default for Player {
    fn default() -> Self {
        Self::with_seed(rand::random())
    }
}

impl Player {
    pub fn with_seed(seed: u64) -> Self {
        Self {
            context: Vec::new(),
            context_ref: ContextRef::None,
            context_name: String::new(),
            order: Vec::new(),
            pos: 0,
            queue: VecDeque::new(),
            shuffle: false,
            repeat: Repeat::Off,
            current: None,
            from_queue: false,
            history: Vec::new(),
            rng: rand::rngs::StdRng::seed_from_u64(seed),
        }
    }

    /// Starts `tracks` as the new context; `start` is an index into `tracks`, or `None` to begin
    /// at the top (or anywhere, when shuffling).
    pub fn play_context(
        &mut self,
        context_ref: ContextRef,
        name: String,
        tracks: Vec<TrackId>,
        start: Option<usize>,
    ) -> Option<TrackId> {
        if tracks.is_empty() {
            return None;
        }
        self.context = tracks;
        self.context_ref = context_ref;
        self.context_name = name;
        self.from_queue = false;
        self.rebuild_order(start);
        self.pos = 0;
        let id = self.context[self.order[0]];
        self.set_current(id);
        Some(id)
    }

    fn set_current(&mut self, id: TrackId) {
        if let Some(prev) = self.current {
            if prev != id {
                self.history.push(prev);
                if self.history.len() > 200 {
                    self.history.remove(0);
                }
            }
        }
        self.current = Some(id);
    }

    /// Puts `first` (a context index) first and fills the rest in play order.
    fn rebuild_order(&mut self, first: Option<usize>) {
        let n = self.context.len();
        let mut order: Vec<usize> = (0..n).collect();
        if self.shuffle {
            order.shuffle(&mut self.rng);
            if let Some(first) = first {
                if let Some(p) = order.iter().position(|&i| i == first) {
                    order.swap(0, p);
                }
            }
        } else if let Some(first) = first {
            order.rotate_left(first.min(n.saturating_sub(1)));
        }
        self.order = order;
    }

    pub fn toggle_shuffle(&mut self) {
        self.set_shuffle(!self.shuffle);
    }

    pub fn set_shuffle(&mut self, on: bool) {
        if self.shuffle == on {
            return;
        }
        self.shuffle = on;
        if self.context.is_empty() {
            return;
        }
        let here = self.order.get(self.pos).copied();
        self.rebuild_order(here);
        if !on {
            // Back to list order: continue from where the current song sits in the list.
            self.order = (0..self.context.len()).collect();
            self.pos = here.unwrap_or(0);
        } else {
            self.pos = 0;
        }
    }

    pub fn cycle_repeat(&mut self) {
        self.repeat = self.repeat.cycled();
    }

    /// The song that will play after the current one finishes on its own.
    pub fn peek_next(&self) -> Option<TrackId> {
        if self.repeat == Repeat::Track {
            return self.current;
        }
        if let Some(&id) = self.queue.front() {
            return Some(id);
        }
        self.next_context_pos().map(|p| self.context[self.order[p]])
    }

    fn next_context_pos(&self) -> Option<usize> {
        if self.order.is_empty() {
            return None;
        }
        if self.pos + 1 < self.order.len() {
            Some(self.pos + 1)
        } else if self.repeat == Repeat::Context {
            Some(0)
        } else {
            None
        }
    }

    /// Moves to the next song. `natural` means the previous song ended by itself; a manual skip
    /// always moves on, even in repeat-one.
    pub fn advance(&mut self, natural: bool) -> Option<TrackId> {
        if natural && self.repeat == Repeat::Track {
            return self.current;
        }
        if let Some(id) = self.queue.pop_front() {
            self.from_queue = true;
            self.set_current(id);
            return Some(id);
        }
        let next = self.next_context_pos()?;
        self.pos = next;
        self.from_queue = false;
        let id = self.context[self.order[next]];
        self.set_current(id);
        Some(id)
    }

    pub fn previous(&mut self, position_ms: u64) -> Previous {
        if position_ms > PREVIOUS_RESTART_MS {
            return Previous::Restart;
        }
        if self.from_queue {
            self.from_queue = false;
            if let Some(&i) = self.order.get(self.pos) {
                let id = self.context[i];
                self.set_current(id);
                return Previous::Track(id);
            }
            return Previous::Restart;
        }
        if self.order.is_empty() {
            return Previous::Restart;
        }
        let prev = if self.pos > 0 {
            self.pos - 1
        } else if self.repeat == Repeat::Context {
            self.order.len() - 1
        } else {
            return Previous::Restart;
        };
        self.pos = prev;
        let id = self.context[self.order[prev]];
        self.set_current(id);
        Previous::Track(id)
    }

    pub fn enqueue(&mut self, id: TrackId) {
        self.queue.push_back(id);
    }

    pub fn dequeue(&mut self, index: usize) {
        self.queue.remove(index);
    }

    /// Reorders a manually queued item from `from` to `to`. Returns `false` if either index is out of bounds.
    pub fn move_queue_item(&mut self, from: usize, to: usize) -> bool {
        if from >= self.queue.len() || to >= self.queue.len() {
            return false;
        }
        if from == to {
            return true;
        }
        if let Some(item) = self.queue.remove(from) {
            self.queue.insert(to, item);
            true
        } else {
            false
        }
    }

    /// Inserts a track into the manual queue at `index` (clamped to `0..=queue.len()`).
    pub fn insert_queue_item(&mut self, index: usize, id: TrackId) {
        let at = index.min(self.queue.len());
        self.queue.insert(at, id);
    }

    pub fn clear_queue(&mut self) {
        self.queue.clear();
    }

    /// Appends autoplay / radio tracks to the end of the live context without interrupting the current song.
    /// Returns the number of newly appended unique tracks.
    pub fn append_context(&mut self, tracks: Vec<TrackId>) -> usize {
        if tracks.is_empty() {
            return 0;
        }
        let mut added = 0;
        for id in tracks {
            if self.current == Some(id) || self.context.contains(&id) || self.queue.contains(&id) {
                continue;
            }
            let idx = self.context.len();
            self.context.push(id);
            self.order.push(idx);
            added += 1;
        }
        added
    }

    /// Context songs still to come, in play order.
    pub fn upcoming(&self, max: usize) -> Vec<TrackId> {
        let mut out = Vec::new();
        if self.order.is_empty() {
            return out;
        }
        let mut p = self.pos;
        for _ in 0..self.order.len() {
            p += 1;
            if p >= self.order.len() {
                if self.repeat == Repeat::Context {
                    p = 0;
                } else {
                    break;
                }
            }
            if out.len() >= max {
                break;
            }
            out.push(self.context[self.order[p]]);
        }
        out
    }

    /// Jumps straight to a song in the upcoming list, dropping what was before it.
    pub fn jump_to_upcoming(&mut self, id: TrackId) -> Option<TrackId> {
        let mut p = self.pos;
        for _ in 0..self.order.len() {
            p += 1;
            if p >= self.order.len() {
                if self.repeat == Repeat::Context {
                    p = 0;
                } else {
                    return None;
                }
            }
            if self.context[self.order[p]] == id {
                self.pos = p;
                self.from_queue = false;
                self.set_current(id);
                return Some(id);
            }
        }
        None
    }

    /// The list being played changed (songs added or removed): keep playing, adopt the new list.
    pub fn update_context(&mut self, tracks: Vec<TrackId>) {
        let current = self.current;
        self.context = tracks;
        let at = current.and_then(|c| self.context.iter().position(|&t| t == c));
        self.rebuild_order(at);
        self.pos = if self.shuffle {
            0
        } else {
            at.unwrap_or(0)
        };
        if !self.shuffle {
            self.order = (0..self.context.len()).collect();
        }
    }

    pub fn snapshot(&self, position_ms: u64) -> Session {
        Session {
            current: self.current,
            position_ms,
            context_name: self.context_name.clone(),
            context: self.context.clone(),
            queue: Some(QueueSnapshot {
                context_ref: self.context_ref.clone(),
                order: self.order.clone(),
                pos: self.pos,
                from_queue: self.from_queue,
                manual: self.queue.clone(),
            }),
        }
    }

    /// Restores a saved session without treating it as a play.
    pub fn restore(&mut self, session: Session, available: impl Fn(TrackId) -> bool) -> Option<(TrackId, u64)> {
        self.queue = session.queue.as_ref().map(|q| q.manual.iter().copied().filter(|&id| available(id)).collect()).unwrap_or_default();
        let current = session.current.filter(|&id| available(id))?;
        self.current = Some(current);
        self.context_name = session.context_name;
        self.context_ref = session.queue.as_ref().map(|q| q.context_ref.clone()).unwrap_or_default();
        let count = session.context.len();
        self.context = session.context.into_iter().filter(|&id| available(id)).collect();
        if let Some(saved) = session.queue.filter(|q| count == self.context.len() && q.valid(&self.context, current)) {
            self.order = saved.order;
            self.pos = saved.pos;
            self.from_queue = saved.from_queue;
            return Some((current, session.position_ms));
        }
        if self.context.is_empty() {
            self.context.push(current);
        }
        self.from_queue = false;
        let at = self.context.iter().position(|&t| t == current);
        self.rebuild_order(at);
        self.pos = if self.shuffle { 0 } else { at.unwrap_or(0) };
        if !self.shuffle {
            self.order = (0..self.context.len()).collect();
        }
        Some((current, session.position_ms))
    }
}

impl QueueSnapshot {
    fn valid(&self, context: &[TrackId], current: TrackId) -> bool {
        if self.order.len() != context.len()
            || !self.order.get(self.pos).and_then(|&i| context.get(i)).is_some_and(|&id| self.from_queue || id == current)
        {
            return false;
        }
        let mut seen = vec![false; context.len()];
        self.order.iter().all(|&index| seen.get_mut(index).is_some_and(|value| !std::mem::replace(value, true)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_preserves_manual_queue_and_shuffled_context() {
        let mut before = player(10);
        before.set_shuffle(true);
        before.advance(false);
        before.enqueue(99);
        before.enqueue(98);
        before.advance(false);
        let data = serde_json::to_vec(&before.snapshot(12_345)).unwrap();
        let mut restored = Player::with_seed(25);
        restored.shuffle = true;
        assert_eq!(restored.restore(serde_json::from_slice(&data).unwrap(), |_| true), Some((99, 12_345)));
        assert_eq!(restored.queue, before.queue);
        assert_eq!(restored.upcoming(20), before.upcoming(20));
        assert_eq!(restored.advance(false), before.advance(false));
        assert_eq!(restored.advance(false), before.advance(false));
    }

    #[test]
    fn old_session_restores_with_missing_tracks_filtered() {
        let old = serde_json::from_str(r#"{"current":2,"position_ms":300,"context":[1,2,3,4]}"#).unwrap();
        let mut restored = Player::with_seed(7);
        assert_eq!(restored.restore(old, |id| id != 3), Some((2, 300)));
        assert_eq!(restored.peek_next(), Some(4));
    }

    fn player(n: u64) -> Player {
        let mut p = Player::with_seed(7);
        p.play_context(ContextRef::All, "x".into(), (1..=n).collect(), Some(0));
        p
    }

    #[test]
    fn plays_in_order_and_stops_at_end() {
        let mut p = player(3);
        assert_eq!(p.current, Some(1));
        assert_eq!(p.advance(true), Some(2));
        assert_eq!(p.advance(true), Some(3));
        assert_eq!(p.advance(true), None);
    }

    #[test]
    fn repeat_context_wraps() {
        let mut p = player(2);
        p.repeat = Repeat::Context;
        p.advance(true);
        assert_eq!(p.peek_next(), Some(1));
        assert_eq!(p.advance(true), Some(1));
    }

    #[test]
    fn repeat_one_holds_on_natural_end_only() {
        let mut p = player(3);
        p.repeat = Repeat::Track;
        assert_eq!(p.peek_next(), Some(1));
        assert_eq!(p.advance(true), Some(1));
        assert_eq!(p.advance(false), Some(2));
    }

    #[test]
    fn user_queue_goes_first_and_returns_to_context() {
        let mut p = player(3);
        p.enqueue(99);
        assert_eq!(p.peek_next(), Some(99));
        assert_eq!(p.advance(true), Some(99));
        assert_eq!(p.advance(true), Some(2));
    }

    #[test]
    fn previous_restarts_after_threshold() {
        let mut p = player(3);
        p.advance(true);
        assert_eq!(p.previous(PREVIOUS_RESTART_MS + 1), Previous::Restart);
        assert_eq!(p.previous(100), Previous::Track(1));
        assert_eq!(p.previous(100), Previous::Restart);
    }

    #[test]
    fn previous_from_queue_song_returns_to_context_song() {
        let mut p = player(3);
        p.enqueue(99);
        p.advance(true);
        assert_eq!(p.previous(0), Previous::Track(1));
        assert_eq!(p.advance(true), Some(2));
    }

    #[test]
    fn shuffle_keeps_current_and_covers_everything() {
        let mut p = player(50);
        p.advance(true);
        p.set_shuffle(true);
        assert_eq!(p.current, Some(2));
        let mut seen = vec![2];
        while let Some(id) = p.advance(true) {
            seen.push(id);
        }
        seen.sort_unstable();
        assert_eq!(seen, (1..=50).collect::<Vec<_>>());
    }

    #[test]
    fn unshuffle_continues_after_current_in_list_order() {
        let mut p = player(10);
        p.set_shuffle(true);
        p.advance(true);
        let c = p.current.unwrap();
        p.set_shuffle(false);
        if c < 10 {
            assert_eq!(p.peek_next(), Some(c + 1));
        } else {
            assert_eq!(p.peek_next(), None);
        }
    }

    #[test]
    fn move_queue_item_reorders_forward_backward_and_rejects_oob() {
        let mut p = player(2);
        for id in [10, 20, 30, 40] {
            p.enqueue(id);
        }
        assert!(p.move_queue_item(0, 2));
        assert_eq!(p.queue.iter().copied().collect::<Vec<_>>(), vec![20, 30, 10, 40]);

        assert!(p.move_queue_item(3, 1));
        assert_eq!(p.queue.iter().copied().collect::<Vec<_>>(), vec![20, 40, 30, 10]);

        assert!(p.move_queue_item(1, 1));
        assert_eq!(p.queue.iter().copied().collect::<Vec<_>>(), vec![20, 40, 30, 10]);

        assert!(!p.move_queue_item(4, 0));
        assert!(!p.move_queue_item(0, 4));
    }

    #[test]
    fn append_context_adds_upcoming_radio_tracks_without_interrupting_current() {
        let mut p = Player::with_seed(7);
        assert_eq!(p.play_context(ContextRef::None, "An Thần".into(), vec![100], Some(0)), Some(100));
        assert_eq!(p.peek_next(), None);

        let added = p.append_context(vec![100, 101, 102, 101, 103]);
        assert_eq!(added, 3);
        assert_eq!(p.current, Some(100));
        assert_eq!(p.peek_next(), Some(101));
        assert_eq!(p.upcoming(5), vec![101, 102, 103]);
        assert_eq!(p.advance(false), Some(101));
        assert_eq!(p.peek_next(), Some(102));
    }
}
