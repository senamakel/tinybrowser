//! Turning `Control+Shift+Enter` into the key events a browser believes.
//!
//! # Why a table and not a passthrough
//!
//! `Input.dispatchKeyEvent` wants four things that have to agree: the `key`
//! (what the character is), the `code` (which physical key it sits on), the
//! legacy `windowsVirtualKeyCode`, and the `text` a keypress inserts. Pages
//! still read all four — a form that submits on Enter usually checks `keyCode`,
//! and a shortcut library usually checks `code` — so a dispatch that fills in
//! only `key` works on some sites and silently does nothing on others.
//!
//! The table below is small because the set of keys an agent presses is small.
//! Anything not in it that is a single character is derived, which covers the
//! rest of typing without enumerating a keyboard layout this module has no way
//! to know.

use crate::error::{Error, Result};

/// The modifier bits `Input.dispatchKeyEvent` expects.
const ALT: u8 = 1;
const CONTROL: u8 = 2;
const META: u8 = 4;
const SHIFT: u8 = 8;

/// One key press, resolved into everything the protocol needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeyStroke {
    /// The modifier bitmask.
    pub(crate) modifiers: u8,
    /// The `key` value, as a `KeyboardEvent` would report it.
    pub(crate) key: String,
    /// The physical key `code`.
    pub(crate) code: String,
    /// The legacy virtual key code.
    pub(crate) key_code: u32,
    /// The text this press inserts, absent for a key that inserts nothing.
    pub(crate) text: Option<String>,
}

/// The named keys, with the four values that have to agree.
const NAMED: &[(&str, &str, &str, u32, Option<&str>)] = &[
    ("enter", "Enter", "Enter", 13, Some("\r")),
    ("tab", "Tab", "Tab", 9, Some("\t")),
    ("escape", "Escape", "Escape", 27, None),
    ("esc", "Escape", "Escape", 27, None),
    ("backspace", "Backspace", "Backspace", 8, None),
    ("delete", "Delete", "Delete", 46, None),
    ("space", " ", "Space", 32, Some(" ")),
    ("arrowup", "ArrowUp", "ArrowUp", 38, None),
    ("arrowdown", "ArrowDown", "ArrowDown", 40, None),
    ("arrowleft", "ArrowLeft", "ArrowLeft", 37, None),
    ("arrowright", "ArrowRight", "ArrowRight", 39, None),
    ("up", "ArrowUp", "ArrowUp", 38, None),
    ("down", "ArrowDown", "ArrowDown", 40, None),
    ("left", "ArrowLeft", "ArrowLeft", 37, None),
    ("right", "ArrowRight", "ArrowRight", 39, None),
    ("home", "Home", "Home", 36, None),
    ("end", "End", "End", 35, None),
    ("pageup", "PageUp", "PageUp", 33, None),
    ("pagedown", "PageDown", "PageDown", 34, None),
];

/// Reads a key or chord such as `Enter`, `a`, or `Control+Shift+Tab`.
///
/// # Errors
///
/// [`Error::InvalidInput`] when the chord is empty, names an unknown modifier,
/// or ends in something that is neither a named key nor a single character.
pub(crate) fn parse(chord: &str) -> Result<KeyStroke> {
    let chord = chord.trim();
    if chord.is_empty() {
        return Err(Error::invalid_input("key is empty"));
    }

    // Split on `+`, but not when `+` *is* the key: `Control++` means control
    // plus the plus key, and splitting naively leaves an empty final part.
    let parts: Vec<&str> = if chord.ends_with("++") {
        let mut parts: Vec<&str> = chord[..chord.len() - 1].split('+').collect();
        parts.pop();
        parts.push("+");
        parts
    } else {
        chord.split('+').collect()
    };

    let (last, modifiers) = parts.split_last().unwrap_or((&"", &[]));

    let mut mask = 0;
    for modifier in modifiers {
        mask |= match modifier.trim().to_ascii_lowercase().as_str() {
            "control" | "ctrl" => CONTROL,
            "shift" => SHIFT,
            "alt" | "option" => ALT,
            "meta" | "command" | "cmd" | "super" => META,
            other => {
                return Err(Error::invalid_input(format!(
                    "{other} is not a modifier; use control, shift, alt, or meta"
                )));
            }
        };
    }

    let key = last.trim();
    if key.is_empty() {
        return Err(Error::invalid_input(format!(
            "{chord} names modifiers but no key"
        )));
    }

    let lowered = key.to_ascii_lowercase();
    if let Some((_, key, code, key_code, text)) =
        NAMED.iter().find(|(name, ..)| *name == lowered)
    {
        return Ok(KeyStroke {
            modifiers: mask,
            key: (*key).to_string(),
            code: (*code).to_string(),
            key_code: *key_code,
            // A modified press inserts nothing: `Control+Enter` submits a form,
            // it does not type a carriage return into it.
            text: (mask & !SHIFT == 0).then(|| (*text)?.to_string()).flatten(),
        });
    }

    let mut characters = key.chars();
    let (Some(character), None) = (characters.next(), characters.next()) else {
        return Err(Error::invalid_input(format!(
            "{key} is not a known key name or a single character"
        )));
    };

    Ok(KeyStroke {
        modifiers: mask,
        key: character.to_string(),
        code: code_for(character),
        key_code: virtual_code(character),
        text: (mask & !SHIFT == 0).then(|| character.to_string()),
    })
}

/// The physical key a character sits on, for the US layout Chrome assumes.
fn code_for(character: char) -> String {
    if character.is_ascii_alphabetic() {
        return format!("Key{}", character.to_ascii_uppercase());
    }
    if character.is_ascii_digit() {
        return format!("Digit{character}");
    }

    match character {
        ' ' => "Space".to_string(),
        '-' => "Minus".to_string(),
        '=' => "Equal".to_string(),
        '.' => "Period".to_string(),
        ',' => "Comma".to_string(),
        '/' => "Slash".to_string(),
        ';' => "Semicolon".to_string(),
        '\'' => "Quote".to_string(),
        '[' => "BracketLeft".to_string(),
        ']' => "BracketRight".to_string(),
        '\\' => "Backslash".to_string(),
        '`' => "Backquote".to_string(),
        // Everything else — an accented letter, a symbol from another layout —
        // gets no code rather than a wrong one. A page that reads `code` will
        // see it as unidentified, which is honest; a page that reads `key`, as
        // most do, is unaffected.
        _ => String::new(),
    }
}

/// The legacy virtual key code a character reports.
fn virtual_code(character: char) -> u32 {
    if character.is_ascii_alphanumeric() {
        return u32::from(character.to_ascii_uppercase() as u8);
    }
    match character {
        ' ' => 32,
        _ => 0,
    }
}
