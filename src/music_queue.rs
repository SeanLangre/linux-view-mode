//! Music queue with shuffle and repeat (ported from the desktop music player). Pure logic, no audio.

use crate::music_db::MusicTrack as Track;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

impl Repeat {
    pub fn cycle(self) -> Self {
        match self {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        }
    }
}

#[derive(Default)]
pub struct Queue {
    items: Vec<Track>,
    order: Vec<usize>,
    cursor: usize,
    active: bool,
    pub shuffle: bool,
    pub repeat: Repeat,
}

impl Queue {
    /// Replace the queue and start at `start` (an index into `items`).
    pub fn start(&mut self, items: Vec<Track>, start: usize) {
        self.active = !items.is_empty();
        self.items = items;
        let first = start.min(self.items.len().saturating_sub(1));
        self.build_order(Some(first));
    }

    fn build_order(&mut self, first: Option<usize>) {
        let mut order: Vec<usize> = (0..self.items.len()).collect();
        if self.shuffle {
            fastrand::shuffle(&mut order);
            if let Some(f) = first {
                if let Some(p) = order.iter().position(|&i| i == f) {
                    order.swap(0, p);
                }
            }
            self.cursor = 0;
        } else {
            self.cursor = first.unwrap_or(0);
        }
        self.order = order;
    }

    pub fn current(&self) -> Option<&Track> {
        if !self.active {
            return None;
        }
        self.order.get(self.cursor).map(|&i| &self.items[i])
    }

    pub fn set_shuffle(&mut self, on: bool) {
        if on == self.shuffle {
            return;
        }
        self.shuffle = on;
        if self.active {
            let current = self.order[self.cursor];
            self.build_order(Some(current));
        }
    }

    /// Advance. `auto` is true when the track ended by itself (repeat-one applies).
    /// Returns `None` and deactivates the queue when it runs out.
    pub fn next(&mut self, auto: bool) -> Option<&Track> {
        if !self.active {
            return None;
        }
        if auto && self.repeat == Repeat::One {
            return self.current();
        }
        if self.cursor + 1 < self.order.len() {
            self.cursor += 1;
        } else if self.repeat == Repeat::All {
            self.build_order(None);
        } else {
            self.active = false;
            return None;
        }
        self.current()
    }

    /// The track that comes after the current one, if that is known. With shuffle
    /// the order after a wrap-around isn't decided yet, so there is no answer then.
    pub fn peek_next(&self) -> Option<&Track> {
        if !self.active {
            return None;
        }
        let i = if self.cursor + 1 < self.order.len() {
            self.order[self.cursor + 1]
        } else if self.repeat == Repeat::All && !self.shuffle {
            *self.order.first()?
        } else {
            return None;
        };
        self.items.get(i)
    }

    /// The current track followed by the ones coming up, in play order.
    pub fn upcoming(&self) -> Vec<&Track> {
        if !self.active {
            return vec![];
        }
        self.order[self.cursor..].iter().map(|&i| &self.items[i]).collect()
    }

    /// Jump ahead by `offset` places in the order (0 is the current track).
    pub fn skip_forward(&mut self, offset: usize) -> Option<&Track> {
        if self.active && self.cursor + offset < self.order.len() {
            self.cursor += offset;
        }
        self.current()
    }

    /// Previous track; at the start this restarts the first track.
    pub fn prev(&mut self) -> Option<&Track> {
        if self.active && self.cursor > 0 {
            self.cursor -= 1;
        }
        self.current()
    }

    /// Adds a song at the end of the play order (it plays after everything already queued).
    /// Does nothing when no song is playing.
    pub fn append(&mut self, track: Track) {
        if !self.active {
            return;
        }
        self.items.push(track);
        self.order.push(self.items.len() - 1);
    }

    pub fn clear(&mut self) {
        self.active = false;
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(n: usize) -> Vec<Track> {
        (0..n)
            .map(|i| Track {
                video_id: i.to_string(),
                title: i.to_string(),
                artist: None,
                duration_secs: None,
                thumbnail_url: None,
                file_path: None,
                downloaded_at: None,
            })
            .collect()
    }

    fn id(q: &Queue) -> String {
        q.current().unwrap().video_id.clone()
    }

    #[test]
    fn plays_in_order_then_stops() {
        let mut q = Queue::default();
        q.start(items(3), 1);
        assert_eq!(id(&q), "1");
        assert_eq!(q.next(true).unwrap().video_id, "2");
        assert!(q.next(true).is_none());
        assert!(q.current().is_none());
    }

    #[test]
    fn repeat_all_wraps_and_repeat_one_only_on_auto() {
        let mut q = Queue::default();
        q.repeat = Repeat::All;
        q.start(items(2), 1);
        assert_eq!(q.next(true).unwrap().video_id, "0");
        q.repeat = Repeat::One;
        assert_eq!(q.next(true).unwrap().video_id, "0");
        assert_eq!(q.next(false).unwrap().video_id, "1");
    }

    #[test]
    fn peek_next_matches_what_next_would_play() {
        let mut q = Queue::default();
        q.start(items(3), 1);
        assert_eq!(q.peek_next().unwrap().video_id, "2");
        q.next(false);
        assert!(q.peek_next().is_none()); // end of the queue
        q.repeat = Repeat::All;
        assert_eq!(q.peek_next().unwrap().video_id, "0"); // wraps
        q.set_shuffle(true); // re-orders with the current song first, so there's a next one
        assert!(q.peek_next().is_some());
        q.next(false);
        q.next(false);
        assert!(q.peek_next().is_none()); // at the end of a shuffled order the next round isn't decided
        q.start(items(5), 0);
        let peeked = q.peek_next().unwrap().video_id.clone();
        assert_eq!(q.next(false).unwrap().video_id, peeked);
    }

    #[test]
    fn upcoming_lists_current_and_following() {
        let mut q = Queue::default();
        q.start(items(4), 1);
        let ids = |q: &Queue| q.upcoming().iter().map(|t| t.video_id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&q), ["1", "2", "3"]);
        assert_eq!(q.skip_forward(2).unwrap().video_id, "3");
        assert_eq!(ids(&q), ["3"]);
        assert_eq!(q.skip_forward(5).unwrap().video_id, "3"); // out of range: stays
    }

    #[test]
    fn append_plays_after_everything_queued() {
        let mut q = Queue::default();
        q.append(items(1).remove(0)); // nothing playing: ignored
        assert!(q.current().is_none());
        q.start(items(2), 0);
        let mut extra = items(1).remove(0);
        extra.video_id = "x".into();
        q.append(extra);
        let ids = |q: &Queue| q.upcoming().iter().map(|t| t.video_id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&q), ["0", "1", "x"]);
        q.set_shuffle(true); // the order is rebuilt from the items, which now include the new song
        assert_eq!(q.upcoming().len(), 3);
        assert!(ids(&q).contains(&"x".to_string()));
    }

    #[test]
    fn prev_goes_back_and_stays_at_start() {
        let mut q = Queue::default();
        q.start(items(3), 1);
        assert_eq!(q.prev().unwrap().video_id, "0");
        assert_eq!(q.prev().unwrap().video_id, "0");
    }

    #[test]
    fn shuffle_starts_on_chosen_track_and_visits_each_once() {
        let mut q = Queue::default();
        q.shuffle = true;
        q.start(items(20), 7);
        assert_eq!(id(&q), "7");
        let mut seen = vec![id(&q)];
        while let Some(t) = q.next(true) {
            seen.push(t.video_id.clone());
        }
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 20);
    }

    #[test]
    fn toggling_shuffle_keeps_current_track() {
        let mut q = Queue::default();
        q.start(items(10), 4);
        q.set_shuffle(true);
        assert_eq!(id(&q), "4");
        q.set_shuffle(false);
        assert_eq!(id(&q), "4");
        assert_eq!(q.next(true).unwrap().video_id, "5");
    }

    fn monkey_seed() -> u64 {
        std::env::var("MONKEY_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or_else(|| fastrand::u64(..))
    }

    /// Monkey test: random queue operations must never panic or leave the queue inconsistent.
    /// Set MONKEY_SEED to replay a failing run.
    #[test]
    fn monkey_test_random_operations_keep_the_queue_consistent() {
        let seed = monkey_seed();
        println!("MONKEY_SEED={seed}");
        monkey_queue(seed, 5000);
    }

    /// Many seeds. Run with `cargo test big_monkey -- --ignored --nocapture` (MONKEY_RUNS, default 2000).
    #[test]
    #[ignore]
    fn big_monkey_queue() {
        let runs: u64 = std::env::var("MONKEY_RUNS").ok().and_then(|v| v.parse().ok()).unwrap_or(2000);
        let base = monkey_seed();
        for i in 0..runs {
            let seed = base.wrapping_add(i);
            println!("MONKEY_SEED={seed}");
            monkey_queue(seed, 20_000);
        }
    }

    fn monkey_queue(seed: u64, steps: usize) {
        let mut rng = fastrand::Rng::with_seed(seed);
        let mut q = Queue::default();
        let mut log: Vec<String> = vec![];

        for step in 0..steps {
            let op = rng.u8(..11);
            log.push(format!("op {op}"));
            match op {
                0 => {
                    let n = rng.usize(..8); // includes an empty queue
                    q.start(items(n), rng.usize(..12)); // start index may be out of range
                }
                1 => q.set_shuffle(rng.bool()),
                2 => q.repeat = q.repeat.cycle(),
                3 | 4 => {
                    q.next(rng.bool());
                }
                5 => {
                    q.prev();
                }
                6 => {
                    q.skip_forward(rng.usize(..12));
                }
                7 => q.clear(),
                8 => {
                    q.peek_next();
                }
                10 => q.append(items(1).remove(0)),
                _ => {
                    q.upcoming();
                }
            }

            let ctx = || format!("seed {seed}, step {step}, recent ops {:?}", &log[log.len().saturating_sub(8)..]);
            let upcoming: Vec<String> = q.upcoming().iter().map(|t| t.video_id.clone()).collect();
            match q.current() {
                None => assert!(upcoming.is_empty(), "upcoming without a current song; {}", ctx()),
                Some(cur) => {
                    assert!(q.len() > 0, "current song in an empty queue; {}", ctx());
                    assert_eq!(upcoming.first(), Some(&cur.video_id), "upcoming must start at current; {}", ctx());
                    assert!(upcoming.len() <= q.len(), "upcoming longer than the queue; {}", ctx());
                    if let Some(next) = q.peek_next() {
                        // With repeat-all the next song may wrap to the start, otherwise it is the following one.
                        if upcoming.len() > 1 {
                            assert_eq!(upcoming[1], next.video_id, "peek_next disagrees with upcoming; {}", ctx());
                        }
                    }
                }
            }
        }
    }
}
