//! Public task, decision, trace, and limit types.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use tinybrowser::{Action, ElementRef, ScrollDirection, Snapshot, Target, WaitState};
use tinyjevclient::Usage;

use crate::{Error, Result};

/// A goal and the concrete values the controller may enter into fields.
#[derive(Clone, PartialEq, Eq)]
pub struct TaskRequest {
    /// What the browser should achieve.
    pub goal: String,
    /// Caller-owned names mapped to exact values. Jev chooses names, while the
    /// values remain local until `TinyBrowser` performs the selected fill.
    pub inputs: BTreeMap<String, String>,
    /// Whether deterministic policy may execute likely irreversible clicks.
    pub allow_irreversible: bool,
}

impl TaskRequest {
    /// Create a task with no form inputs and irreversible actions disabled.
    #[must_use]
    pub fn new(goal: impl Into<String>) -> Self {
        Self {
            goal: goal.into(),
            inputs: BTreeMap::new(),
            allow_irreversible: false,
        }
    }

    /// Supply concrete values Jev may assign to fields by name.
    #[must_use]
    pub fn with_inputs(mut self, inputs: BTreeMap<String, String>) -> Self {
        self.inputs = inputs;
        self
    }

    /// Permit clicks such as buy, pay, send, publish, and delete.
    ///
    /// Set this only after the user has authorized the effect represented by
    /// the task.
    #[must_use]
    pub fn allowing_irreversible(mut self) -> Self {
        self.allow_irreversible = true;
        self
    }
}

impl fmt::Debug for TaskRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TaskRequest")
            .field("goal", &self.goal)
            .field("input_names", &self.inputs.keys().collect::<Vec<_>>())
            .field("allow_irreversible", &self.allow_irreversible)
            .finish()
    }
}

/// Finite limits and thresholds for one controller run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlLimits {
    /// Maximum Jev decisions before returning [`TaskStatus::Budget`].
    pub max_steps: usize,
    /// Consecutive non-wait actions allowed without a visible snapshot change.
    pub max_unchanged_steps: usize,
    /// Minimum independent goal probability required to accept `DONE`.
    pub completion_threshold: f64,
    /// Flat delay used when Jev selects `WAIT`.
    pub wait_ms: u64,
}

impl Default for ControlLimits {
    fn default() -> Self {
        Self {
            max_steps: 30,
            max_unchanged_steps: 3,
            completion_threshold: 0.5,
            wait_ms: 250,
        }
    }
}

/// The closed operation set Jev can choose from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    /// Click one current snapshot ref.
    Click,
    /// Fill one current field with one caller-supplied input.
    Fill,
    /// Leave a checkbox, radio, or switch checked.
    Check,
    /// Scroll the page down by one viewport.
    ScrollDown,
    /// Scroll the page up by one viewport.
    ScrollUp,
    /// Move back in browser history.
    Back,
    /// Wait briefly for an in-progress page update.
    Wait,
    /// Stop because the visible page satisfies the goal.
    Done,
    /// Stop because no supported operation can make progress.
    Blocked,
}

impl Operation {
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::Click => "CLICK",
            Self::Fill => "FILL",
            Self::Check => "CHECK",
            Self::ScrollDown => "SCROLL_DOWN",
            Self::ScrollUp => "SCROLL_UP",
            Self::Back => "BACK",
            Self::Wait => "WAIT",
            Self::Done => "DONE",
            Self::Blocked => "BLOCKED",
        }
    }

    pub(crate) fn from_key(key: &str) -> Option<Self> {
        [
            Self::Click,
            Self::Fill,
            Self::Check,
            Self::ScrollDown,
            Self::ScrollUp,
            Self::Back,
            Self::Wait,
            Self::Done,
            Self::Blocked,
        ]
        .into_iter()
        .find(|operation| operation.key() == key)
    }
}

/// One typed Jev decision, before deterministic policy executes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    /// The chosen operation.
    pub operation: Operation,
    /// Concentration reported for the operation choice.
    pub confidence: f64,
    /// Independent probability that the visible page satisfies the goal.
    pub goal_done: f64,
    /// The current snapshot element selected for a targeted operation.
    pub target: Option<ElementRef>,
    /// Concentration reported for the selected target.
    pub target_confidence: Option<f64>,
    /// Name of the caller-supplied value selected for a fill.
    pub input_name: Option<String>,
    /// HTTP attempts consumed by the decision.
    pub attempts: u32,
    /// End-to-end provider latency, including retries.
    pub latency: Duration,
    /// Provider-reported token usage.
    pub usage: Usage,
    /// Provider-resolved model identifier.
    pub model: String,
    /// Provider request identifier, when returned.
    pub request_id: Option<String>,
}

impl Decision {
    /// Map this decision to a typed `TinyBrowser` action.
    ///
    /// Terminal operations return `None`. The caller supplies the bounded wait
    /// used for [`Operation::Wait`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDecision`] when a targeted operation has no
    /// target, or when a fill does not name one of `task`'s inputs.
    pub fn to_action(&self, task: &TaskRequest, wait_ms: u64) -> Result<Option<Action>> {
        let target = || {
            self.target
                .as_ref()
                .map(|element| Target::reference(&element.id))
                .ok_or_else(|| Error::invalid_decision("selected operation has no target"))
        };
        let action = match self.operation {
            Operation::Click => Some(Action::Click {
                target: target()?,
                new_tab: false,
            }),
            Operation::Fill => {
                let input_name = self.input_name.as_ref().ok_or_else(|| {
                    Error::invalid_decision("selected fill has no caller input name")
                })?;
                let value = task.inputs.get(input_name).ok_or_else(|| {
                    Error::invalid_decision(format!("selected unknown input {input_name}"))
                })?;
                Some(Action::Fill {
                    target: target()?,
                    value: value.clone(),
                })
            }
            Operation::Check => Some(Action::Check {
                target: target()?,
                checked: true,
            }),
            Operation::ScrollDown => Some(Action::Scroll {
                direction: ScrollDirection::Down,
                pixels: None,
                target: None,
            }),
            Operation::ScrollUp => Some(Action::Scroll {
                direction: ScrollDirection::Up,
                pixels: None,
                target: None,
            }),
            Operation::Back => Some(Action::Back),
            Operation::Wait => Some(Action::WaitFor {
                target: None,
                text: None,
                state: WaitState::Visible,
                ms: Some(wait_ms),
                timeout_ms: Some(wait_ms.saturating_add(1_000)),
            }),
            Operation::Done | Operation::Blocked => None,
        };
        Ok(action)
    }
}

/// What happened when `TinyBrowser` attempted a selected action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StepOutcome {
    /// The typed action completed.
    Acted,
    /// The page invalidated the target; the controller took a fresh snapshot.
    RecoverableError {
        /// Stable `TinyBrowser` wire error name.
        name: String,
    },
}

/// One decision and its observable browser result.
#[derive(Clone, Debug, PartialEq)]
pub struct StepRecord {
    /// One-based step number.
    pub step: usize,
    /// The Jev decision that was applied.
    pub decision: Decision,
    /// Whether the next snapshot differed in URL, title, or rendered tree.
    pub page_changed: bool,
    /// Browser outcome for the action.
    pub outcome: StepOutcome,
}

/// Why a controller run stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskStatus {
    /// `DONE` passed the independent completion threshold.
    Done,
    /// `DONE` did not pass the independent completion threshold.
    DoneUnconfirmed,
    /// Jev selected `BLOCKED`.
    Blocked,
    /// Too many consecutive actions left the visible page unchanged.
    Stuck,
    /// The configured step budget was exhausted.
    Budget,
    /// A likely irreversible click needs explicit caller approval.
    NeedsConfirmation,
}

/// The terminal result and trace of one controller run.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskResult {
    /// Why the run stopped.
    pub status: TaskStatus,
    /// Every action `TinyBrowser` attempted.
    pub steps: Vec<StepRecord>,
    /// The page observation at the stop point.
    pub final_snapshot: Snapshot,
    /// Unexecuted decision when confirmation is required.
    pub pending: Option<Decision>,
    /// The `DONE` or `BLOCKED` decision that ended the loop.
    ///
    /// Retained so callers can inspect completion probability, confidence,
    /// latency, attempts, and usage even though terminal decisions do not
    /// produce an action step.
    pub terminal: Option<Decision>,
}
