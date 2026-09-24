//! Tests for the parts of a session that need no browser.
//!
//! The session type itself cannot be built without a socket, so what is covered
//! here is everything it decides *around* that socket: which URLs it will
//! navigate to, which it refuses, how a ref goes stale, and how a CDP reply is
//! turned into a value or an error. The last one matters more than it looks: a
//! thrown exception arrives as a successful protocol reply, and reading it wrong
//! turns every page error into a silent `null`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;

use serde_json::json;
use tinybrowser_bus::WaitUntil;

use super::policy::{check_allowed, normalize_url};
use super::refs::RefMap;
use super::{lifecycle_name, string_field, unwrap_evaluation};
use crate::error::Error;

#[test]
fn a_bare_host_becomes_https() {
    assert_eq!(
        normalize_url("example.com").expect("normalizes").as_str(),
        "https://example.com/"
    );
    assert_eq!(
        normalize_url("localhost:3000")
            .expect("normalizes")
            .as_str(),
        "https://localhost:3000/"
    );
}

#[test]
fn an_explicit_scheme_is_kept() {
    assert_eq!(
        normalize_url("http://example.com/a?b=c")
            .expect("normalizes")
            .as_str(),
        "http://example.com/a?b=c"
    );
}

#[test]
fn about_blank_is_admitted_so_a_caller_can_clear_the_page() {
    assert_eq!(
        normalize_url("about:blank").expect("normalizes").as_str(),
        "about:blank"
    );
    assert_eq!(
        normalize_url("ABOUT:BLANK").expect("normalizes").as_str(),
        "about:blank"
    );
}

#[test]
fn file_urls_are_refused() {
    // A module that will navigate to `file:` is a filesystem reader for whoever
    // can reach the bus.
    let error = normalize_url("file:///etc/passwd").expect_err("refused");
    assert!(matches!(error, Error::InvalidInput { .. }), "{error}");
}

#[test]
fn javascript_urls_are_refused() {
    // Otherwise navigation becomes a second evaluation channel, one that skips
    // `Browser::evaluate` and every deadline attached to it.
    let error = normalize_url("javascript:alert(1)").expect_err("refused");
    assert!(matches!(error, Error::InvalidInput { .. }), "{error}");
}

#[test]
fn an_empty_url_is_refused() {
    let error = normalize_url("   ").expect_err("refused");
    assert!(matches!(error, Error::InvalidInput { .. }), "{error}");
}

#[test]
fn an_empty_allowlist_admits_everything() {
    let url = normalize_url("https://anywhere.test/").expect("normalizes");
    assert!(check_allowed(&url, &[]).is_ok());
}

#[test]
fn an_origin_entry_matches_only_that_origin() {
    let allowed = vec!["https://example.com".to_string()];

    assert!(check_allowed(&normalize_url("https://example.com/a").unwrap(), &allowed).is_ok());
    assert!(check_allowed(&normalize_url("http://example.com/a").unwrap(), &allowed).is_err());
    assert!(check_allowed(&normalize_url("https://other.test/").unwrap(), &allowed).is_err());
}

#[test]
fn a_dotted_entry_matches_the_host_and_its_subdomains() {
    let allowed = vec![".example.com".to_string()];

    assert!(check_allowed(&normalize_url("https://example.com/").unwrap(), &allowed).is_ok());
    assert!(
        check_allowed(
            &normalize_url("https://docs.example.com/").unwrap(),
            &allowed
        )
        .is_ok()
    );
    assert!(
        check_allowed(
            &normalize_url("https://a.b.example.com/").unwrap(),
            &allowed
        )
        .is_ok()
    );
}

#[test]
fn a_dotted_entry_does_not_admit_a_lookalike_domain() {
    // The whole point of the leading dot: a plain suffix comparison would admit
    // `evil-example.com`, which is a different site entirely.
    let allowed = vec![".example.com".to_string()];
    let url = normalize_url("https://evil-example.com/").expect("normalizes");

    let error = check_allowed(&url, &allowed).expect_err("refused");
    assert!(matches!(error, Error::BlockedByPolicy { .. }), "{error}");
}

#[test]
fn a_scheme_qualified_dotted_entry_allows_only_https_on_that_host_tree() {
    let allowed = vec!["https://.example.com".to_string()];
    for url in [
        "https://example.com/",
        "https://docs.example.com/path",
        "https://example.com./",
        "https://docs.example.com./path",
    ] {
        assert!(
            check_allowed(&normalize_url(url).unwrap(), &allowed).is_ok(),
            "{url}"
        );
    }
    for url in [
        "http://example.com/",
        "http://docs.example.com/",
        "https://evil-example.com/",
    ] {
        assert!(
            check_allowed(&normalize_url(url).unwrap(), &allowed).is_err(),
            "{url}"
        );
    }
}

#[test]
fn explicit_https_wildcard_blocks_http_local_names_and_all_ip_literals() {
    let allowed = vec!["https://.*".to_owned()];
    for url in ["https://example.com/", "https://docs.example.com/"] {
        assert!(
            check_allowed(&normalize_url(url).unwrap(), &allowed).is_ok(),
            "{url}"
        );
    }
    for url in [
        "http://example.com/",
        "https://localhost/",
        "https://test.local/",
        "https://127.0.0.1/",
        "https://10.0.0.1/",
        "https://169.254.1.2/",
        "https://100.64.0.1/",
        "https://198.19.0.1/",
        "https://192.0.0.1/",
        "https://8.8.8.8/",
        "https://[::1]/",
        "https://[fc00::1]/",
        "https://[::ffff:127.0.0.1]/",
    ] {
        assert!(
            check_allowed(&normalize_url(url).unwrap(), &allowed).is_err(),
            "{url}"
        );
    }
    assert!(
        check_allowed(
            &normalize_url("https://[2606:4700:4700::1111]/").unwrap(),
            &allowed
        )
        .is_err()
    );
}

#[test]
fn a_bare_host_entry_is_read_as_that_host() {
    // An operator who wrote `example.com` meant the site. Refusing to interpret
    // it would block everything instead, which is a worse failure than being
    // lenient about the form.
    let allowed = vec!["example.com".to_string()];

    assert!(check_allowed(&normalize_url("https://example.com/").unwrap(), &allowed).is_ok());
    assert!(check_allowed(&normalize_url("http://example.com/").unwrap(), &allowed).is_ok());
    assert!(check_allowed(&normalize_url("https://other.test/").unwrap(), &allowed).is_err());
}

#[test]
fn a_ref_from_no_snapshot_at_all_is_stale() {
    let map = RefMap::default();

    let error = map.resolve("e1").expect_err("refused");
    assert!(
        matches!(error, Error::StaleRef { current: 0, .. }),
        "{error}"
    );
}

#[test]
fn refs_resolve_within_the_snapshot_that_minted_them() {
    let mut map = RefMap::default();
    let sequence = map.replace(HashMap::from([("e1".to_string(), 42)]));

    assert_eq!(sequence, 1);
    assert_eq!(map.sequence(), 1);
    assert_eq!(map.resolve("e1").expect("resolves"), 42);
    assert_eq!(map.resolve("@e1").expect("resolves"), 42);
}

#[test]
fn a_new_snapshot_retires_the_previous_refs() {
    let mut map = RefMap::default();
    map.replace(HashMap::from([("e1".to_string(), 42)]));
    map.replace(HashMap::from([("e1".to_string(), 99)]));

    assert_eq!(map.sequence(), 2);
    assert_eq!(map.resolve("e1").expect("resolves"), 99);

    let error = map.resolve("e2").expect_err("refused");
    assert!(
        matches!(error, Error::StaleRef { current: 2, .. }),
        "{error}"
    );
}

#[test]
fn commit_waits_for_no_lifecycle_event() {
    assert!(lifecycle_name(WaitUntil::Commit).is_none());
}

#[test]
fn the_wait_modes_map_onto_chrome_lifecycle_names() {
    // These strings are Chrome's, not ours: a typo here means a navigation that
    // waits for an event that never arrives and settles at its deadline.
    assert_eq!(
        lifecycle_name(WaitUntil::DomContentLoaded),
        Some("DOMContentLoaded")
    );
    assert_eq!(lifecycle_name(WaitUntil::Load), Some("load"));
    assert_eq!(lifecycle_name(WaitUntil::NetworkIdle), Some("networkIdle"));
}

#[test]
fn a_missing_field_is_reported_against_the_browser() {
    let error = string_field(&json!({}), "targetId").expect_err("refused");
    assert!(matches!(error, Error::PageError { .. }), "{error}");
    assert!(error.to_string().contains("targetId"));
}

#[test]
fn an_evaluation_result_yields_its_value() {
    let result = json!({ "result": { "type": "string", "value": "hello" } });
    assert_eq!(unwrap_evaluation(&result).expect("a value"), json!("hello"));
}

#[test]
fn an_undefined_result_is_null_rather_than_an_error() {
    let result = json!({ "result": { "type": "undefined" } });
    assert_eq!(unwrap_evaluation(&result).expect("a value"), json!(null));
}

#[test]
fn a_thrown_exception_is_an_error_not_a_null() {
    // CDP reports a throw as a *successful* reply carrying exceptionDetails. A
    // caller that only checks for transport failures reads this as `undefined`
    // and reports that the script ran.
    let result = json!({
        "result": { "type": "object", "subtype": "error" },
        "exceptionDetails": {
            "text": "Uncaught",
            "exception": { "description": "ReferenceError: nope is not defined" },
        },
    });

    let error = unwrap_evaluation(&result).expect_err("refused");
    assert!(matches!(error, Error::PageError { .. }), "{error}");
    assert!(error.to_string().contains("ReferenceError"));
}

#[test]
fn an_exception_without_a_description_still_reports_something() {
    let result = json!({ "exceptionDetails": { "text": "Uncaught (in promise)" } });

    let error = unwrap_evaluation(&result).expect_err("refused");
    assert!(error.to_string().contains("Uncaught (in promise)"));
}

#[test]
fn a_host_and_port_entry_matches_that_port_only() {
    // `localhost:3000` is what an operator developing against a local server
    // will write. Parsed as a URL it becomes the scheme `localhost` with the
    // path `3000`, matches nothing, and blocks everything — a failure that
    // looks exactly like a typo in their configuration.
    let allowed = vec!["localhost:3000".to_string()];

    assert!(
        check_allowed(
            &normalize_url("http://localhost:3000/app").unwrap(),
            &allowed
        )
        .is_ok()
    );
    assert!(check_allowed(&normalize_url("http://localhost:3001/").unwrap(), &allowed).is_err());
    assert!(check_allowed(&normalize_url("https://elsewhere.test/").unwrap(), &allowed).is_err());
}

#[test]
fn a_bare_host_entry_admits_any_port() {
    // No port named means the operator did not care which one.
    let allowed = vec!["localhost".to_string()];

    assert!(check_allowed(&normalize_url("http://localhost:3000/").unwrap(), &allowed).is_ok());
    assert!(check_allowed(&normalize_url("http://localhost:9999/").unwrap(), &allowed).is_ok());
}

#[test]
fn an_origin_entry_still_carries_its_port() {
    let allowed = vec!["http://localhost:3000".to_string()];

    assert!(check_allowed(&normalize_url("http://localhost:3000/a").unwrap(), &allowed).is_ok());
    assert!(check_allowed(&normalize_url("http://localhost:3001/a").unwrap(), &allowed).is_err());
}

#[test]
fn about_blank_is_admitted_even_under_an_allowlist() {
    // It is not a destination on the network, and it is how a caller clears the
    // page. A session that could never let go of the last page it loaded would
    // be the opposite of what an allowlist is for.
    let allowed = vec!["https://example.com".to_string()];
    let blank = normalize_url("about:blank").expect("normalizes");

    assert!(check_allowed(&blank, &allowed).is_ok());
}

#[test]
fn a_default_port_matches_an_entry_that_spells_it_out() {
    // `https://example.com/` has no explicit port; the entry names 443.
    let allowed = vec!["example.com:443".to_string()];

    assert!(check_allowed(&normalize_url("https://example.com/").unwrap(), &allowed).is_ok());
    assert!(check_allowed(&normalize_url("http://example.com/").unwrap(), &allowed).is_err());
}
