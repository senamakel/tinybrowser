//! Fast, bounded agentic control over [`tinybrowser`].
//!
//! `tinybrowser-control` keeps the browser engine and its wire contract
//! provider-neutral while adding an optional Jev decision loop. Every step is
//! one typed System One request: Jev selects an operation and speculative
//! targets, an independent Noul answer checks completion, and deterministic
//! Rust code applies budgets and irreversible-action policy before calling
//! [`tinybrowser::Browser`] or a host-owned [`BrowserControl`] port.
//!
//! The model never emits selectors, coordinates, JavaScript, or text to type.
//! It selects only refs minted by the current accessibility snapshot and names
//! of values the caller already supplied.
//!
//! This crate does not own session lifecycle: [`JevController::run`] neither
//! opens nor closes sessions. Origin policy, current-ref validation, hit testing,
//! and input dispatch remain in `tinybrowser`; provider-specific transport stays
//! with the caller. Keeping those responsibilities outside preserves this crate's
//! provider-neutral control role.
//!
//! # Example
//!
//! ```no_run
//! # #[cfg(feature = "engine")]
//! # mod engine_example {
//! use std::collections::BTreeMap;
//! use tinybrowser::{Browser, NavigateRequest, SessionOptions};
//! use tinybrowser_control::{JevController, TaskRequest};
//! use tinyjevclient::Client;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let browser = Browser::new();
//! let session = browser.open_session(SessionOptions::default()).await?;
//! browser
//!     .navigate(&session.id, &NavigateRequest::new("https://example.com"))
//!     .await?;
//!
//! let controller = JevController::new(Client::from_env()?);
//! let task = TaskRequest::new("Search for TinyBrowser")
//!     .with_inputs(BTreeMap::from([("search query".into(), "TinyBrowser".into())]));
//! let result = controller.run(&browser, &session.id, &task).await?;
//! println!("{:?}: {} actions", result.status, result.steps.len());
//! # Ok(())
//! # }
//! # }
//! ```

mod error;
mod policy;
mod types;

#[cfg(feature = "engine")]
use tinybrowser::Browser;
use tinybrowser_bus::{Action, ActionOutcome, SessionId, Snapshot, SnapshotRequest, errors};
use tinyjevclient::Client;

pub use error::{BrowserControlError, Error, Result};
pub use types::{
    ControlLimits, Decision, Operation, StepOutcome, StepRecord, TaskRequest, TaskResult,
    TaskStatus,
};

/// A Jev decision client plus deterministic loop limits.
#[derive(Clone, Debug)]
pub struct JevController {
    client: Client,
    limits: ControlLimits,
}

/// Browser operations needed by the Jev loop. Hosts may implement this over `TinyBus`.
#[allow(async_fn_in_trait)]
pub trait BrowserControl {
    /// Read the current accessibility snapshot for a session.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserControlError`] when the session is unavailable or the
    /// browser cannot capture its page.
    async fn snapshot(
        &self,
        session: &SessionId,
        request: &SnapshotRequest,
    ) -> std::result::Result<Snapshot, BrowserControlError>;

    /// Perform one typed action in that session.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserControlError`] when the action is refused, its target
    /// is stale, or the browser cannot complete it.
    async fn perform(
        &self,
        session: &SessionId,
        action: &Action,
    ) -> std::result::Result<ActionOutcome, BrowserControlError>;
}

#[cfg(feature = "engine")]
impl BrowserControl for Browser {
    async fn snapshot(
        &self,
        session: &SessionId,
        request: &SnapshotRequest,
    ) -> std::result::Result<Snapshot, BrowserControlError> {
        Browser::snapshot(self, session, request)
            .await
            .map_err(Into::into)
    }

    async fn perform(
        &self,
        session: &SessionId,
        action: &Action,
    ) -> std::result::Result<ActionOutcome, BrowserControlError> {
        Browser::perform(self, session, action)
            .await
            .map_err(Into::into)
    }
}

trait DecisionSource {
    async fn decide(
        &self,
        task: &TaskRequest,
        snapshot: &Snapshot,
        history: &[StepRecord],
    ) -> Result<Decision>;
}

impl DecisionSource for JevController {
    async fn decide(
        &self,
        task: &TaskRequest,
        snapshot: &Snapshot,
        history: &[StepRecord],
    ) -> Result<Decision> {
        JevController::decide(self, task, snapshot, history).await
    }
}

impl JevController {
    /// Create a controller with finite default limits.
    #[must_use]
    pub fn new(client: Client) -> Self {
        Self {
            client,
            limits: ControlLimits::default(),
        }
    }

    /// Replace the task-loop limits.
    #[must_use]
    pub fn with_limits(mut self, limits: ControlLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Ask Jev for one typed next action without executing it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidTask`] for an empty goal,
    /// [`Error::Provider`] when the evaluation fails, and
    /// [`Error::InvalidDecision`] when the response cannot map back to the
    /// current snapshot.
    pub async fn decide(
        &self,
        task: &TaskRequest,
        snapshot: &Snapshot,
        history: &[StepRecord],
    ) -> Result<Decision> {
        let request = policy::build_request(task, snapshot, history)?;
        let result = self.client.evaluate(&request).await?;
        policy::decode(&result, snapshot, task)
    }

    /// Run a bounded task against one existing `TinyBrowser` session.
    ///
    /// The method does not open or close the session. A likely irreversible
    /// click returns [`TaskStatus::NeedsConfirmation`] before acting unless
    /// [`TaskRequest::allow_irreversible`] is true.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidTask`] for unusable limits or a blank goal,
    /// [`Error::Provider`] for Jev failures, [`Error::InvalidDecision`] for an
    /// answer that cannot become a typed action, and [`Error::Browser`] for
    /// non-recoverable browser failures.
    pub async fn run(
        &self,
        browser: &impl BrowserControl,
        session: &SessionId,
        task: &TaskRequest,
    ) -> Result<TaskResult> {
        self.validate(task)?;
        self.run_with(browser, self, session, task).await
    }

    async fn run_with(
        &self,
        browser: &impl BrowserControl,
        decisions: &impl DecisionSource,
        session: &SessionId,
        task: &TaskRequest,
    ) -> Result<TaskResult> {
        let snapshot_request = SnapshotRequest {
            // Keep visible prose for the independent completion check. Action
            // candidates are still filtered to interactive roles in policy.
            interactive_only: false,
            compact: true,
            max_chars: 50_000,
            ..SnapshotRequest::default()
        };
        let mut snapshot = browser.snapshot(session, &snapshot_request).await?;
        let mut history = Vec::new();
        let mut unchanged = 0_usize;

        for step in 1..=self.limits.max_steps {
            let decision = decisions.decide(task, &snapshot, &history).await?;
            if let Some(status) =
                policy::terminal_status(&decision, self.limits.completion_threshold)
            {
                return Ok(TaskResult {
                    status,
                    steps: history,
                    final_snapshot: snapshot,
                    pending: None,
                    terminal: Some(decision),
                });
            }
            if policy::is_irreversible(&decision) && !task.allow_irreversible {
                return Ok(TaskResult {
                    status: TaskStatus::NeedsConfirmation,
                    steps: history,
                    final_snapshot: snapshot,
                    pending: Some(decision),
                    terminal: None,
                });
            }

            let action = decision
                .to_action(task, self.limits.wait_ms)?
                .ok_or_else(|| {
                    Error::invalid_decision("non-terminal decision produced no action")
                })?;
            let outcome = match browser.perform(session, &action).await {
                Ok(_) => StepOutcome::Acted,
                Err(source) if errors::is_agent_recoverable(source.wire_name()) => {
                    StepOutcome::RecoverableError {
                        name: source.wire_name().to_owned(),
                    }
                }
                Err(source) => return Err(source.into()),
            };
            let after = browser.snapshot(session, &snapshot_request).await?;
            let changed = policy::page_changed(&snapshot, &after);
            unchanged = policy::next_unchanged(unchanged, decision.operation, changed);
            history.push(StepRecord {
                step,
                decision,
                page_changed: changed,
                outcome,
            });
            snapshot = after;
            if unchanged >= self.limits.max_unchanged_steps {
                return Ok(TaskResult {
                    status: TaskStatus::Stuck,
                    steps: history,
                    final_snapshot: snapshot,
                    pending: None,
                    terminal: None,
                });
            }
        }

        Ok(TaskResult {
            status: TaskStatus::Budget,
            steps: history,
            final_snapshot: snapshot,
            pending: None,
            terminal: None,
        })
    }

    fn validate(&self, task: &TaskRequest) -> Result<()> {
        if task.goal.trim().is_empty() {
            return Err(Error::invalid_task("goal must not be empty"));
        }
        if task.inputs.keys().any(|name| name.trim().is_empty()) {
            return Err(Error::invalid_task("input names must not be empty"));
        }
        if self.limits.max_steps == 0 {
            return Err(Error::invalid_task("max_steps must be greater than zero"));
        }
        if self.limits.max_unchanged_steps == 0 {
            return Err(Error::invalid_task(
                "max_unchanged_steps must be greater than zero",
            ));
        }
        if !(0.0..=1.0).contains(&self.limits.completion_threshold) {
            return Err(Error::invalid_task(
                "completion_threshold must be between zero and one",
            ));
        }
        Ok(())
    }
}

#[cfg(all(test, feature = "engine"))]
mod test;
