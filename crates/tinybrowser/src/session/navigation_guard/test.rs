use serde_json::json;

use super::{admitted, request_id};
use crate::cdp::CdpEvent;

fn allowed() -> Vec<String> {
    vec![".selenium.dev".to_owned()]
}

#[test]
fn admits_allowed_document_and_subdomain() {
    assert!(admitted(
        &json!({"request":{"url":"https://selenium.dev/a"}}),
        &allowed()
    ));
    assert!(admitted(
        &json!({"request":{"url":"https://www.selenium.dev/a"}}),
        &allowed()
    ));
}

#[test]
fn blocks_redirects_to_other_domains_and_lookalikes() {
    assert!(!admitted(
        &json!({"request":{"url":"https://example.com/"}}),
        &allowed()
    ));
    assert!(!admitted(
        &json!({"request":{"url":"https://evilselenium.dev/"}}),
        &allowed()
    ));
}

#[test]
fn scheme_qualified_host_tree_blocks_http_before_continue() {
    let allowed = vec!["https://.selenium.dev".to_owned()];
    assert!(admitted(
        &json!({"request":{"url":"https://www.selenium.dev/form"}}),
        &allowed,
    ));
    assert!(!admitted(
        &json!({"request":{"url":"http://www.selenium.dev/form"}}),
        &allowed,
    ));
}

#[test]
fn malformed_paused_request_fails_closed() {
    assert!(!admitted(&json!({"request":{}}), &allowed()));
    assert!(!admitted(
        &json!({"request":{"url":"not a url"}}),
        &allowed()
    ));
}

#[test]
fn only_this_pages_well_formed_paused_requests_are_handled() {
    let mut event = CdpEvent {
        method: "Fetch.requestPaused".into(),
        session_id: Some("page-a".into()),
        params: json!({"requestId":"request-1"}),
    };
    assert_eq!(request_id(&event, "page-a"), Some("request-1"));
    assert_eq!(request_id(&event, "page-b"), None);
    event.method = "Page.loadEventFired".into();
    assert_eq!(request_id(&event, "page-a"), None);
    event.method = "Fetch.requestPaused".into();
    event.params = json!({});
    assert_eq!(request_id(&event, "page-a"), None);
}
