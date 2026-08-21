//! Tests for extraction.
//!
//! The traversal runs in the page, so what is testable here is the boundary
//! around it: the truncation that decides how much of a page a model sees.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::truncate;

#[test]
fn content_within_the_limit_is_untouched() {
    let (content, truncated) = truncate("hello".to_string(), 200_000);

    assert_eq!(content, "hello");
    assert!(!truncated);
}

#[test]
fn content_exactly_at_the_limit_is_not_truncated() {
    let (content, truncated) = truncate("hello".to_string(), 5);

    assert_eq!(content, "hello");
    assert!(!truncated);
}

#[test]
fn content_over_the_limit_is_cut_and_reported() {
    // Reporting matters as much as cutting: a model shown a truncated page
    // without being told will conclude the rest of it does not exist.
    let (content, truncated) = truncate("hello world".to_string(), 5);

    assert_eq!(content, "hello");
    assert!(truncated);
}

#[test]
fn truncation_counts_characters_not_bytes() {
    // A byte-wise cut through a multi-byte character produces invalid UTF-8,
    // which would fail to serialize onto the bus rather than returning a short
    // string.
    let (content, truncated) = truncate("héllo wörld".to_string(), 5);

    assert_eq!(content, "héllo");
    assert!(truncated);
    assert_eq!(content.chars().count(), 5);
}

#[test]
fn a_zero_limit_yields_nothing_rather_than_panicking() {
    let (content, truncated) = truncate("hello".to_string(), 0);

    assert!(content.is_empty());
    assert!(truncated);
}

#[test]
fn empty_content_is_never_truncated() {
    let (content, truncated) = truncate(String::new(), 0);

    assert!(content.is_empty());
    assert!(!truncated);
}
