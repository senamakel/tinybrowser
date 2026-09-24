//! Errors from task validation, Jev decisions, and browser execution.

/// A failure before or during an agentic browser task.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The caller supplied a task or limits that cannot be executed.
    #[error("invalid task: {message}")]
    InvalidTask {
        /// What must be corrected.
        message: String,
    },
    /// Jev returned a validated response whose answers cannot form an action.
    #[error("invalid decision: {message}")]
    InvalidDecision {
        /// Which expected answer or mapping was absent.
        message: String,
    },
    /// The Jev request failed.
    #[error("decision provider failed: {source}")]
    Provider {
        /// The measured provider failure.
        #[source]
        source: tinyjevclient::EvaluationFailure,
    },
    /// `TinyBrowser` could not snapshot or act on the page.
    #[error("browser control failed: {source}")]
    Browser {
        /// The engine failure.
        #[source]
        source: BrowserControlError,
    },
}

impl Error {
    pub(crate) fn invalid_task(message: impl Into<String>) -> Self {
        Self::InvalidTask {
            message: message.into(),
        }
    }

    pub(crate) fn invalid_decision(message: impl Into<String>) -> Self {
        Self::InvalidDecision {
            message: message.into(),
        }
    }
}

impl From<tinyjevclient::EvaluationFailure> for Error {
    fn from(source: tinyjevclient::EvaluationFailure) -> Self {
        Self::Provider { source }
    }
}

impl From<BrowserControlError> for Error {
    fn from(source: BrowserControlError) -> Self {
        Self::Browser { source }
    }
}

/// A browser port failure with the stable wire name used for recovery policy.
#[derive(Debug, thiserror::Error)]
#[error("{name}: {message}")]
pub struct BrowserControlError {
    /// Stable `TinyBrowser` error name.
    pub name: String,
    /// Human-readable failure detail.
    pub message: String,
}

impl BrowserControlError {
    /// Return the stable error name.
    #[must_use]
    pub fn wire_name(&self) -> &str {
        &self.name
    }
}

#[cfg(feature = "engine")]
impl From<tinybrowser::Error> for BrowserControlError {
    fn from(source: tinybrowser::Error) -> Self {
        Self {
            name: source.wire_name().to_owned(),
            message: source.to_string(),
        }
    }
}

/// A result produced by the agentic controller.
pub type Result<T> = std::result::Result<T, Error>;
