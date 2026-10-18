use rayon::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

/// A shared live budget. Reducing it lets in-flight curves finish and prevents
/// extra workers from taking the next batch. Nested jobs share the same pool.
pub struct Parallelism {
    maximum: usize,
    active: AtomicUsize,
    foreground: Option<rayon::ThreadPool>,
    background: Arc<rayon::ThreadPool>,
}
impl Parallelism {
    pub fn new(maximum: usize) -> Self {
        let maximum = maximum.max(1);
        static SERIAL: OnceLock<Arc<rayon::ThreadPool>> = OnceLock::new();
        Self {
            maximum,
            active: AtomicUsize::new(maximum),
            foreground: (maximum > 1).then(|| {
                rayon::ThreadPoolBuilder::new()
                    .num_threads(maximum)
                    .build()
                    .expect("foreground workers")
            }),
            background: SERIAL
                .get_or_init(|| {
                    Arc::new(
                        rayon::ThreadPoolBuilder::new()
                            .num_threads(1)
                            .build()
                            .expect("background worker"),
                    )
                })
                .clone(),
        }
    }
    pub fn set_active(&self, active: usize) {
        self.active
            .store(active.clamp(1, self.maximum), Ordering::Relaxed);
    }
    pub fn maximum(&self) -> usize {
        self.maximum
    }
    pub fn active(&self) -> usize {
        self.active.load(Ordering::Relaxed)
    }
    pub fn install<R: Send>(&self, job: impl FnOnce() -> R + Send) -> R {
        // Stay inside the current pool for nested jobs, including during focus
        // changes; hopping between pools can deadlock their waiting workers.
        if rayon::current_thread_index().is_some() {
            return job();
        }
        let pool = if self.active.load(Ordering::Relaxed) == 1 || self.foreground.is_none() {
            &self.background
        } else {
            self.foreground.as_ref().unwrap()
        };
        pool.install(job)
    }
    pub fn map<T: Sync, R: Send, F: Fn(&T) -> R + Sync>(&self, items: &[T], process: F) -> Vec<R> {
        self.map_limit(items, self.maximum, process)
    }
    pub fn map_limit<T: Sync, R: Send, F: Fn(&T) -> R + Sync>(
        &self,
        items: &[T],
        limit: usize,
        process: F,
    ) -> Vec<R> {
        let mut result = Vec::with_capacity(items.len());
        let mut offset = 0;
        while offset < items.len() {
            let budget = self.active.load(Ordering::Relaxed).min(limit.max(1));
            let end = (offset + budget).min(items.len());
            let batch = self.install(|| {
                items[offset..end]
                    .par_iter()
                    .map(&process)
                    .collect::<Vec<_>>()
            });
            result.extend(batch);
            offset = end;
        }
        result
    }
}
