//! One WebSocket to a browser, shared by everything driving it.
//!
//! # The shape of the thing
//!
//! CDP is JSON-RPC with events: a command carries an `id` and comes back with
//! that `id`, and anything without one is an event. So the client is a reader
//! task plus two maps' worth of state — a table of in-flight commands keyed by
//! id, and a broadcast channel for events — and [`CdpClient::send`] is a
//! oneshot parked in the first table.
//!
//! That arrangement is what makes the socket shareable. Sessions, snapshots and
//! screenshots all issue commands concurrently against one `Arc<CdpClient>`
//! without a lock held across any await, because nothing waits on the socket —
//! it waits on its own oneshot.
//!
//! # Flat sessions
//!
//! Every command may carry a `sessionId`. Attaching to a page target with
//! `flatten: true` gets one, and from then on a command addressed to that id
//! runs in that page while the same socket still reaches the browser itself.
//! One connection, every target: no second socket per tab, and no reconnect
//! when a tab goes away.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::{Mutex, broadcast, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

use crate::error::{Error, Result};

/// How many events to buffer for a subscriber that is not reading fast enough.
///
/// A busy page emits thousands of network and lifecycle events. A subscriber
/// that falls this far behind is told it lagged rather than being allowed to
/// hold the reader task hostage.
const EVENT_BUFFER: usize = 2_048;

/// How often to ping the socket.
///
/// A CDP socket carrying an idle page sends nothing for minutes at a time, and
/// an intermediary between this module and a remote browser will happily reap
/// what looks like a dead connection.
const KEEPALIVE: Duration = Duration::from_secs(30);

/// A CDP event: something the browser said without being asked.
#[derive(Debug, Clone)]
pub(crate) struct CdpEvent {
    /// The event name, such as `Page.loadEventFired`.
    pub(crate) method: String,
    /// Its parameters.
    pub(crate) params: Value,
    /// The page session it belongs to, absent for browser-level events.
    pub(crate) session_id: Option<String>,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<CommandReply>>>>;

/// What came back for one command.
#[derive(Debug)]
struct CommandReply {
    result: Option<Value>,
    error: Option<Value>,
}

/// A connected CDP socket.
#[derive(Debug)]
pub(crate) struct CdpClient {
    sink: Mutex<Sink>,
    next_id: AtomicU64,
    pending: Pending,
    events: broadcast::Sender<CdpEvent>,
    reader: tokio::task::JoinHandle<()>,
    /// Set once, immediately after construction.
    ///
    /// The keepalive task holds a `Weak` back to this client so that pinging
    /// does not keep the socket alive forever, which means the client must
    /// exist before the task can be spawned. A `OnceLock` is what lets the
    /// field be filled afterwards without an `Arc::try_unwrap` that would
    /// invalidate the very `Weak` the task depends on.
    keepalive: std::sync::OnceLock<tokio::task::JoinHandle<()>>,
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
type Sink = futures_util::stream::SplitSink<Socket, Message>;

impl Drop for CdpClient {
    fn drop(&mut self) {
        // The reader owns the read half of the socket and the keepalive owns a
        // timer; neither ends on its own when the last handle goes away.
        self.reader.abort();
        if let Some(keepalive) = self.keepalive.get() {
            keepalive.abort();
        }
    }
}

/// Removes an in-flight command if the caller's future is dropped.
///
/// A `send` cancelled by an outer deadline would otherwise leave its entry in
/// the table until the socket closes — one leaked oneshot per timed-out
/// command, on exactly the pages where commands time out most.
struct PendingGuard {
    pending: Pending,
    id: u64,
    armed: bool,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let pending = Arc::clone(&self.pending);
        let id = self.id;
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                pending.lock().await.remove(&id);
            });
        }
    }
}

impl CdpClient {
    /// Connects to the browser at `url`.
    ///
    /// # Errors
    ///
    /// [`Error::BrowserUnavailable`] when the socket cannot be opened.
    pub(crate) async fn connect(url: &str) -> Result<Arc<Self>> {
        let config = WebSocketConfig {
            // A full-page screenshot arrives base64-encoded in a single CDP
            // frame and routinely exceeds tungstenite's 16 MiB default. The
            // module bounds the image itself; bounding it again here would only
            // turn a large screenshot into a closed socket.
            max_message_size: None,
            max_frame_size: None,
            ..Default::default()
        };

        let (socket, _) = tokio_tungstenite::connect_async_with_config(url, Some(config), false)
            .await
            .map_err(|error| {
                Error::browser_unavailable(format!("cdp connect to {url} failed: {error}"))
            })?;

        let (sink, mut stream) = socket.split();
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (events, _) = broadcast::channel(EVENT_BUFFER);

        let reader_pending = Arc::clone(&pending);
        let reader_events = events.clone();
        let (closed_tx, mut closed_rx) = tokio::sync::watch::channel(false);

        let reader = tokio::spawn(async move {
            while let Some(message) = stream.next().await {
                let text = match message {
                    Ok(Message::Text(text)) => text,
                    // A CDP proxy in front of a remote browser may frame its
                    // replies as binary; the payload is the same JSON.
                    Ok(Message::Binary(bytes)) => match String::from_utf8(bytes) {
                        Ok(text) => text,
                        Err(_) => continue,
                    },
                    Ok(Message::Close(_)) | Err(_) => break,
                    Ok(_) => continue,
                };

                let Ok(message) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };

                if let Some(id) = message.get("id").and_then(Value::as_u64) {
                    if let Some(sender) = reader_pending.lock().await.remove(&id) {
                        let _ = sender.send(CommandReply {
                            result: message.get("result").cloned(),
                            error: message.get("error").cloned(),
                        });
                    }
                } else if let Some(method) = message.get("method").and_then(Value::as_str) {
                    let _ = reader_events.send(CdpEvent {
                        method: method.to_string(),
                        params: message.get("params").cloned().unwrap_or(Value::Null),
                        session_id: message
                            .get("sessionId")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    });
                }
            }

            // The socket is gone. Dropping every pending sender fails the
            // commands waiting on it immediately, rather than making each one
            // sit out its own deadline against a connection that will never
            // answer.
            reader_pending.lock().await.clear();
            let _ = closed_tx.send(true);
        });

        let client = Arc::new(Self {
            sink: Mutex::new(sink),
            next_id: AtomicU64::new(1),
            pending,
            events,
            reader,
            keepalive: std::sync::OnceLock::new(),
        });

        let pinger = Arc::downgrade(&client);
        let keepalive = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(KEEPALIVE);
            ticker.tick().await;
            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        // A weak handle on purpose: a strong one would keep the
                        // client — and so the socket — alive for as long as this
                        // task runs, which is forever.
                        let Some(client) = pinger.upgrade() else { break };
                        if client.sink.lock().await.send(Message::Ping(Vec::new())).await.is_err() {
                            break;
                        }
                    }
                    _ = closed_rx.changed() => break,
                }
            }
        });

        // Set, never replaced: `connect` is the only writer and it runs before
        // any other reference to this client exists.
        let _ = client.keepalive.set(keepalive);
        Ok(client)
    }

    /// Sends `method` with `params` and waits for its reply.
    ///
    /// `session_id` addresses a page attached with `flatten: true`; `None`
    /// addresses the browser itself.
    ///
    /// # Errors
    ///
    /// [`Error::ConnectionLost`] when the socket is gone, [`Error::Timeout`]
    /// when the browser does not answer in time, and [`Error::PageError`] when
    /// it answers with a protocol error.
    pub(crate) async fn send(
        &self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
        timeout: Duration,
    ) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();

        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session_id {
            message["sessionId"] = json!(session);
        }

        self.pending.lock().await.insert(id, sender);
        let mut guard = PendingGuard {
            pending: Arc::clone(&self.pending),
            id,
            armed: true,
        };

        self.sink
            .lock()
            .await
            .send(Message::Text(message.to_string()))
            .await
            .map_err(|error| Error::connection_lost(format!("sending {method}: {error}")))?;

        let reply = match tokio::time::timeout(timeout, receiver).await {
            Ok(Ok(reply)) => {
                guard.armed = false;
                reply
            }
            // The sender was dropped: the reader loop cleared the table because
            // the socket closed.
            Ok(Err(_)) => {
                guard.armed = false;
                return Err(Error::connection_lost(format!(
                    "browser closed the connection during {method}"
                )));
            }
            Err(_) => {
                return Err(Error::timeout(
                    method.to_string(),
                    u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
                ));
            }
        };

        if let Some(error) = reply.error {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown protocol error");
            return Err(Error::page(format!("{method}: {message}")));
        }

        Ok(reply.result.unwrap_or(Value::Null))
    }

    /// Subscribes to every event the browser emits from now on.
    pub(crate) fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }
}
