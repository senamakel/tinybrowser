//! Embedded agent-facing assets for a `TinyBrowser` harness.
//!
//! The browser and Jev controller are capabilities. This crate supplies the
//! small tool schemas and operating guidance a main agent needs to use those
//! capabilities reliably. Keeping them versioned together prevents a globally
//! installed skill from describing tools older or newer than the harness.
//!
//! The primary entry points are [`skill_assets`] and [`tool_schema`].
//!
//! # Example
//!
//! ```
//! use tinybrowser_skills::{skill_assets, tool_schema};
//!
//! assert!(!skill_assets().is_empty());
//! assert!(tool_schema("browser").is_some());
//! ```
//!
//! This crate does not hold browser sessions, provider calls, or credentials.
//! The harness owns those runtime concerns because this crate only packages
//! schemas and operating guidance.

/// The loadable `TinyBrowser` skill in Markdown form.
pub const TINYBROWSER_SKILL: &str = include_str!("../skills/tinybrowser/SKILL.md");

/// JSON Schema for the high-level, Jev-driven task tool.
pub const BROWSER_TASK_SCHEMA: &str = include_str!("../schemas/browser_task.schema.json");

/// JSON Schema for direct browser operations and recovery.
pub const BROWSER_SCHEMA: &str = include_str!("../schemas/browser.schema.json");

/// One file a harness can install into its skill directory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SkillAsset {
    /// Path relative to the harness's skill or asset root.
    pub path: &'static str,
    /// Complete UTF-8 file contents.
    pub contents: &'static str,
}

const ASSETS: &[SkillAsset] = &[
    SkillAsset {
        path: "tinybrowser/SKILL.md",
        contents: TINYBROWSER_SKILL,
    },
    SkillAsset {
        path: "tinybrowser/schemas/browser_task.schema.json",
        contents: BROWSER_TASK_SCHEMA,
    },
    SkillAsset {
        path: "tinybrowser/schemas/browser.schema.json",
        contents: BROWSER_SCHEMA,
    },
];

/// Every file in the loadable `TinyBrowser` skill package.
#[must_use]
pub const fn skill_assets() -> &'static [SkillAsset] {
    ASSETS
}

/// Return the JSON Schema for one `TinyBrowser` harness tool.
#[must_use]
pub fn tool_schema(name: &str) -> Option<&'static str> {
    match name {
        "browser_task" => Some(BROWSER_TASK_SCHEMA),
        "browser" => Some(BROWSER_SCHEMA),
        _ => None,
    }
}

#[cfg(test)]
mod test {
    //! Contract tests for the embedded skill package.

    #![allow(clippy::expect_used)]

    use super::*;

    #[test]
    fn every_tool_schema_is_valid_json_with_its_tool_name() {
        for (name, schema) in [
            ("browser_task", BROWSER_TASK_SCHEMA),
            ("browser", BROWSER_SCHEMA),
        ] {
            let value: serde_json::Value = serde_json::from_str(schema).expect("valid schema JSON");
            assert_eq!(value["name"], name);
            assert_eq!(value["input_schema"]["type"], "object");
        }
    }

    #[test]
    fn skill_names_both_tools_and_the_confirmation_boundary() {
        assert!(TINYBROWSER_SKILL.contains("`browser_task`"));
        assert!(TINYBROWSER_SKILL.contains("`browser`"));
        assert!(TINYBROWSER_SKILL.contains("confirmation"));
    }

    #[test]
    fn package_paths_are_relative_and_unique() {
        let paths = skill_assets()
            .iter()
            .map(|asset| asset.path)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(paths.len(), skill_assets().len());
        assert!(paths.iter().all(|path| !path.starts_with('/')));
    }

    #[test]
    fn an_unknown_tool_has_no_schema() {
        assert_eq!(tool_schema("unknown"), None);
    }
}
