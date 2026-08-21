//! The shape of an accessibility node as Chrome reports it.
//!
//! Only the fields this crate reads are modelled. `Accessibility.getFullAXTree`
//! returns considerably more — every ARIA property, source annotations for each
//! computed value, relationship edges — and deserializing it all would cost a
//! parse of a document-sized payload for fields nothing looks at.

use serde::Deserialize;

/// A computed accessibility value: Chrome wraps every one in an object.
#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct AxValue {
    /// The value itself, absent when the property is not computed for this node.
    pub(crate) value: Option<serde_json::Value>,
}

impl AxValue {
    /// The value as a string, empty when it is absent or not a string.
    pub(crate) fn as_text(&self) -> String {
        match &self.value {
            Some(serde_json::Value::String(text)) => text.clone(),
            Some(other) => other.to_string(),
            None => String::new(),
        }
    }

    /// The value as a boolean, when it is one.
    pub(crate) fn as_bool(&self) -> Option<bool> {
        self.value.as_ref().and_then(serde_json::Value::as_bool)
    }
}

/// One node of the accessibility tree.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AxNode {
    /// The node's identity within the tree.
    pub(crate) node_id: String,
    /// Whether the node is excluded from the accessibility tree. Ignored nodes
    /// are kept in the response so the tree stays connected, and dropped here.
    #[serde(default)]
    pub(crate) ignored: bool,
    /// The computed role, absent on ignored nodes.
    #[serde(default)]
    pub(crate) role: AxValue,
    /// The computed accessible name.
    #[serde(default)]
    pub(crate) name: AxValue,
    /// The current value of a field, slider, or similar.
    #[serde(default)]
    pub(crate) value: AxValue,
    /// The computed ARIA properties worth rendering.
    #[serde(default)]
    pub(crate) properties: Vec<AxProperty>,
    /// The identities of this node's children, in document order.
    #[serde(default)]
    pub(crate) child_ids: Vec<String>,
    /// The DOM node behind this one, absent for nodes with no element — a text
    /// run inside a paragraph, for instance. A node without one cannot be given
    /// a ref, because there is nothing to act on.
    #[serde(default)]
    pub(crate) backend_dom_node_id: Option<i64>,
}

/// One computed ARIA property.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct AxProperty {
    /// The property name, such as `checked` or `disabled`.
    pub(crate) name: String,
    /// Its computed value.
    #[serde(default)]
    pub(crate) value: AxValue,
}

impl AxNode {
    /// The node's role, or an empty string.
    pub(crate) fn role(&self) -> String {
        self.role.as_text()
    }

    /// The value of a named property, when the node has one.
    pub(crate) fn property(&self, name: &str) -> Option<&AxValue> {
        self.properties
            .iter()
            .find(|property| property.name == name)
            .map(|property| &property.value)
    }
}
