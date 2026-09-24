use serde_json::json;

use super::admitted;

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
fn malformed_paused_request_fails_closed() {
    assert!(!admitted(&json!({"request":{}}), &allowed()));
    assert!(!admitted(
        &json!({"request":{"url":"not a url"}}),
        &allowed()
    ));
}
