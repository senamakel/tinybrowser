//! Tests for the parts of interaction that need no browser.
//!
//! Key parsing and target description are pure, and both are places where a
//! quiet mistake produces an action that runs, reports success, and does the
//! wrong thing — a chord dispatched without its modifier, an error naming a
//! target nobody recognises. The dispatch itself is exercised by the
//! `live-chrome` suite.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tinybrowser_bus::{LocateBy, Locator, Target};

use super::keys::parse;
use super::resolve::{describe_locator, describe_target, dimension};
use crate::error::Error;

#[test]
fn a_named_key_carries_all_four_values() {
    // A page that submits on Enter usually reads `keyCode`; a shortcut library
    // usually reads `code`. Filling in only `key` works on some sites and does
    // nothing on others.
    let stroke = parse("Enter").expect("parses");

    assert_eq!(stroke.key, "Enter");
    assert_eq!(stroke.code, "Enter");
    assert_eq!(stroke.key_code, 13);
    assert_eq!(stroke.text.as_deref(), Some("\r"));
    assert_eq!(stroke.modifiers, 0);
}

#[test]
fn key_names_are_matched_case_insensitively() {
    assert_eq!(
        parse("enter").expect("parses"),
        parse("Enter").expect("parses")
    );
    assert_eq!(parse("ESCAPE").expect("parses").key, "Escape");
}

#[test]
fn the_short_arrow_names_are_accepted() {
    assert_eq!(parse("down").expect("parses").key, "ArrowDown");
    assert_eq!(parse("ArrowDown").expect("parses").key, "ArrowDown");
}

#[test]
fn a_single_character_is_derived_rather_than_enumerated() {
    let stroke = parse("a").expect("parses");

    assert_eq!(stroke.key, "a");
    assert_eq!(stroke.code, "KeyA");
    assert_eq!(stroke.key_code, u32::from(b'A'));
    assert_eq!(stroke.text.as_deref(), Some("a"));
}

#[test]
fn digits_and_punctuation_get_their_physical_key() {
    assert_eq!(parse("1").expect("parses").code, "Digit1");
    assert_eq!(parse("-").expect("parses").code, "Minus");
    assert_eq!(parse("/").expect("parses").code, "Slash");
}

#[test]
fn a_character_outside_the_us_layout_gets_no_code_rather_than_a_wrong_one() {
    let stroke = parse("é").expect("parses");

    assert_eq!(stroke.key, "é");
    assert!(stroke.code.is_empty());
}

#[test]
fn modifiers_accumulate_into_the_bitmask() {
    let stroke = parse("Control+Shift+Tab").expect("parses");

    // Control is 2 and Shift is 8.
    assert_eq!(stroke.modifiers, 10);
    assert_eq!(stroke.key, "Tab");
}

#[test]
fn the_modifier_aliases_people_actually_type_are_accepted() {
    assert_eq!(parse("ctrl+a").expect("parses").modifiers, 2);
    assert_eq!(parse("cmd+a").expect("parses").modifiers, 4);
    assert_eq!(parse("command+a").expect("parses").modifiers, 4);
    assert_eq!(parse("option+a").expect("parses").modifiers, 1);
}

#[test]
fn a_modified_press_inserts_no_text() {
    // `Control+Enter` submits a form. If it also inserted a carriage return,
    // every shortcut would type into the field it was meant to act on.
    assert!(parse("Control+Enter").expect("parses").text.is_none());
    assert!(parse("Control+a").expect("parses").text.is_none());
}

#[test]
fn shift_alone_still_inserts_text() {
    // Shift is how capital letters are typed; treating it like the other
    // modifiers would make `Shift+a` insert nothing.
    assert_eq!(parse("Shift+a").expect("parses").text.as_deref(), Some("a"));
}

#[test]
fn a_plus_key_is_not_mistaken_for_a_separator() {
    let stroke = parse("Control++").expect("parses");

    assert_eq!(stroke.modifiers, 2);
    assert_eq!(stroke.key, "+");
}

#[test]
fn an_empty_key_is_refused() {
    let error = parse("   ").expect_err("refused");
    assert!(matches!(error, Error::InvalidInput { .. }), "{error}");
}

#[test]
fn modifiers_without_a_key_are_refused() {
    let error = parse("Control+").expect_err("refused");
    assert!(matches!(error, Error::InvalidInput { .. }), "{error}");
}

#[test]
fn an_unknown_modifier_is_refused_by_name() {
    let error = parse("Hyper+a").expect_err("refused");

    assert!(matches!(error, Error::InvalidInput { .. }), "{error}");
    assert!(error.to_string().contains("hyper"));
}

#[test]
fn a_multi_character_name_that_is_not_a_key_is_refused() {
    // Otherwise `parse("Retrun")` would silently dispatch nothing useful.
    let error = parse("Retrun").expect_err("refused");
    assert!(matches!(error, Error::InvalidInput { .. }), "{error}");
}

#[test]
fn the_locator_dimensions_match_what_the_page_script_switches_on() {
    // These strings cross into JavaScript. A rename on one side alone produces
    // "unknown locator dimension" at runtime and nothing at compile time.
    assert_eq!(dimension(LocateBy::Role), "role");
    assert_eq!(dimension(LocateBy::TestId), "test_id");
    assert_eq!(dimension(LocateBy::AltText), "alt_text");
    assert_eq!(dimension(LocateBy::Placeholder), "placeholder");
}

#[test]
fn a_target_describes_itself_the_way_the_caller_named_it() {
    assert_eq!(describe_target(&Target::parse("@e12")), "@e12");
    assert_eq!(describe_target(&Target::parse("#submit")), "#submit");
}

#[test]
fn a_locator_describes_itself_with_everything_that_narrowed_it() {
    let locator = Locator {
        index: 2,
        ..Locator::new(LocateBy::Role, "button").with_name("Submit")
    };

    let described = describe_locator(&locator);
    assert!(described.contains("role"));
    assert!(described.contains("button"));
    assert!(described.contains("Submit"));
    assert!(described.contains("index 2"));
}

#[test]
fn a_first_match_locator_does_not_mention_its_index() {
    let described = describe_locator(&Locator::new(LocateBy::Text, "Sign in"));

    assert!(!described.contains("index"));
    assert!(described.contains("Sign in"));
}

#[test]
fn every_punctuation_key_on_the_us_layout_gets_its_physical_key() {
    // A page reading `code` for a shortcut — a great many do — sees the wrong
    // key otherwise, and the failure is silent.
    for (character, code) in [
        (" ", "Space"),
        ("-", "Minus"),
        ("=", "Equal"),
        (".", "Period"),
        (",", "Comma"),
        (";", "Semicolon"),
        ("'", "Quote"),
        ("[", "BracketLeft"),
        ("]", "BracketRight"),
        ("\\", "Backslash"),
        ("`", "Backquote"),
    ] {
        assert_eq!(parse(character).expect("parses").code, code, "{character}");
    }
}

#[test]
fn a_space_carries_the_virtual_code_a_page_expects() {
    let stroke = parse(" ").expect("parses");

    assert_eq!(stroke.key_code, 32);
    assert_eq!(stroke.text.as_deref(), Some(" "));
}

#[test]
fn punctuation_reports_no_virtual_code_rather_than_a_wrong_one() {
    // There is no correct legacy code for these without knowing the layout, and
    // a plausible-looking wrong one is worse than an unidentified key.
    assert_eq!(parse("-").expect("parses").key_code, 0);
}

#[test]
fn the_named_space_and_the_character_agree() {
    assert_eq!(parse("space").expect("parses"), parse(" ").expect("parses"));
}

#[test]
fn every_named_key_is_reachable_by_its_name() {
    for name in [
        "enter",
        "tab",
        "escape",
        "esc",
        "backspace",
        "delete",
        "space",
        "arrowup",
        "arrowdown",
        "arrowleft",
        "arrowright",
        "up",
        "down",
        "left",
        "right",
        "home",
        "end",
        "pageup",
        "pagedown",
    ] {
        let stroke = parse(name).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert!(!stroke.key.is_empty(), "{name} parsed to an empty key");
    }
}

#[test]
fn a_key_without_inserted_text_stays_that_way() {
    // Escape and the arrows type nothing; giving them `text` would insert a
    // character every time an agent moved the cursor.
    for name in ["Escape", "ArrowDown", "Backspace", "Delete", "Home"] {
        assert!(parse(name).expect("parses").text.is_none(), "{name}");
    }
}
