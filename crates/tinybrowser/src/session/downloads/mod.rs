//! Retained Chrome download events and the waitable handle queue.
//!
//! The browser emits these events independently of page navigation. The store
//! is deliberately session-owned: events that arrive immediately after a click
//! remain available when the host calls `WaitDownload` afterwards.

#[cfg(test)]
mod test;

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tinybrowser_bus::{DownloadId, DownloadInfo, DownloadState};
use tokio::sync::{Mutex, Notify, broadcast};

use crate::cdp::CdpEvent;
use crate::error::{Error, Result};

#[derive(Debug, Default)]
struct DownloadStore {
    next_sequence: u64,
    downloads: BTreeMap<DownloadId, DownloadInfo>,
    terminal: VecDeque<DownloadId>,
    queued: HashSet<DownloadId>,
}

impl DownloadStore {
    fn apply(&mut self, event: &CdpEvent, directory: Option<&Path>) -> bool {
        match event.method.as_str() {
            "Browser.downloadWillBegin" => self.begin(&event.params, directory),
            "Browser.downloadProgress" => self.progress(&event.params),
            _ => false,
        }
    }

    fn begin(&mut self, params: &serde_json::Value, directory: Option<&Path>) -> bool {
        let Some(guid) = params.get("guid").and_then(serde_json::Value::as_str) else {
            return false;
        };
        let id = DownloadId::new(guid);
        if self.downloads.contains_key(&id) {
            return false;
        }
        self.next_sequence = self.next_sequence.saturating_add(1);
        let suggested_filename = params
            .get("suggestedFilename")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let path = directory.and_then(|directory| expected_path(directory, &suggested_filename));
        self.downloads.insert(
            id.clone(),
            DownloadInfo {
                sequence: self.next_sequence,
                id,
                url: params
                    .get("url")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                suggested_filename,
                state: DownloadState::InProgress,
                received_bytes: 0,
                total_bytes: None,
                path: path.map(|path| path.to_string_lossy().into_owned()),
            },
        );
        true
    }

    fn progress(&mut self, params: &serde_json::Value) -> bool {
        let Some(guid) = params.get("guid").and_then(serde_json::Value::as_str) else {
            return false;
        };
        let id = DownloadId::new(guid);
        if !self.downloads.contains_key(&id) {
            self.next_sequence = self.next_sequence.saturating_add(1);
            self.downloads.insert(
                id.clone(),
                DownloadInfo {
                    sequence: self.next_sequence,
                    id: id.clone(),
                    url: String::new(),
                    suggested_filename: String::new(),
                    state: DownloadState::InProgress,
                    received_bytes: 0,
                    total_bytes: None,
                    path: None,
                },
            );
        }
        let Some(info) = self.downloads.get_mut(&id) else {
            return false;
        };
        info.received_bytes = byte_count(params.get("receivedBytes"));
        let total = byte_count(params.get("totalBytes"));
        info.total_bytes = (total > 0).then_some(total);
        info.state = match params.get("state").and_then(serde_json::Value::as_str) {
            Some("completed") => DownloadState::Completed,
            Some("canceled") => DownloadState::Cancelled,
            _ => DownloadState::InProgress,
        };
        if info.state.is_terminal() && self.queued.insert(id.clone()) {
            self.terminal.push_back(id);
        }
        true
    }

    fn take_terminal(&mut self) -> Option<DownloadInfo> {
        let id = self.terminal.pop_front()?;
        self.downloads.get(&id).cloned()
    }

    fn list(&self) -> Vec<DownloadInfo> {
        let mut downloads = self.downloads.values().cloned().collect::<Vec<_>>();
        downloads.sort_by_key(|info| info.sequence);
        downloads
    }
}

/// One session's retained downloads and waiter notification.
#[derive(Debug)]
pub(crate) struct DownloadTracker {
    store: Mutex<DownloadStore>,
    notify: Notify,
    directory: Option<PathBuf>,
}

impl DownloadTracker {
    pub(crate) fn new(directory: Option<&str>) -> Arc<Self> {
        Arc::new(Self {
            store: Mutex::new(DownloadStore::default()),
            notify: Notify::new(),
            directory: directory.map(PathBuf::from),
        })
    }

    pub(crate) fn spawn(
        tracker: Arc<Self>,
        mut events: broadcast::Receiver<CdpEvent>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                let event = match events.recv().await {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                let changed = tracker
                    .store
                    .lock()
                    .await
                    .apply(&event, tracker.directory.as_deref());
                if changed {
                    tracker.notify.notify_waiters();
                }
            }
        })
    }

    pub(crate) async fn list(&self) -> Vec<DownloadInfo> {
        self.store.lock().await.list()
    }

    pub(crate) async fn wait(&self, timeout: Duration) -> Result<DownloadInfo> {
        let wait = async {
            loop {
                let notified = self.notify.notified();
                if let Some(info) = self.store.lock().await.take_terminal() {
                    return info;
                }
                notified.await;
            }
        };
        tokio::time::timeout(timeout, wait)
            .await
            .map_err(|_| Error::timeout("download", duration_ms(timeout)))
    }
}

fn expected_path(directory: &Path, suggested_filename: &str) -> Option<PathBuf> {
    let filename = Path::new(suggested_filename).file_name()?;
    (!filename.is_empty()).then(|| directory.join(filename))
}

fn byte_count(value: Option<&serde_json::Value>) -> u64 {
    value
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_f64().and_then(float_byte_count))
        })
        .unwrap_or(0)
}

fn float_byte_count(number: f64) -> Option<u64> {
    if !number.is_finite() || number.is_sign_negative() {
        return None;
    }
    format!("{:.0}", number.trunc()).parse().ok()
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
