use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex},
};
use tokio::sync::watch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferControl {
    Running,
    Paused,
    Cancelled,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ControlledDownload<T> {
    Finished(T),
    Paused,
    Cancelled,
}

struct ActiveTransfer {
    control: watch::Sender<TransferControl>,
    finished: watch::Receiver<()>,
}

#[derive(Clone, Default)]
pub struct TransferRegistry(Arc<Mutex<HashMap<String, ActiveTransfer>>>);

// The lease covers acquisition, transfer, progress draining and final persistence.
// A new attempt cannot take ownership until all of those steps have finished.
pub struct TransferLease {
    registry: TransferRegistry,
    id: String,
    control: watch::Receiver<TransferControl>,
    _finished: watch::Sender<()>,
}

impl TransferRegistry {
    pub fn begin(&self, id: &str) -> Result<TransferLease, String> {
        let mut active = self
            .0
            .lock()
            .map_err(|_| "download registry is unavailable")?;
        if active.contains_key(id) {
            return Err("download task is already active".into());
        }
        let (control, rx) = watch::channel(TransferControl::Running);
        let (finished, finished_rx) = watch::channel(());
        active.insert(
            id.into(),
            ActiveTransfer {
                control,
                finished: finished_rx,
            },
        );
        Ok(TransferLease {
            registry: self.clone(),
            id: id.into(),
            control: rx,
            _finished: finished,
        })
    }

    pub async fn stop(&self, id: &str, control: TransferControl) -> Result<bool, String> {
        let mut finished = {
            let active = self
                .0
                .lock()
                .map_err(|_| "download registry is unavailable")?;
            let Some(transfer) = active.get(id) else {
                return Ok(false);
            };
            // The first control request wins; a later cancel cannot race a pause.
            transfer.control.send_if_modified(|current| {
                if *current != TransferControl::Running {
                    return false;
                }
                *current = control;
                true
            });
            transfer.finished.clone()
        };
        // This sender is dropped only after the worker persists its final state.
        let _ = finished.changed().await;
        Ok(true)
    }
}

impl TransferLease {
    pub async fn run<F: Future>(&mut self, future: F) -> ControlledDownload<F::Output> {
        tokio::select! {
            biased;
            control = self.control.wait_for(|value| *value != TransferControl::Running) => {
                match control.as_deref() {
                    Ok(TransferControl::Paused) => ControlledDownload::Paused,
                    _ => ControlledDownload::Cancelled,
                }
            }
            result = future => ControlledDownload::Finished(result),
        }
    }
}

impl Drop for TransferLease {
    fn drop(&mut self) {
        if let Ok(mut active) = self.registry.0.lock() {
            active.remove(&self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::pending;
    use tokio::time::{timeout, Duration};

    #[tokio::test]
    async fn pause_waits_for_finalization_and_blocks_overlapping_attempts() {
        let registry = TransferRegistry::default();
        let mut lease = registry.begin("book").unwrap();
        assert!(registry.begin("book").is_err());
        let stop = registry.stop("book", TransferControl::Paused);
        tokio::pin!(stop);
        {
            let transfer = lease.run(pending::<()>());
            tokio::pin!(transfer);
            tokio::select! {
                result = &mut stop => panic!("pause returned before cleanup: {result:?}"),
                outcome = &mut transfer => assert_eq!(outcome, ControlledDownload::Paused),
            }
        }
        assert!(registry.begin("book").is_err());
        assert!(timeout(Duration::from_millis(20), &mut stop).await.is_err());
        drop(lease);
        assert!(timeout(Duration::from_secs(1), stop)
            .await
            .unwrap()
            .unwrap());
        assert!(registry.begin("book").is_ok());
    }

    #[tokio::test]
    async fn completed_transfer_holds_ownership_until_record_is_saved() {
        let registry = TransferRegistry::default();
        let mut lease = registry.begin("book").unwrap();
        assert_eq!(
            lease.run(async { 42 }).await,
            ControlledDownload::Finished(42)
        );
        assert!(registry.begin("book").is_err());
        drop(lease);
        assert!(!registry
            .stop("book", TransferControl::Cancelled)
            .await
            .unwrap());
        assert!(registry.begin("book").is_ok());
    }
}
