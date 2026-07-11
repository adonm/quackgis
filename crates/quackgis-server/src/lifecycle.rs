// SPDX-License-Identifier: Apache-2.0
//! Process-local readiness and transaction-drain state.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

use tokio::sync::Notify;

#[derive(Debug)]
pub struct RuntimeLifecycle {
    accepting: AtomicBool,
    readiness: AtomicU8,
    active_transactions: AtomicUsize,
    transactions_drained: Notify,
}

impl Default for RuntimeLifecycle {
    fn default() -> Self {
        Self {
            accepting: AtomicBool::new(true),
            readiness: AtomicU8::new(ReadinessState::Starting as u8),
            active_transactions: AtomicUsize::new(0),
            transactions_drained: Notify::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ReadinessState {
    Starting,
    Ready,
    StorageUnavailable,
    Draining,
}

impl RuntimeLifecycle {
    pub fn is_accepting(&self) -> bool {
        self.accepting.load(Ordering::Acquire)
    }

    pub fn active_transactions(&self) -> usize {
        self.active_transactions.load(Ordering::Acquire)
    }

    pub fn readiness(&self) -> ReadinessState {
        match self.readiness.load(Ordering::Acquire) {
            value if value == ReadinessState::Ready as u8 => ReadinessState::Ready,
            value if value == ReadinessState::StorageUnavailable as u8 => {
                ReadinessState::StorageUnavailable
            }
            value if value == ReadinessState::Draining as u8 => ReadinessState::Draining,
            _ => ReadinessState::Starting,
        }
    }

    pub fn mark_storage_ready(&self) {
        self.update_readiness(ReadinessState::Ready);
    }

    pub fn mark_storage_unavailable(&self) {
        self.update_readiness(ReadinessState::StorageUnavailable);
    }

    fn update_readiness(&self, next: ReadinessState) {
        let mut current = self.readiness.load(Ordering::Acquire);
        loop {
            if current == ReadinessState::Draining as u8 {
                return;
            }
            match self.readiness.compare_exchange_weak(
                current,
                next as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(actual) => current = actual,
            }
        }
    }

    pub fn begin_drain(&self) {
        self.accepting.store(false, Ordering::Release);
        self.readiness
            .store(ReadinessState::Draining as u8, Ordering::Release);
        if self.active_transactions() == 0 {
            self.transactions_drained.notify_waiters();
        }
    }

    /// Atomically join the drain set while startup is still accepted. A drain
    /// racing this call either observes the new transaction or makes this fail.
    pub fn try_start_transaction(&self) -> bool {
        if !self.is_accepting() {
            return false;
        }
        self.active_transactions.fetch_add(1, Ordering::AcqRel);
        if self.is_accepting() {
            true
        } else {
            self.transaction_finished();
            false
        }
    }

    pub fn transaction_finished(&self) {
        let previous = self.active_transactions.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "transaction accounting underflow");
        if previous == 1 {
            self.transactions_drained.notify_waiters();
        }
    }

    pub async fn wait_for_transactions(&self) {
        while self.active_transactions() != 0 {
            self.transactions_drained.notified().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn drain_rejects_new_transactions_and_waits_for_active_ones() {
        let lifecycle = RuntimeLifecycle::default();
        assert_eq!(lifecycle.readiness(), ReadinessState::Starting);
        lifecycle.mark_storage_ready();
        assert_eq!(lifecycle.readiness(), ReadinessState::Ready);
        lifecycle.mark_storage_unavailable();
        assert_eq!(lifecycle.readiness(), ReadinessState::StorageUnavailable);
        lifecycle.mark_storage_ready();
        assert!(lifecycle.try_start_transaction());
        lifecycle.begin_drain();
        assert!(!lifecycle.is_accepting());
        assert_eq!(lifecycle.readiness(), ReadinessState::Draining);
        lifecycle.mark_storage_ready();
        assert_eq!(lifecycle.readiness(), ReadinessState::Draining);
        assert!(!lifecycle.try_start_transaction());
        assert_eq!(lifecycle.active_transactions(), 1);
        lifecycle.transaction_finished();
        lifecycle.wait_for_transactions().await;
        assert_eq!(lifecycle.active_transactions(), 0);
    }
}
