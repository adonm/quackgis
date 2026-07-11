// SPDX-License-Identifier: Apache-2.0
//! Process-local operation admission with bounded queueing.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::engine_api::{EngineCancellation, EngineResult};

type QueryKey = (i32, Vec<u8>);
type CancellationMap = HashMap<QueryKey, Arc<dyn EngineCancellation>>;

static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static QUEUED: AtomicUsize = AtomicUsize::new(0);
static STARTED: AtomicU64 = AtomicU64::new(0);
static REJECTED: AtomicU64 = AtomicU64::new(0);
static QUEUE_TIMEOUTS: AtomicU64 = AtomicU64::new(0);
static READER_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static READER_QUEUED: AtomicUsize = AtomicUsize::new(0);
static READER_HIGH_WATER: AtomicUsize = AtomicUsize::new(0);
static WRITER_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static WRITER_QUEUED: AtomicUsize = AtomicUsize::new(0);
static WRITER_HIGH_WATER: AtomicUsize = AtomicUsize::new(0);
static MAINTENANCE_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static MAINTENANCE_QUEUED: AtomicUsize = AtomicUsize::new(0);
static MAINTENANCE_HIGH_WATER: AtomicUsize = AtomicUsize::new(0);
static CANCEL_REQUESTED: AtomicU64 = AtomicU64::new(0);
static CANCEL_COMPLETED: AtomicU64 = AtomicU64::new(0);
static CANCEL_FAILED: AtomicU64 = AtomicU64::new(0);
static STATEMENT_TIMEOUTS: AtomicU64 = AtomicU64::new(0);
static BLOCKING_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static BLOCKING_REGULAR_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static BLOCKING_CONTROL_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static BLOCKING_QUEUED: AtomicUsize = AtomicUsize::new(0);
static BLOCKING_HIGH_WATER: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug)]
pub struct AdmissionController {
    active: Arc<Semaphore>,
    readers: Arc<Semaphore>,
    writers: Arc<Semaphore>,
    maintenance: Arc<Semaphore>,
    queue_slots: Arc<Semaphore>,
    queue_timeout: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationClass {
    Reader,
    Writer,
    Maintenance,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionError {
    QueueFull,
    QueueTimeout,
    Closed,
}

pub struct AdmissionPermit {
    _global: OwnedSemaphorePermit,
    _class: OwnedSemaphorePermit,
    class: OperationClass,
}

struct AdmissionQueueGuard {
    class: OperationClass,
}

impl AdmissionQueueGuard {
    fn new(class: OperationClass) -> Self {
        QUEUED.fetch_add(1, Ordering::Relaxed);
        class_queued(class).fetch_add(1, Ordering::Relaxed);
        Self { class }
    }
}

impl Drop for AdmissionQueueGuard {
    fn drop(&mut self) {
        QUEUED.fetch_sub(1, Ordering::Relaxed);
        class_queued(self.class).fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Clone, Debug)]
pub struct BlockingWorkerPool {
    total: Arc<Semaphore>,
    regular: Arc<Semaphore>,
}

#[derive(Debug)]
pub enum BlockingWorkerError {
    Closed,
    Join(tokio::task::JoinError),
}

struct BlockingQueueGuard;

struct BlockingWorkerPermit {
    _total: OwnedSemaphorePermit,
    _regular: Option<OwnedSemaphorePermit>,
    control: bool,
}

impl BlockingQueueGuard {
    fn new() -> Self {
        BLOCKING_QUEUED.fetch_add(1, Ordering::Relaxed);
        Self
    }
}

impl Drop for BlockingQueueGuard {
    fn drop(&mut self) {
        BLOCKING_QUEUED.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Drop for BlockingWorkerPermit {
    fn drop(&mut self) {
        BLOCKING_ACTIVE.fetch_sub(1, Ordering::Relaxed);
        if self.control {
            BLOCKING_CONTROL_ACTIVE.fetch_sub(1, Ordering::Relaxed);
        } else {
            BLOCKING_REGULAR_ACTIVE.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

impl BlockingWorkerPool {
    pub fn new(max_workers: usize) -> Self {
        assert!(
            max_workers >= 2,
            "max_workers must reserve one control slot"
        );
        Self {
            total: Arc::new(Semaphore::new(max_workers)),
            regular: Arc::new(Semaphore::new(max_workers - 1)),
        }
    }

    async fn acquire(&self, control: bool) -> Result<BlockingWorkerPermit, BlockingWorkerError> {
        let queued = BlockingQueueGuard::new();
        let regular = if control {
            None
        } else {
            Some(
                Arc::clone(&self.regular)
                    .acquire_owned()
                    .await
                    .map_err(|_| BlockingWorkerError::Closed)?,
            )
        };
        let total = Arc::clone(&self.total)
            .acquire_owned()
            .await
            .map_err(|_| BlockingWorkerError::Closed)?;
        drop(queued);
        let active = BLOCKING_ACTIVE.fetch_add(1, Ordering::Relaxed) + 1;
        let mut observed = BLOCKING_HIGH_WATER.load(Ordering::Relaxed);
        while active > observed {
            match BLOCKING_HIGH_WATER.compare_exchange_weak(
                observed,
                active,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => observed = actual,
            }
        }
        if control {
            BLOCKING_CONTROL_ACTIVE.fetch_add(1, Ordering::Relaxed);
        } else {
            BLOCKING_REGULAR_ACTIVE.fetch_add(1, Ordering::Relaxed);
        }
        Ok(BlockingWorkerPermit {
            _total: total,
            _regular: regular,
            control,
        })
    }

    pub async fn spawn_regular<F, T>(
        &self,
        operation: F,
    ) -> Result<tokio::task::JoinHandle<T>, BlockingWorkerError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let permit = self.acquire(false).await?;
        Ok(tokio::task::spawn_blocking(move || {
            let _permit = permit;
            operation()
        }))
    }

    pub async fn run_regular<F, T>(&self, operation: F) -> Result<T, BlockingWorkerError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        self.spawn_regular(operation)
            .await?
            .await
            .map_err(BlockingWorkerError::Join)
    }

    pub async fn run_control<F, T>(&self, operation: F) -> Result<T, BlockingWorkerError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let permit = self.acquire(true).await?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            operation()
        })
        .await
        .map_err(BlockingWorkerError::Join)
    }
}

#[derive(Default)]
pub struct ActiveQueryRegistry {
    entries: Mutex<CancellationMap>,
}

pub struct ActiveQueryGuard {
    registry: Arc<ActiveQueryRegistry>,
    key: QueryKey,
}

pub struct OperationDeadline {
    task: tokio::task::JoinHandle<()>,
}

impl OperationDeadline {
    pub fn start(duration: Duration, cancellation: Arc<dyn EngineCancellation>) -> Self {
        let task = tokio::spawn(async move {
            tokio::time::sleep(duration).await;
            STATEMENT_TIMEOUTS.fetch_add(1, Ordering::Relaxed);
            let _ = cancellation.cancel();
        });
        Self { task }
    }
}

impl Drop for OperationDeadline {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Drop for ActiveQueryGuard {
    fn drop(&mut self) {
        if let Ok(mut entries) = self.registry.entries.lock() {
            entries.remove(&self.key);
        }
    }
}

impl ActiveQueryRegistry {
    pub fn register(
        self: &Arc<Self>,
        pid: i32,
        secret: Vec<u8>,
        cancellation: Arc<dyn EngineCancellation>,
    ) -> ActiveQueryGuard {
        let key = (pid, secret);
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(key.clone(), cancellation);
        }
        ActiveQueryGuard {
            registry: Arc::clone(self),
            key,
        }
    }

    pub fn cancel(&self, pid: i32, secret: &[u8]) -> EngineResult<bool> {
        CANCEL_REQUESTED.fetch_add(1, Ordering::Relaxed);
        let cancellation = self
            .entries
            .lock()
            .ok()
            .and_then(|entries| entries.get(&(pid, secret.to_vec())).cloned());
        let Some(cancellation) = cancellation else {
            return Ok(false);
        };
        match cancellation.cancel() {
            Ok(()) => {
                CANCEL_COMPLETED.fetch_add(1, Ordering::Relaxed);
                Ok(true)
            }
            Err(error) => {
                CANCEL_FAILED.fetch_add(1, Ordering::Relaxed);
                Err(error)
            }
        }
    }
}

impl Drop for AdmissionPermit {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::Relaxed);
        class_active(self.class).fetch_sub(1, Ordering::Relaxed);
    }
}

impl AdmissionController {
    pub fn new(
        max_active: usize,
        max_queued: usize,
        max_readers: usize,
        max_writers: usize,
        max_maintenance: usize,
        queue_timeout: Duration,
    ) -> Self {
        assert!(max_active > 0, "max_active must be positive");
        assert!(max_queued > 0, "max_queued must be positive");
        assert!(max_readers > 0, "max_readers must be positive");
        assert!(max_writers > 0, "max_writers must be positive");
        assert!(max_maintenance > 0, "max_maintenance must be positive");
        Self {
            active: Arc::new(Semaphore::new(max_active)),
            readers: Arc::new(Semaphore::new(max_readers)),
            writers: Arc::new(Semaphore::new(max_writers)),
            maintenance: Arc::new(Semaphore::new(max_maintenance)),
            queue_slots: Arc::new(Semaphore::new(max_queued)),
            queue_timeout,
        }
    }

    pub async fn acquire(&self, class: OperationClass) -> Result<AdmissionPermit, AdmissionError> {
        let queue_slot = Arc::clone(&self.queue_slots)
            .try_acquire_owned()
            .map_err(|_| {
                REJECTED.fetch_add(1, Ordering::Relaxed);
                AdmissionError::QueueFull
            })?;
        let queued = AdmissionQueueGuard::new(class);
        let class_limit = match class {
            OperationClass::Reader => Arc::clone(&self.readers),
            OperationClass::Writer => Arc::clone(&self.writers),
            OperationClass::Maintenance => Arc::clone(&self.maintenance),
        };
        let global_limit = Arc::clone(&self.active);
        let active = tokio::time::timeout(self.queue_timeout, async move {
            let class = class_limit
                .acquire_owned()
                .await
                .map_err(|_| AdmissionError::Closed)?;
            let global = global_limit
                .acquire_owned()
                .await
                .map_err(|_| AdmissionError::Closed)?;
            Ok::<_, AdmissionError>((global, class))
        })
        .await;
        drop(queued);
        drop(queue_slot);
        let (global, class_permit) = match active {
            Ok(Ok(permits)) => permits,
            Ok(Err(error)) => return Err(error),
            Err(_) => {
                QUEUE_TIMEOUTS.fetch_add(1, Ordering::Relaxed);
                return Err(AdmissionError::QueueTimeout);
            }
        };
        ACTIVE.fetch_add(1, Ordering::Relaxed);
        let class_count = class_active(class).fetch_add(1, Ordering::Relaxed) + 1;
        update_high_water(class_high_water(class), class_count);
        STARTED.fetch_add(1, Ordering::Relaxed);
        Ok(AdmissionPermit {
            _global: global,
            _class: class_permit,
            class,
        })
    }
}

fn class_active(class: OperationClass) -> &'static AtomicUsize {
    match class {
        OperationClass::Reader => &READER_ACTIVE,
        OperationClass::Writer => &WRITER_ACTIVE,
        OperationClass::Maintenance => &MAINTENANCE_ACTIVE,
    }
}

fn class_queued(class: OperationClass) -> &'static AtomicUsize {
    match class {
        OperationClass::Reader => &READER_QUEUED,
        OperationClass::Writer => &WRITER_QUEUED,
        OperationClass::Maintenance => &MAINTENANCE_QUEUED,
    }
}

fn class_high_water(class: OperationClass) -> &'static AtomicUsize {
    match class {
        OperationClass::Reader => &READER_HIGH_WATER,
        OperationClass::Writer => &WRITER_HIGH_WATER,
        OperationClass::Maintenance => &MAINTENANCE_HIGH_WATER,
    }
}

fn update_high_water(target: &AtomicUsize, value: usize) {
    let mut observed = target.load(Ordering::Relaxed);
    while value > observed {
        match target.compare_exchange_weak(observed, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(actual) => observed = actual,
        }
    }
}

pub fn active_operations() -> usize {
    ACTIVE.load(Ordering::Relaxed)
}

pub fn queued_operations() -> usize {
    QUEUED.load(Ordering::Relaxed)
}

pub fn active_operations_for(class: OperationClass) -> usize {
    class_active(class).load(Ordering::Relaxed)
}

pub fn queued_operations_for(class: OperationClass) -> usize {
    class_queued(class).load(Ordering::Relaxed)
}

pub fn operations_high_water_for(class: OperationClass) -> usize {
    class_high_water(class).load(Ordering::Relaxed)
}

pub fn started_total() -> u64 {
    STARTED.load(Ordering::Relaxed)
}

pub fn rejected_total() -> u64 {
    REJECTED.load(Ordering::Relaxed)
}

pub fn queue_timeouts_total() -> u64 {
    QUEUE_TIMEOUTS.load(Ordering::Relaxed)
}

pub fn cancellations_requested_total() -> u64 {
    CANCEL_REQUESTED.load(Ordering::Relaxed)
}

pub fn cancellations_completed_total() -> u64 {
    CANCEL_COMPLETED.load(Ordering::Relaxed)
}

pub fn cancellations_failed_total() -> u64 {
    CANCEL_FAILED.load(Ordering::Relaxed)
}

pub fn statement_timeouts_total() -> u64 {
    STATEMENT_TIMEOUTS.load(Ordering::Relaxed)
}

pub fn blocking_workers_active() -> usize {
    BLOCKING_ACTIVE.load(Ordering::Relaxed)
}

pub fn blocking_regular_active() -> usize {
    BLOCKING_REGULAR_ACTIVE.load(Ordering::Relaxed)
}

pub fn blocking_control_active() -> usize {
    BLOCKING_CONTROL_ACTIVE.load(Ordering::Relaxed)
}

pub fn blocking_workers_queued() -> usize {
    BLOCKING_QUEUED.load(Ordering::Relaxed)
}

pub fn blocking_workers_high_water() -> usize {
    BLOCKING_HIGH_WATER.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bounds_active_and_queue_slots() {
        let controller = AdmissionController::new(1, 1, 1, 1, 1, Duration::from_millis(20));
        let active = controller
            .acquire(OperationClass::Reader)
            .await
            .expect("active permit");
        assert_eq!(controller.active.available_permits(), 0);
        let queued_controller = controller.clone();
        let queued =
            tokio::spawn(async move { queued_controller.acquire(OperationClass::Reader).await });
        tokio::task::yield_now().await;
        assert_eq!(
            controller.acquire(OperationClass::Reader).await.err(),
            Some(AdmissionError::QueueFull)
        );
        assert_eq!(
            queued.await.expect("queue task").err(),
            Some(AdmissionError::QueueTimeout)
        );
        drop(active);
        assert_eq!(controller.active.available_permits(), 1);
    }

    #[tokio::test]
    async fn thirty_two_contenders_never_exceed_eight_active() {
        let controller = AdmissionController::new(8, 32, 8, 2, 1, Duration::from_secs(2));
        let active = Arc::new(AtomicUsize::new(0));
        let high_water = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(tokio::sync::Barrier::new(33));
        let mut tasks = Vec::new();
        for _ in 0..32 {
            let controller = controller.clone();
            let active = Arc::clone(&active);
            let high_water = Arc::clone(&high_water);
            let barrier = Arc::clone(&barrier);
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                let _permit = controller
                    .acquire(OperationClass::Reader)
                    .await
                    .expect("reader admission");
                let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                update_high_water(&high_water, count);
                tokio::time::sleep(Duration::from_millis(5)).await;
                active.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        barrier.wait().await;
        for task in tasks {
            task.await.expect("contender task");
        }
        assert_eq!(high_water.load(Ordering::SeqCst), 8);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn writer_and_maintenance_limits_are_independent() {
        let controller = AdmissionController::new(4, 4, 4, 1, 1, Duration::from_millis(20));
        let writer = controller
            .acquire(OperationClass::Writer)
            .await
            .expect("writer permit");
        let maintenance = controller
            .acquire(OperationClass::Maintenance)
            .await
            .expect("maintenance permit");
        let reader = controller
            .acquire(OperationClass::Reader)
            .await
            .expect("reader permit");
        assert_eq!(controller.writers.available_permits(), 0);
        assert_eq!(controller.maintenance.available_permits(), 0);
        assert_eq!(controller.readers.available_permits(), 3);
        assert_eq!(controller.active.available_permits(), 1);
        assert_eq!(
            controller.acquire(OperationClass::Writer).await.err(),
            Some(AdmissionError::QueueTimeout)
        );
        drop((writer, maintenance, reader));
    }

    #[tokio::test]
    async fn reserves_control_capacity_from_regular_workers() {
        let pool = BlockingWorkerPool::new(2);
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let regular = pool
            .spawn_regular(move || release_rx.recv().expect("regular release"))
            .await
            .expect("regular worker");
        let control = tokio::time::timeout(
            Duration::from_secs(1),
            pool.run_control(|| "control-completed"),
        )
        .await
        .expect("reserved control slot")
        .expect("control worker");
        assert_eq!(control, "control-completed");
        release_tx.send(()).expect("release regular worker");
        regular.await.expect("join regular worker");
    }
}
