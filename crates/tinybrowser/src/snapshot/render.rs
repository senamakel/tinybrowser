//! Turning an accessibility tree into the indented text an agent reads, and the
//! refs that make it actionable.
//!
//! # Why this is separate from the protocol
//!
//! Everything in this file is a pure function of a node list. That is
//! deliberate: rendering is where the judgement lives — which roles are worth a
//! ref, what "interactive only" means, when a container is noise — and
//! judgement that can only be exercised against a live browser is judgement that
//! does not get tested. The socket hands over a `Vec<AxNode>`, and from there it
//! is all arithmetic.
//!
//! # What the output looks like
//!
//! ```text
//! - document "Example Domain"
//!   - heading "Example Domain" @e1
//!   - paragraph "This domain is for use in illustrative examples."
//!   - link "More information..." @e2
//! ```
//!
//! Two spaces per level, one line per node, `role "name"` and a `@ref` on
//! anything addressable. It is YAML-ish rather than YAML on purpose: an agent
//! reads it as a list, and nothing downstream should be tempted to parse it when
//! [`tinybrowser_bus::Snapshot::refs`] carries the same information structurally.

use std::collections::HashMap;

use tinybrowser_bus::{ElementRef, SnapshotRequest};

use super::types::AxNode;

/// Roles an agent can act on. These get a ref.
const INTERACTIVE_ROLES: &[&str] = &[
    "button",
    "checkbox",
    "combobox",
    "link",
    "listbox",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "option",
    "radio",
    "searchbox",
    "slider",
    "spinbutton",
    "switch",
    "tab",
    "textbox",
    "treeitem",
    "Iframe",
];

/// Roles that carry meaning without being actionable. These get a ref too — an
/// agent reads text out of them — but they are dropped by `interactive_only`.
const CONTENT_ROLES: &[&str] = &[
    "article",
    "cell",
    "columnheader",
    "gridcell",
    "heading",
    "img",
    "listitem",
    "main",
    "navigation",
    "paragraph",
    "region",
    "rowheader",
    "StaticText",
];

/// Roles that exist to hold other nodes. `compact` drops them, keeping their
/// children at the parent's depth.
const STRUCTURAL_ROLES: &[&str] = &[
    "application",
    "directory",
    "document",
    "generic",
    "grid",
    "group",
    "list",
    "menu",
    "menubar",
    "none",
    "presentation",
    "row",
    "rowgroup",
    "table",
    "tablist",
    "toolbar",
    "tree",
    "treegrid",
    "RootWebArea",
    "WebArea",
];

/// A rendered snapshot: the text, the refs in it, and the map that resolves
/// them.
#[derive(Debug, Default)]
pub(crate) struct Rendered {
    /// The indented tree.
    pub(crate) tree: String,
    /// Every ref in the tree, in the order it appears.
    pub(crate) refs: Vec<ElementRef>,
    /// Ref id to backend node id, for [`crate::session::refs::RefMap`].
    pub(crate) nodes: HashMap<String, i64>,
    /// Whether `max_chars` cut the tree short.
    pub(crate) truncated: bool,
}

/// Renders `nodes` according to `request`.
///
/// `nodes` is the tree as `Accessibility.getFullAXTree` returned it: a flat
/// list in which parents name their children by id.
pub(crate) fn render(nodes: &[AxNode], request: &SnapshotRequest) -> Rendered {
    let by_id: HashMap<&str, &AxNode> = nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect();

    let Some(root) = nodes.iter().find(|node| !node.ignored) else {
        return Rendered::default();
    };

    let mut state = Walk {
        by_id,
        request,
        next_ref: 1,
        lines: Vec::new(),
        refs: Vec::new(),
        nodes: HashMap::new(),
        // A malformed tree — one whose child ids form a cycle — would otherwise
        // walk forever. The visited set costs one hash per node and removes a
        // hang from the list of things a hostile page can cause.
        visited: std::collections::HashSet::new(),
    };
    state.walk(root, 0);

    let mut tree = state.lines.join("\n");
    let truncated = tree.chars().count() > request.max_chars;
    if truncated {
        tree = tree.chars().take(request.max_chars).collect();
    }

    Rendered {
        tree,
        refs: state.refs,
        nodes: state.nodes,
        truncated,
    }
}

/// The state carried down one traversal.
struct Walk<'a> {
    by_id: HashMap<&'a str, &'a AxNode>,
    request: &'a SnapshotRequest,
    next_ref: u32,
    lines: Vec<String>,
    refs: Vec<ElementRef>,
    nodes: HashMap<String, i64>,
    visited: std::collections::HashSet<&'a str>,
}

impl<'a> Walk<'a> {
    /// Emits `node` and everything under it at `depth`.
    fn walk(&mut self, node: &'a AxNode, depth: usize) {
        if !self.visited.insert(node.node_id.as_str()) {
            return;
        }
        if self
            .request
            .depth
            .is_some_and(|limit| depth > limit as usize)
        {
            return;
        }

        // An ignored node is not in the accessibility tree at all, but its
        // children may be: an `aria-hidden` wrapper around visible content is
        // exactly this shape. Descending without emitting keeps them reachable
        // at the parent's depth rather than orphaning them.
        let emit = !node.ignored && self.keeps(node);
        let child_depth = if emit { depth + 1 } else { depth };

        if emit {
            let reference = self.mint(node);
            self.lines
                .push(self.line(node, depth, reference.as_deref()));
        }

        for child_id in &node.child_ids {
            if let Some(child) = self.by_id.get(child_id.as_str()).copied() {
                self.walk(child, child_depth);
            }
        }
    }

    /// Whether this node survives the request's filters.
    fn keeps(&self, node: &AxNode) -> bool {
        let role = node.role();
        if role.is_empty() {
            return false;
        }

        if self.request.interactive_only && !is_interactive(&role) {
            return false;
        }

        // `compact` drops the scaffolding even when it is named: a document, a
        // list, or a table wrapper tells an agent nothing it cannot see from the
        // items inside it, and on a real page they are most of the lines.
        if self.request.compact && is_structural(&role) {
            return false;
        }

        // A node with neither a name nor a value nor a reason to be acted on
        // contributes an empty line and nothing else.
        is_interactive(&role)
            || !node.name.as_text().trim().is_empty()
            || !node.value.as_text().trim().is_empty()
    }

    /// Gives `node` a ref when it is one an agent could address.
    fn mint(&mut self, node: &AxNode) -> Option<String> {
        let backend = node.backend_dom_node_id?;
        let role = node.role();
        if !is_interactive(&role) && !is_content(&role) {
            return None;
        }

        let id = format!("e{}", self.next_ref);
        self.next_ref += 1;
        self.refs.push(ElementRef {
            id: id.clone(),
            role,
            name: clean(&node.name.as_text()),
        });
        self.nodes.insert(id.clone(), backend);
        Some(id)
    }

    /// One rendered line.
    fn line(&self, node: &AxNode, depth: usize, reference: Option<&str>) -> String {
        let mut line = format!("{}- {}", "  ".repeat(depth), node.role());

        let name = clean(&node.name.as_text());
        if !name.is_empty() {
            line.push_str(&format!(" \"{name}\""));
        }

        let value = clean(&node.value.as_text());
        if !value.is_empty() && value != name {
            line.push_str(&format!(" value=\"{value}\""));
        }

        for (property, label) in [("checked", "checked"), ("expanded", "expanded")] {
            if let Some(state) = node.property(property) {
                let rendered = state.as_text();
                if !rendered.is_empty() && rendered != "false" {
                    line.push_str(&format!(" {label}={rendered}"));
                }
            }
        }

        // Reported only when true: an agent needs to know a control is disabled,
        // and rendering `disabled=false` on every other control would triple the
        // size of the tree to say nothing.
        for property in ["disabled", "required", "selected"] {
            if node.property(property).and_then(AxValueExt::truthy) == Some(true) {
                line.push_str(&format!(" {property}"));
            }
        }

        if self.request.include_urls
            && let Some(url) = node.property("url").map(|value| clean(&value.as_text()))
            && !url.is_empty()
        {
            line.push_str(&format!(" url=\"{url}\""));
        }

        if let Some(reference) = reference {
            line.push_str(&format!(" @{reference}"));
        }

        line
    }
}

/// Reading a property as a boolean, whichever way Chrome encoded it.
trait AxValueExt {
    /// The property as a boolean, when it is one.
    fn truthy(&self) -> Option<bool>;
}

impl AxValueExt for super::types::AxValue {
    fn truthy(&self) -> Option<bool> {
        self.as_bool().or_else(|| match self.as_text().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        })
    }
}

/// Collapses whitespace and strips the invisible characters that make an
/// accessible name look mangled in a terminal.
///
/// A name is a screen reader's announcement, and pages routinely build them out
/// of zero-width joiners, non-breaking spaces, and newlines from the markup. An
/// agent comparing "Add to cart" against `Add\u{00A0}to cart` finds nothing.
pub(crate) fn clean(raw: &str) -> String {
    const INVISIBLE: &[char] = &['\u{FEFF}', '\u{200B}', '\u{200C}', '\u{200D}', '\u{2060}'];

    raw.chars()
        .map(|character| {
            if character == '\u{00A0}' {
                ' '
            } else {
                character
            }
        })
        .filter(|character| !INVISIBLE.contains(character))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether a role is one an agent acts on.
pub(crate) fn is_interactive(role: &str) -> bool {
    INTERACTIVE_ROLES.contains(&role)
}

/// Whether a role carries content without being actionable.
fn is_content(role: &str) -> bool {
    CONTENT_ROLES.contains(&role)
}

/// Whether a role exists only to hold other nodes.
fn is_structural(role: &str) -> bool {
    STRUCTURAL_ROLES.contains(&role)
}
