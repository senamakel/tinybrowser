//! The refs a snapshot mints, and the rule that decides when one has gone
//! stale.
//!
//! # Why refs expire
//!
//! A ref names a node the agent saw in one snapshot. The page then changes — a
//! list re-renders, a modal opens, a navigation happens — and the node behind
//! that ref is either gone or, far worse, is now a different element occupying
//! the same position. Acting on it would succeed and do the wrong thing, and the
//! agent would report that it clicked what it meant to.
//!
//! So refs are scoped to the snapshot that produced them and to the document
//! they were taken in. A ref from an earlier snapshot of the same page is stale
//! and says so; a ref from before a navigation is stale for the same reason. The
//! remedy is always the same one sentence — snapshot again — which is why
//! [`crate::Error::StaleRef`] exists separately from "not found".

use std::collections::HashMap;

use crate::error::{Error, Result};

/// The refs minted by the most recent snapshot of one session.
#[derive(Debug, Default)]
pub(crate) struct RefMap {
    /// Which snapshot these came from, counting from one. Zero means no
    /// snapshot has been taken yet.
    sequence: u64,
    /// Ref id to the backend node id it names.
    nodes: HashMap<String, i64>,
}

impl RefMap {
    /// Replaces every ref with those of a new snapshot, and returns its
    /// sequence number.
    pub(crate) fn replace(&mut self, nodes: HashMap<String, i64>) -> u64 {
        self.sequence += 1;
        self.nodes = nodes;
        self.sequence
    }

    /// The sequence of the snapshot currently minting refs.
    pub(crate) fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Resolves `reference` to the backend node id it names.
    ///
    /// # Errors
    ///
    /// [`Error::StaleRef`] when no snapshot has been taken, or when the ref is
    /// not in the current one — those are the same situation from the caller's
    /// side, and both are fixed by taking a snapshot.
    pub(crate) fn resolve(&self, reference: &str) -> Result<i64> {
        let reference = reference.trim_start_matches('@');

        self.nodes.get(reference).copied().ok_or(Error::StaleRef {
            reference: reference.to_string(),
            // A ref this map has never held came from *some* earlier snapshot;
            // reporting zero when none has been taken says exactly that.
            minted: 0,
            current: self.sequence,
        })
    }
}
