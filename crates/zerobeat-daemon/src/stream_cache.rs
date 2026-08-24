use std::{
    collections::HashMap,
    sync::{Arc, Mutex as StdMutex, Weak},
    time::{SystemTime, UNIX_EPOCH},
};

use tokio::sync::{Mutex, watch};
use zerobeat_catalog::{AudioQuality, CatalogError, MusicCatalog, ResolvedStream};

const MAX_ENTRIES: usize = 32;
const SAFE_MARGIN_SECONDS: u64 = 15;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Key {
    track_id: String,
    quality: AudioQuality,
}

struct Entry {
    stream: ResolvedStream,
    last_used: u64,
}

struct Flight {
    result: watch::Sender<Option<Result<ResolvedStream, String>>>,
    key: Key,
}

struct OwnerGuard {
    cache: Weak<StreamCache>,
    flight: Arc<Flight>,
    armed: bool,
}

impl OwnerGuard {
    fn new(cache: &Arc<StreamCache>, flight: Arc<Flight>) -> Self {
        Self {
            cache: Arc::downgrade(cache),
            flight,
            armed: true,
        }
    }

    fn complete(mut self, result: Result<ResolvedStream, String>) {
        let _ = self.flight.result.send(Some(result));
        self.remove();
        self.armed = false;
    }

    fn remove(&self) {
        let Some(cache) = self.cache.upgrade() else {
            return;
        };
        let Ok(mut flights) = cache.flights.lock() else {
            return;
        };
        if flights
            .get(&self.flight.key)
            .is_some_and(|current| Arc::ptr_eq(current, &self.flight))
        {
            flights.remove(&self.flight.key);
        }
    }
}

impl Drop for OwnerGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let _ = self
            .flight
            .result
            .send(Some(Err("stream resolver aborted".into())));
        self.remove();
    }
}

pub(crate) struct StreamCache {
    entries: Mutex<HashMap<Key, Entry>>,
    flights: StdMutex<HashMap<Key, Arc<Flight>>>,
    clock: Mutex<u64>,
}

impl StreamCache {
    pub(crate) fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            flights: StdMutex::new(HashMap::new()),
            clock: Mutex::new(0),
        }
    }

    async fn get(&self, key: &Key) -> Option<ResolvedStream> {
        let now = unix_seconds();
        let mut entries = self.entries.lock().await;
        let expired = entries.get(key).is_some_and(|entry| {
            entry
                .stream
                .expires_at_epoch_seconds
                .is_some_and(|expiry| now.saturating_add(SAFE_MARGIN_SECONDS) >= expiry)
        });
        if expired {
            entries.remove(key);
            return None;
        }
        let entry = entries.get_mut(key)?;
        let mut clock = self.clock.lock().await;
        *clock = clock.saturating_add(1);
        entry.last_used = *clock;
        Some(entry.stream.clone())
    }

    async fn insert(&self, key: Key, stream: ResolvedStream) {
        let mut entries = self.entries.lock().await;
        let mut clock = self.clock.lock().await;
        *clock = clock.saturating_add(1);
        entries.insert(
            key,
            Entry {
                stream,
                last_used: *clock,
            },
        );
        while entries.len() > MAX_ENTRIES {
            let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            entries.remove(&oldest);
        }
    }

    pub(crate) async fn resolve(
        self: &Arc<Self>,
        catalog: &dyn MusicCatalog,
        track_id: &str,
        quality: AudioQuality,
    ) -> Result<ResolvedStream, CatalogError> {
        let key = Key {
            track_id: track_id.to_owned(),
            quality,
        };
        if let Some(stream) = self.get(&key).await {
            return Ok(stream);
        }
        let (flight, owner) = {
            let mut flights = self.flights.lock().expect("stream flights mutex poisoned");
            if let Some(flight) = flights.get(&key).cloned() {
                (flight, false)
            } else {
                let (sender, _) = watch::channel(None);
                let flight = Arc::new(Flight {
                    result: sender,
                    key: key.clone(),
                });
                flights.insert(key.clone(), Arc::clone(&flight));
                (flight, true)
            }
        };
        if !owner {
            let mut receiver = flight.result.subscribe();
            loop {
                if let Some(result) = receiver.borrow().clone() {
                    return result.map_err(CatalogError::Unavailable);
                }
                if receiver.changed().await.is_err() {
                    return Err(CatalogError::Unavailable("stream resolver aborted".into()));
                }
            }
        }
        let owner_guard = OwnerGuard::new(self, Arc::clone(&flight));
        let result = catalog.resolve_stream(track_id, quality).await;
        if let Ok(stream) = &result {
            self.insert(key.clone(), stream.clone()).await;
        }
        let output = result
            .as_ref()
            .map(Clone::clone)
            .map_err(|error| CatalogError::Unavailable(error.to_string()));
        let stored = result
            .as_ref()
            .map(Clone::clone)
            .map_err(ToString::to_string);
        owner_guard.complete(stored);
        output
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use zerobeat_catalog::{CatalogFuture, Lyrics, SearchRequest};
    use zerobeat_core::Track;

    struct CountingCatalog {
        calls: Arc<AtomicUsize>,
        delay: std::time::Duration,
        error: bool,
    }

    impl MusicCatalog for CountingCatalog {
        fn search_songs(&self, _request: SearchRequest) -> CatalogFuture<'_, Vec<Track>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn resolve_stream(
            &self,
            track_id: &str,
            _quality: AudioQuality,
        ) -> CatalogFuture<'_, ResolvedStream> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let delay = self.delay;
            let error = self.error;
            let track_id = track_id.to_owned();
            Box::pin(async move {
                tokio::time::sleep(delay).await;
                if error {
                    return Err(CatalogError::Unavailable("resolver failed".into()));
                }
                Ok(ResolvedStream::new(format!("https://stream/{track_id}")))
            })
        }

        fn lyrics(&self, _track: &Track) -> CatalogFuture<'_, Option<Lyrics>> {
            Box::pin(async { Ok(None) })
        }
    }

    #[tokio::test]
    async fn cache_is_bounded_and_deduplicates_singleflight() {
        let cache = Arc::new(StreamCache::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let catalog = CountingCatalog {
            calls: Arc::clone(&calls),
            delay: std::time::Duration::from_millis(10),
            error: false,
        };
        let (first, second) = tokio::join!(
            cache.resolve(&catalog, "same", AudioQuality::Automatic),
            cache.resolve(&catalog, "same", AudioQuality::Automatic),
        );
        assert!(first.is_ok());
        assert!(second.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        for index in 0..40 {
            cache
                .resolve(&catalog, &format!("track-{index}"), AudioQuality::Automatic)
                .await
                .unwrap();
        }
        assert_eq!(cache.entries.lock().await.len(), MAX_ENTRIES);
    }

    #[tokio::test]
    async fn completed_flight_is_replayed_to_late_waiters() {
        let cache = Arc::new(StreamCache::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let catalog = CountingCatalog {
            calls: Arc::clone(&calls),
            delay: std::time::Duration::ZERO,
            error: false,
        };
        let first = cache
            .resolve(&catalog, "instant", AudioQuality::Automatic)
            .await
            .unwrap();
        let second = cache
            .resolve(&catalog, "instant", AudioQuality::Automatic)
            .await
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(cache.flights.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn concurrent_waiters_receive_the_same_error() {
        let cache = Arc::new(StreamCache::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let catalog = CountingCatalog {
            calls: Arc::clone(&calls),
            delay: std::time::Duration::ZERO,
            error: true,
        };
        let (first, second, third, fourth) = tokio::join!(
            cache.resolve(&catalog, "broken", AudioQuality::Automatic),
            cache.resolve(&catalog, "broken", AudioQuality::Automatic),
            cache.resolve(&catalog, "broken", AudioQuality::Automatic),
            cache.resolve(&catalog, "broken", AudioQuality::Automatic),
        );
        for result in [first, second, third, fourth] {
            assert_eq!(
                result.unwrap_err().to_string(),
                "catalog is unavailable: catalog is unavailable: resolver failed"
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(cache.flights.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn aborted_owner_does_not_poison_the_key_for_the_next_resolver() {
        let cache = Arc::new(StreamCache::new());
        let catalog = Arc::new(CountingCatalog {
            calls: Arc::new(AtomicUsize::new(0)),
            delay: std::time::Duration::from_millis(100),
            error: false,
        });
        let owner_cache = Arc::clone(&cache);
        let owner_catalog = Arc::clone(&catalog);
        let owner = tokio::spawn(async move {
            owner_cache
                .resolve(owner_catalog.as_ref(), "aborted", AudioQuality::Automatic)
                .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while catalog.calls.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let waiter_cache = Arc::clone(&cache);
        let waiter_catalog = Arc::clone(&catalog);
        let waiter = tokio::spawn(async move {
            waiter_cache
                .resolve(waiter_catalog.as_ref(), "aborted", AudioQuality::Automatic)
                .await
        });
        tokio::task::yield_now().await;
        owner.abort();
        let _ = owner.await;
        let waiter_result = tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .expect("waiter must be released when owner aborts")
            .expect("waiter task must finish");
        assert!(waiter_result.is_err());
        assert!(cache.flights.lock().unwrap().is_empty());
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            cache.resolve(catalog.as_ref(), "aborted", AudioQuality::Automatic),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result.url, "https://stream/aborted");
        assert!(cache.flights.lock().unwrap().is_empty());
    }
}
