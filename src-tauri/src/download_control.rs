use std::{collections::HashMap, future::Future, sync::OnceLock, time::Duration};
use tokio::{
    sync::{watch, RwLock},
    time::{sleep, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransferControl {
    Running,
    Paused,
    Cancelled,
}

#[derive(Debug)]
pub enum ControlledDownload<T> {
    Finished(T),
    Paused,
    Cancelled,
    AlreadyActive,
}

fn active_transfers() -> &'static RwLock<HashMap<String, watch::Sender<TransferControl>>> {
    static ACTIVE: OnceLock<RwLock<HashMap<String, watch::Sender<TransferControl>>>> =
        OnceLock::new();
    ACTIVE.get_or_init(|| RwLock::new(HashMap::new()))
}

pub async fn run_controlled<F, T>(task_id: String, future: F) -> ControlledDownload<T>
where
    F: Future<Output = T>,
{
    let (tx, mut rx) = watch::channel(TransferControl::Running);
    {
        let mut active = active_transfers().write().await;
        if active.contains_key(&task_id) {
            return ControlledDownload::AlreadyActive;
        }
        active.insert(task_id.clone(), tx);
    }

    let outcome = tokio::select! {
        result = future => ControlledDownload::Finished(result),
        changed = rx.changed() => {
            if changed.is_err() {
                ControlledDownload::Cancelled
            } else {
                match *rx.borrow() {
                    TransferControl::Paused => ControlledDownload::Paused,
                    TransferControl::Cancelled => ControlledDownload::Cancelled,
                    TransferControl::Running => ControlledDownload::Cancelled,
                }
            }
        }
    };

    active_transfers().write().await.remove(&task_id);
    outcome
}

async fn signal_and_wait(task_id: &str, control: TransferControl) -> bool {
    let sender = {
        let active = active_transfers().read().await;
        active.get(task_id).cloned()
    };
    let Some(sender) = sender else {
        return false;
    };
    if sender.send(control).is_err() {
        return false;
    }

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if !active_transfers().read().await.contains_key(task_id) {
            return true;
        }
        if Instant::now() >= deadline {
            return true;
        }
        sleep(Duration::from_millis(10)).await;
    }
}

pub async fn pause(task_id: &str) -> bool {
    signal_and_wait(task_id, TransferControl::Paused).await
}

pub async fn cancel(task_id: &str) -> bool {
    signal_and_wait(task_id, TransferControl::Cancelled).await
}
