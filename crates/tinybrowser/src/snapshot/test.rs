//! Tests for snapshot rendering.
//!
//! Rendering is pure, so all of it is testable from a node list. These fix the
//! output an agent reads: the indentation, where a ref appears, and which nodes
//! each filter drops. A change to any of those changes what every agent sees, so
//! it should have to be made here too.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
use tinybrowser_bus::SnapshotRequest;

use super::render::{clean, is_interactive, render};
use super::types::AxNode;

/// A small page: a heading, a paragraph, a link, and a disabled button, inside
/// the generic wrappers a real document is full of.
fn page() -> Vec<AxNode> {
    nodes(json!([
        {
            "nodeId": "1",
            "role": { "value": "RootWebArea" },
            "name": { "value": "Example Domain" },
            "childIds": ["2"],
            "backendDOMNodeId": 1,
        },
        {
            "nodeId": "2",
            "role": { "value": "generic" },
            "name": { "value": "" },
            "childIds": ["3", "4", "5", "6"],
            "backendDOMNodeId": 2,
        },
        {
            "nodeId": "3",
            "role": { "value": "heading" },
            "name": { "value": "Example Domain" },
            "childIds": [],
            "backendDOMNodeId": 3,
        },
        {
            "nodeId": "4",
            "role": { "value": "paragraph" },
            "name": { "value": "This domain is for use in examples." },
            "childIds": [],
            "backendDOMNodeId": 4,
        },
        {
            "nodeId": "5",
            "role": { "value": "link" },
            "name": { "value": "More information" },
            "properties": [{ "name": "url", "value": { "value": "https://iana.org/domains" } }],
            "childIds": [],
            "backendDOMNodeId": 5,
        },
        {
            "nodeId": "6",
            "role": { "value": "button" },
            "name": { "value": "Submit" },
            "properties": [{ "name": "disabled", "value": { "value": true } }],
            "childIds": [],
            "backendDOMNodeId": 6,
        },
    ]))
}

fn nodes(value: serde_json::Value) -> Vec<AxNode> {
    serde_json::from_value(value).expect("the fixture parses")
}

#[test]
fn a_tree_renders_as_indented_roles_and_names() {
    let rendered = render(&page(), &SnapshotRequest::default());

    assert!(rendered.tree.starts_with("- RootWebArea \"Example Domain\""));
    assert!(rendered.tree.contains("\n  - heading \"Example Domain\""));
    assert!(rendered.tree.contains("\n  - link \"More information\""));
}

#[test]
fn addressable_nodes_carry_a_ref_and_unaddressable_ones_do_not() {
    let rendered = render(&page(), &SnapshotRequest::default());

    assert!(rendered.tree.contains("- link \"More information\" @"));
    // The generic wrapper is neither interactive nor content: there is nothing
    // an agent would do with a ref to it.
    assert!(!rendered.tree.contains("- generic"));
}

#[test]
fn refs_are_minted_in_document_order() {
    let rendered = render(&page(), &SnapshotRequest::default());
    let ids: Vec<&str> = rendered.refs.iter().map(|r| r.id.as_str()).collect();

    assert_eq!(ids.first(), Some(&"e1"));
    assert_eq!(
        rendered
            .refs
            .iter()
            .find(|r| r.role == "link")
            .map(|r| r.name.as_str()),
        Some("More information")
    );
}

#[test]
fn every_ref_resolves_to_a_backend_node() {
    let rendered = render(&page(), &SnapshotRequest::default());

    for element in &rendered.refs {
        assert!(
            rendered.nodes.contains_key(&element.id),
            "@{} appears in the tree with nothing to resolve to",
            element.id
        );
    }
    assert_eq!(rendered.nodes.len(), rendered.refs.len());
}

#[test]
fn interactive_only_keeps_the_controls_and_drops_the_prose() {
    let rendered = render(&page(), &SnapshotRequest::interactive());

    assert!(rendered.tree.contains("- link \"More information\""));
    assert!(rendered.tree.contains("- button \"Submit\""));
    assert!(!rendered.tree.contains("paragraph"));
    assert!(!rendered.tree.contains("heading"));
}

#[test]
fn compact_drops_unnamed_containers() {
    let request = SnapshotRequest {
        compact: true,
        ..SnapshotRequest::default()
    };
    let rendered = render(&page(), &request);

    assert!(!rendered.tree.contains("RootWebArea"));
    assert!(rendered.tree.contains("- heading \"Example Domain\""));
}

#[test]
fn a_disabled_control_says_so() {
    // An agent that cannot see this clicks the button, gets no error, and
    // reports that it submitted the form.
    let rendered = render(&page(), &SnapshotRequest::default());
    assert!(rendered.tree.contains("- button \"Submit\" disabled"));
}

#[test]
fn state_that_is_false_is_not_rendered() {
    let tree = nodes(json!([{
        "nodeId": "1",
        "role": { "value": "checkbox" },
        "name": { "value": "Remember me" },
        "properties": [
            { "name": "checked", "value": { "value": "false" } },
            { "name": "disabled", "value": { "value": false } },
        ],
        "childIds": [],
        "backendDOMNodeId": 1,
    }]));
    let rendered = render(&tree, &SnapshotRequest::default());

    assert!(rendered.tree.contains("- checkbox \"Remember me\""));
    assert!(!rendered.tree.contains("checked"));
    assert!(!rendered.tree.contains("disabled"));
}

#[test]
fn urls_are_annotated_only_when_asked_for() {
    let without = render(&page(), &SnapshotRequest::default());
    assert!(!without.tree.contains("iana.org"));

    let with = render(
        &page(),
        &SnapshotRequest {
            include_urls: true,
            ..SnapshotRequest::default()
        },
    );
    assert!(with.tree.contains("url=\"https://iana.org/domains\""));
}

#[test]
fn depth_stops_the_walk() {
    let request = SnapshotRequest {
        depth: Some(0),
        ..SnapshotRequest::default()
    };
    let rendered = render(&page(), &request);

    assert!(rendered.tree.contains("RootWebArea"));
    assert!(!rendered.tree.contains("heading"));
}

#[test]
fn max_chars_truncates_and_says_so() {
    let request = SnapshotRequest {
        max_chars: 20,
        ..SnapshotRequest::default()
    };
    let rendered = render(&page(), &request);

    assert!(rendered.truncated);
    assert_eq!(rendered.tree.chars().count(), 20);
}

#[test]
fn an_ignored_node_is_skipped_but_its_children_are_not() {
    // An `aria-hidden` wrapper around visible content is exactly this shape.
    // Dropping the subtree with the wrapper would lose the content entirely.
    let tree = nodes(json!([
        {
            "nodeId": "1",
            "role": { "value": "RootWebArea" },
            "name": { "value": "Page" },
            "childIds": ["2"],
            "backendDOMNodeId": 1,
        },
        {
            "nodeId": "2",
            "ignored": true,
            "childIds": ["3"],
            "backendDOMNodeId": 2,
        },
        {
            "nodeId": "3",
            "role": { "value": "button" },
            "name": { "value": "Buried" },
            "childIds": [],
            "backendDOMNodeId": 3,
        },
    ]));
    let rendered = render(&tree, &SnapshotRequest::default());

    assert!(rendered.tree.contains("- button \"Buried\""));
}

#[test]
fn a_cyclic_tree_terminates() {
    // Nothing should be able to make a snapshot hang: a page controls the tree
    // this walks, and a hang is a wedged session a host cannot recover.
    let tree = nodes(json!([
        {
            "nodeId": "1",
            "role": { "value": "RootWebArea" },
            "name": { "value": "Loop" },
            "childIds": ["2"],
            "backendDOMNodeId": 1,
        },
        {
            "nodeId": "2",
            "role": { "value": "button" },
            "name": { "value": "Round" },
            "childIds": ["1"],
            "backendDOMNodeId": 2,
        },
    ]));
    let rendered = render(&tree, &SnapshotRequest::default());

    assert!(rendered.tree.contains("- button \"Round\""));
}

#[test]
fn an_empty_tree_renders_to_nothing_rather_than_failing() {
    let rendered = render(&[], &SnapshotRequest::default());

    assert!(rendered.tree.is_empty());
    assert!(rendered.refs.is_empty());
    assert!(!rendered.truncated);
}

#[test]
fn names_are_cleaned_of_invisible_characters() {
    // Pages build accessible names out of non-breaking spaces and zero-width
    // joiners; an agent matching on "Add to cart" finds nothing otherwise.
    assert_eq!(clean("Add\u{00A0}to\u{200B} cart"), "Add to cart");
    assert_eq!(clean("  spread   over\nlines  "), "spread over lines");
    assert_eq!(clean(""), "");
}

#[test]
fn the_interactive_roles_are_the_ones_an_agent_can_act_on() {
    for role in ["button", "link", "textbox", "checkbox", "combobox"] {
        assert!(is_interactive(role), "{role} should be interactive");
    }
    for role in ["paragraph", "heading", "generic"] {
        assert!(!is_interactive(role), "{role} should not be interactive");
    }
}
