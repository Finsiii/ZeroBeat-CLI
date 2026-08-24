use std::{
    collections::{HashSet, VecDeque},
    time::{Duration, Instant},
};

use zerobeat_catalog::{RadioPage, RadioRequest};
use zerobeat_core::Track;

pub(crate) const MAX_VISIBLE_QUEUE: usize = 12;
const PREFETCH_INVENTORY: usize = MAX_VISIBLE_QUEUE * 2;
const MAX_NO_PROGRESS_PAGES: u8 = 3;
const MAX_RETRY_ATTEMPTS: u32 = 6;
const RETRY_BASE: Duration = Duration::from_millis(250);
const RETRY_MAX: Duration = Duration::from_secs(8);

#[derive(Default)]
pub(crate) struct EndlessQueue {
    enabled: bool,
    seed_track_id: String,
    continuation: Option<String>,
    pending: VecDeque<Track>,
    seen: HashSet<String>,
    fetching: bool,
    exhausted: bool,
    generation: u64,
    retry_after: Option<Instant>,
    retry_attempt: u32,
    no_progress_pages: u8,
}

impl EndlessQueue {
    pub(crate) fn start(
        &mut self,
        seed: &Track,
        visible: &[Track],
        overflow: impl IntoIterator<Item = Track>,
    ) {
        self.enabled = true;
        self.seed_track_id.clone_from(&seed.id);
        self.continuation = None;
        self.pending.clear();
        self.seen.clear();
        self.seen.insert(seed.id.clone());
        self.seen
            .extend(visible.iter().map(|track| track.id.clone()));
        self.fetching = false;
        self.exhausted = false;
        self.retry_after = None;
        self.retry_attempt = 0;
        self.no_progress_pages = 0;
        self.generation = self.generation.wrapping_add(1);
        for track in overflow {
            if self.seen.insert(track.id.clone()) {
                self.pending.push_back(track);
            }
        }
    }

    pub(crate) fn stop(&mut self) {
        self.enabled = false;
        self.pending.clear();
        self.continuation = None;
        self.fetching = false;
        self.retry_after = None;
        self.retry_attempt = 0;
        self.no_progress_pages = 0;
        self.generation = self.generation.wrapping_add(1);
    }

    pub(crate) fn reserve(&mut self, track_id: &str) -> bool {
        if !self.enabled {
            return true;
        }
        self.seen.insert(track_id.to_owned())
    }

    pub(crate) fn top_up(&mut self, queue: &mut Vec<Track>) {
        while queue.len() < MAX_VISIBLE_QUEUE {
            let Some(track) = self.pending.pop_front() else {
                break;
            };
            if !queue.iter().any(|queued| queued.id == track.id) {
                queue.push(track);
            }
        }
    }

    pub(crate) fn stash_overflow(&mut self, queue: &mut Vec<Track>) {
        if queue.len() <= MAX_VISIBLE_QUEUE {
            return;
        }
        let overflow = queue.split_off(MAX_VISIBLE_QUEUE);
        for track in overflow.into_iter().rev() {
            self.pending.push_front(track);
        }
    }

    pub(crate) fn request(&mut self, visible_count: usize) -> Option<(u64, RadioRequest)> {
        if !self.enabled
            || self.fetching
            || self.exhausted
            || visible_count.saturating_add(self.pending.len()) >= PREFETCH_INVENTORY
        {
            return None;
        }
        if let Some(retry_after) = self.retry_after {
            if retry_after > Instant::now() {
                return None;
            }
            self.retry_after = None;
        }
        self.fetching = true;
        let request = match self.continuation.clone() {
            Some(continuation) => RadioRequest::from_continuation(
                self.seed_track_id.clone(),
                continuation,
                PREFETCH_INVENTORY,
            ),
            None => RadioRequest::from_seed(self.seed_track_id.clone(), PREFETCH_INVENTORY),
        };
        Some((self.generation, request))
    }

    pub(crate) fn accept(&mut self, generation: u64, page: RadioPage) -> bool {
        if generation != self.generation || !self.enabled {
            return false;
        }
        self.fetching = false;
        let previous_continuation = self.continuation.clone();
        let continuation = page.continuation.filter(|value| !value.trim().is_empty());
        let token_did_not_advance = previous_continuation
            .as_ref()
            .is_some_and(|previous| continuation.as_ref() == Some(previous));
        self.continuation = continuation.clone();
        let mut added = false;
        for track in page.tracks {
            if self.seen.insert(track.id.clone()) {
                self.seed_track_id.clone_from(&track.id);
                self.pending.push_back(track);
                added = true;
            }
        }
        if added {
            self.no_progress_pages = 0;
            self.retry_attempt = 0;
            self.retry_after = None;
            if token_did_not_advance {
                self.exhausted = true;
                self.continuation = None;
            }
        } else if self.continuation.is_none() || token_did_not_advance {
            self.exhausted = true;
            self.retry_after = None;
        } else {
            self.no_progress_pages = self.no_progress_pages.saturating_add(1);
            if self.no_progress_pages >= MAX_NO_PROGRESS_PAGES {
                self.exhausted = true;
                self.retry_after = None;
            } else {
                self.schedule_retry(u32::from(self.no_progress_pages));
            }
        }
        true
    }

    pub(crate) fn reject(&mut self, generation: u64) {
        if generation == self.generation && self.enabled {
            self.fetching = false;
            self.retry_attempt = self.retry_attempt.saturating_add(1);
            self.schedule_retry(self.retry_attempt);
        }
    }

    pub(crate) fn retry_delay(&self, visible_count: usize) -> Option<Duration> {
        if !self.enabled
            || self.exhausted
            || self.fetching
            || visible_count.saturating_add(self.pending.len()) >= PREFETCH_INVENTORY
        {
            return None;
        }
        self.retry_after
            .map(|retry_after| retry_after.saturating_duration_since(Instant::now()))
    }

    fn schedule_retry(&mut self, attempt: u32) {
        self.retry_after = Some(Instant::now() + retry_backoff(attempt));
    }
}

fn retry_backoff(attempt: u32) -> Duration {
    let exponent = attempt.saturating_sub(1).min(MAX_RETRY_ATTEMPTS - 1);
    let multiplier = 1_u32 << exponent;
    RETRY_BASE.saturating_mul(multiplier).min(RETRY_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str) -> Track {
        Track::new(id, id, "ZeroBeat", 180_000)
    }

    #[test]
    fn no_progress_with_a_new_token_is_delayed_then_same_token_exhausts() {
        let seed = track("seed");
        let mut queue = EndlessQueue::default();
        queue.start(&seed, &[], std::iter::empty());
        let (generation, _) = queue.request(0).expect("initial request");

        assert!(queue.accept(
            generation,
            RadioPage {
                tracks: Vec::new(),
                continuation: Some("next".into()),
            },
        ));
        assert!(queue.retry_delay(0).is_some());
        assert!(queue.request(0).is_none());

        assert!(queue.accept(
            generation,
            RadioPage {
                tracks: Vec::new(),
                continuation: Some("next".into()),
            },
        ));
        assert!(queue.retry_delay(0).is_none());
        assert!(queue.request(0).is_none());
    }

    #[test]
    fn reservation_rejects_seen_and_pending_tracks() {
        let seed = track("seed");
        let pending = track("pending");
        let mut queue = EndlessQueue::default();
        queue.start(&seed, &[], [pending.clone()]);
        assert!(!queue.reserve("pending"));
        assert!(!queue.reserve("seed"));
        assert!(queue.reserve("manual"));
        assert!(!queue.reserve("manual"));
    }

    #[test]
    fn a_page_without_continuation_rolls_over_from_the_last_unique_track() {
        let seed = track("seed");
        let mut queue = EndlessQueue::default();
        queue.start(&seed, &[], std::iter::empty());
        let (generation, _) = queue.request(0).expect("initial request");

        assert!(queue.accept(
            generation,
            RadioPage {
                tracks: vec![track("last")],
                continuation: None,
            },
        ));
        let (_, request) = queue.request(0).expect("seed rollover request");
        assert_eq!(request.seed_track_id, "last");
        assert_eq!(request.continuation, None);
    }
}
