//! Key chord parsing and matching shared by the keymap engine and the
//! client-only raw keys in config.toml.

#[cfg(test)]
use crossterm::event::KeyEvent;
use crossterm::event::{KeyCode, KeyModifiers};

use crate::input::TerminalKey;

pub type KeyCombo = (KeyCode, KeyModifiers);

/// Zero-based index of a `1..9` chord when `key` presses `combo`, accepting
/// the shifted-number symbols that legacy terminals report for shifted digits.
pub(crate) fn indexed_key_index(key: &TerminalKey, combo: KeyCombo) -> Option<usize> {
    let (expected_code, _) = normalize_key_combo(combo);
    let KeyCode::Char(key_number @ '1'..='9') = expected_code else {
        return None;
    };
    let legacy_shifted_number = matches!(key.code, KeyCode::Char(c)
        if shifted_number_symbol(c) == Some(key_number)
            && indexed_shifted_number_matches(key, combo, key_number));
    if terminal_key_matches_combo(key, combo) || legacy_shifted_number {
        Some((key_number as usize) - ('1' as usize))
    } else {
        None
    }
}

pub fn format_key_combo(binding: KeyCombo) -> String {
    let (code, modifiers) = binding;
    let mut parts = Vec::new();
    if modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("ctrl".to_string());
    }
    if modifiers.contains(KeyModifiers::ALT) {
        parts.push("alt".to_string());
    }
    if modifiers.contains(KeyModifiers::SHIFT) && !matches!(code, KeyCode::BackTab) {
        parts.push("shift".to_string());
    }
    if modifiers.contains(KeyModifiers::SUPER) {
        parts.push(super_modifier_label().to_string());
    }
    if modifiers.contains(KeyModifiers::HYPER) {
        parts.push("hyper".to_string());
    }
    if modifiers.contains(KeyModifiers::META) {
        parts.push("meta".to_string());
    }

    let key = match code {
        KeyCode::Char(' ') => "space".to_string(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "enter".to_string(),
        KeyCode::Esc => "esc".to_string(),
        KeyCode::Tab => "tab".to_string(),
        KeyCode::BackTab => "shift+tab".to_string(),
        KeyCode::Backspace => "backspace".to_string(),
        KeyCode::Left => "left".to_string(),
        KeyCode::Right => "right".to_string(),
        KeyCode::Up => "up".to_string(),
        KeyCode::Down => "down".to_string(),
        KeyCode::F(n) => format!("f{n}"),
        _ => format!("{:?}", code).to_lowercase(),
    };

    if matches!(code, KeyCode::BackTab) {
        return if parts.is_empty() {
            key
        } else {
            format!("{}+{key}", parts.join("+"))
        };
    }

    parts.push(key);
    parts.join("+")
}

fn super_modifier_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "super"
    }
}

fn parse_modifier_token(token: &str) -> Option<KeyModifiers> {
    match token.to_lowercase().as_str() {
        "ctrl" | "control" => Some(KeyModifiers::CONTROL),
        "shift" => Some(KeyModifiers::SHIFT),
        "alt" | "option" | "meta" => Some(KeyModifiers::ALT),
        "cmd" | "command" | "super" => Some(KeyModifiers::SUPER),
        "hyper" => Some(KeyModifiers::HYPER),
        _ => None,
    }
}

pub(crate) fn parse_range_modifiers(s: &str) -> Option<KeyModifiers> {
    let mut modifiers = KeyModifiers::empty();
    let mut saw_range = false;
    for part in s.split('+') {
        let trimmed = part.trim();
        if trimmed == "1..9" {
            if saw_range {
                return None;
            }
            saw_range = true;
        } else {
            modifiers |= parse_modifier_token(trimmed)?;
        }
    }
    saw_range.then_some(modifiers)
}

pub(crate) fn parse_key_combo(s: &str) -> Option<KeyCombo> {
    let parts: Vec<&str> = s.split('+').collect();
    let mut modifiers = KeyModifiers::empty();
    let mut key_str: Option<&str> = None;

    for part in &parts {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Some(modifier) = parse_modifier_token(trimmed) {
            modifiers |= modifier;
        } else if key_str.is_some() {
            return None;
        } else {
            key_str = Some(trimmed);
        }
    }

    let key_str = key_str?;
    let single_char = single_key_char(key_str);
    let lower = key_str.to_lowercase();
    let code = match lower.as_str() {
        "space" | " " => KeyCode::Char(' '),
        "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "tab" if modifiers.contains(KeyModifiers::SHIFT) => {
            modifiers.remove(KeyModifiers::SHIFT);
            KeyCode::BackTab
        }
        "tab" => KeyCode::Tab,
        "backspace" | "bs" => KeyCode::Backspace,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "minus" => KeyCode::Char('-'),
        "comma" => KeyCode::Char(','),
        "period" => KeyCode::Char('.'),
        "slash" => KeyCode::Char('/'),
        "backslash" => KeyCode::Char('\\'),
        "quote" => KeyCode::Char('\''),
        "double_quote" | "double-quote" => KeyCode::Char('"'),
        "semicolon" => KeyCode::Char(';'),
        "colon" => KeyCode::Char(':'),
        "percent" => KeyCode::Char('%'),
        "ampersand" => KeyCode::Char('&'),
        "backtick" => KeyCode::Char('`'),
        "plus" => KeyCode::Char('+'),
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "page_up" | "page-up" | "pgup" => KeyCode::PageUp,
        "pagedown" | "page_down" | "page-down" | "pgdn" => KeyCode::PageDown,
        "delete" | "del" => KeyCode::Delete,
        "insert" | "ins" => KeyCode::Insert,
        "lbracket" | "left_bracket" | "left-bracket" => KeyCode::Char('['),
        "rbracket" | "right_bracket" | "right-bracket" => KeyCode::Char(']'),
        "lbrace" | "left_brace" | "left-brace" => KeyCode::Char('{'),
        "rbrace" | "right_brace" | "right-brace" => KeyCode::Char('}'),
        "hash" => KeyCode::Char('#'),
        "equals" | "equal" => KeyCode::Char('='),
        "pipe" => KeyCode::Char('|'),
        _ if single_char.is_some() => {
            let ch = single_char?;
            if ch.is_ascii_uppercase() {
                modifiers |= KeyModifiers::SHIFT;
                KeyCode::Char(ch.to_ascii_lowercase())
            } else {
                KeyCode::Char(ch)
            }
        }
        s if s.starts_with('f') => s[1..].parse::<u8>().ok().map(KeyCode::F)?,
        _ => return None,
    };

    Some(normalize_key_combo((code, modifiers)))
}

fn single_key_char(s: &str) -> Option<char> {
    let mut chars = s.chars();
    let ch = chars.next()?;
    if chars.next().is_none() {
        Some(ch)
    } else {
        None
    }
}

pub fn normalize_key_combo((mut code, mut modifiers): KeyCombo) -> KeyCombo {
    if matches!(code, KeyCode::Tab) && modifiers.contains(KeyModifiers::SHIFT) {
        code = KeyCode::BackTab;
        modifiers.remove(KeyModifiers::SHIFT);
    } else if matches!(code, KeyCode::BackTab) {
        modifiers.remove(KeyModifiers::SHIFT);
    }
    (code, modifiers)
}

#[cfg(test)]
pub fn key_event_matches_combo(key: &KeyEvent, combo: KeyCombo) -> bool {
    key_parts_match_combo(key.code, key.modifiers, None, combo)
}

pub fn terminal_key_matches_combo(key: &TerminalKey, combo: KeyCombo) -> bool {
    key_parts_match_combo(key.code, key.modifiers, key.shifted_codepoint, combo)
}

fn key_parts_match_combo(
    actual_code: KeyCode,
    actual_modifiers: KeyModifiers,
    shifted_codepoint: Option<u32>,
    combo: KeyCombo,
) -> bool {
    let (actual_code, actual_modifiers) = normalize_key_combo((actual_code, actual_modifiers));
    let (expected_code, expected_modifiers) = normalize_key_combo(combo);

    if actual_modifiers == expected_modifiers
        && key_codes_match(
            actual_code,
            actual_modifiers,
            expected_code,
            expected_modifiers,
            shifted_codepoint,
        )
    {
        return true;
    }

    let actual_without_shift = actual_modifiers.difference(KeyModifiers::SHIFT);
    actual_modifiers.contains(KeyModifiers::SHIFT)
        && actual_without_shift == expected_modifiers
        && shifted_char_matches_expected(actual_code, shifted_codepoint, expected_code)
        || legacy_shifted_ascii_letter_matches(
            actual_code,
            actual_modifiers,
            expected_code,
            expected_modifiers,
        )
}

fn key_codes_match(
    actual: KeyCode,
    actual_modifiers: KeyModifiers,
    expected: KeyCode,
    expected_modifiers: KeyModifiers,
    shifted_codepoint: Option<u32>,
) -> bool {
    match (actual, expected) {
        (KeyCode::Char(actual), KeyCode::Char(expected))
            if actual.is_ascii_alphabetic() && expected.is_ascii_alphabetic() =>
        {
            actual == expected
                || actual_modifiers.contains(KeyModifiers::SHIFT)
                    && expected_modifiers.contains(KeyModifiers::SHIFT)
                    && actual.eq_ignore_ascii_case(&expected)
        }
        (KeyCode::Char(actual), KeyCode::Char(expected)) => {
            actual == expected
                || shifted_char_matches_expected(
                    KeyCode::Char(actual),
                    shifted_codepoint,
                    KeyCode::Char(expected),
                )
        }
        (actual, expected) => actual == expected,
    }
}

fn legacy_shifted_ascii_letter_matches(
    actual_code: KeyCode,
    actual_modifiers: KeyModifiers,
    expected_code: KeyCode,
    expected_modifiers: KeyModifiers,
) -> bool {
    if actual_modifiers.contains(KeyModifiers::SHIFT) {
        return false;
    }
    let (KeyCode::Char(actual), KeyCode::Char(expected)) = (actual_code, expected_code) else {
        return false;
    };
    actual.is_ascii_uppercase()
        && expected.is_ascii_lowercase()
        && actual.to_ascii_lowercase() == expected
        && actual_modifiers | KeyModifiers::SHIFT == expected_modifiers
}

const SHIFTED_NUMBER_SYMBOLS: [(char, char); 9] = [
    ('1', '!'),
    ('2', '@'),
    ('3', '#'),
    ('4', '$'),
    ('5', '%'),
    ('6', '^'),
    ('7', '&'),
    ('8', '*'),
    ('9', '('),
];

fn shifted_number_symbol(ch: char) -> Option<char> {
    SHIFTED_NUMBER_SYMBOLS
        .iter()
        .find_map(|(number, symbol)| (*symbol == ch).then_some(*number))
}

fn indexed_shifted_number_matches(key: &TerminalKey, combo: KeyCombo, number: char) -> bool {
    let (expected_code, expected_modifiers) = normalize_key_combo(combo);
    matches!(expected_code, KeyCode::Char(expected) if expected == number)
        && expected_modifiers.contains(KeyModifiers::SHIFT)
        && key.modifiers == expected_modifiers.difference(KeyModifiers::SHIFT)
}

fn shifted_char_matches_expected(
    actual_code: KeyCode,
    shifted_codepoint: Option<u32>,
    expected_code: KeyCode,
) -> bool {
    let KeyCode::Char(expected) = expected_code else {
        return false;
    };
    if let Some(shifted) = shifted_codepoint.and_then(char::from_u32) {
        return shifted == expected;
    }
    matches!(actual_code, KeyCode::Char(actual) if actual == expected && is_shifted_punctuation(expected))
}

fn is_shifted_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '!' | '@'
            | '#'
            | '$'
            | '%'
            | '^'
            | '&'
            | '*'
            | '('
            | ')'
            | '_'
            | '+'
            | '{'
            | '}'
            | '|'
            | ':'
            | '"'
            | '<'
            | '>'
            | '?'
            | '~'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(label: &str, key: &TerminalKey) -> bool {
        let combo = parse_key_combo(label).expect("valid chord");
        terminal_key_matches_combo(key, combo)
    }

    #[test]
    fn parse_simple_char_combo() {
        assert_eq!(
            parse_key_combo("v"),
            Some((KeyCode::Char('v'), KeyModifiers::empty()))
        );
    }

    #[test]
    fn parse_unicode_char_combo() {
        assert_eq!(
            parse_key_combo("ö"),
            Some((KeyCode::Char('ö'), KeyModifiers::empty()))
        );
        assert_eq!(
            parse_key_combo("alt+é"),
            Some((KeyCode::Char('é'), KeyModifiers::ALT))
        );
    }

    #[test]
    fn parse_shift_tab_as_backtab() {
        assert_eq!(
            parse_key_combo("shift+tab"),
            Some((KeyCode::BackTab, KeyModifiers::empty()))
        );
    }

    #[test]
    fn parse_named_keys_and_punctuation() {
        for (label, code) in [
            ("minus", KeyCode::Char('-')),
            ("comma", KeyCode::Char(',')),
            ("ampersand", KeyCode::Char('&')),
            ("home", KeyCode::Home),
            ("end", KeyCode::End),
            ("pageup", KeyCode::PageUp),
            ("pgdn", KeyCode::PageDown),
            ("delete", KeyCode::Delete),
            ("lbracket", KeyCode::Char('[')),
            ("rbrace", KeyCode::Char('}')),
            ("hash", KeyCode::Char('#')),
            ("equals", KeyCode::Char('=')),
        ] {
            assert_eq!(
                parse_key_combo(label),
                Some((code, KeyModifiers::empty())),
                "{label}"
            );
        }
        assert_eq!(
            format_key_combo((KeyCode::PageUp, KeyModifiers::empty())),
            "pageup"
        );
        assert_eq!(
            parse_key_combo("pageup").map(format_key_combo).as_deref(),
            Some("pageup")
        );
    }

    #[test]
    fn unicode_bindings_match_non_us_keys() {
        for ch in ['ğ', 'ç', 'ş', 'ı', 'é', 'ø'] {
            assert!(matches(
                &ch.to_string(),
                &TerminalKey::new(KeyCode::Char(ch), KeyModifiers::empty())
            ));
        }
    }

    #[test]
    fn shifted_unicode_bindings_match_layout_aware_input() {
        for (base, shifted) in [('ğ', 'Ğ'), ('ç', 'Ç'), ('ş', 'Ş'), ('ı', 'I'), ('ø', 'Ø')]
        {
            assert!(matches(
                &format!("shift+{base}"),
                &TerminalKey::new(KeyCode::Char(base), KeyModifiers::SHIFT)
                    .with_shifted_codepoint(shifted as u32)
            ));
        }
    }

    #[test]
    fn shifted_letter_binding_matches_uppercase_key_event() {
        let combo = parse_key_combo("shift+n").expect("chord");
        assert!(key_event_matches_combo(
            &KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT),
            combo
        ));
    }

    #[test]
    fn shifted_letter_binding_matches_legacy_uppercase_key_event() {
        assert!(matches(
            "shift+n",
            &TerminalKey::new(KeyCode::Char('N'), KeyModifiers::empty())
        ));
        assert!(matches(
            "N",
            &TerminalKey::new(KeyCode::Char('N'), KeyModifiers::empty())
        ));
    }

    #[test]
    fn shifted_letter_binding_matches_modern_modified_key_event() {
        assert!(matches(
            "cmd+shift+j",
            &TerminalKey::new(
                KeyCode::Char('J'),
                KeyModifiers::SUPER | KeyModifiers::SHIFT
            )
        ));
    }

    #[test]
    fn legacy_uppercase_key_event_does_not_match_unshifted_letter_binding() {
        assert!(!matches(
            "n",
            &TerminalKey::new(KeyCode::Char('N'), KeyModifiers::empty())
        ));
    }

    #[test]
    fn legacy_uppercase_shift_fallback_is_limited_to_ascii_letters() {
        assert!(!matches(
            "shift+1",
            &TerminalKey::new(KeyCode::Char('!'), KeyModifiers::empty())
        ));
        assert!(!matches(
            "shift+ö",
            &TerminalKey::new(KeyCode::Char('Ö'), KeyModifiers::empty())
        ));
    }

    #[test]
    fn shifted_tab_inputs_match_backtab_canonical_binding() {
        for key in [
            TerminalKey::new(KeyCode::BackTab, KeyModifiers::empty()),
            TerminalKey::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            TerminalKey::new(KeyCode::Tab, KeyModifiers::SHIFT),
        ] {
            assert!(matches("shift+tab", &key), "{key:?}");
        }
        assert!(!matches(
            "tab",
            &TerminalKey::new(KeyCode::Tab, KeyModifiers::SHIFT)
        ));
        assert_eq!(
            normalize_key_combo((KeyCode::Tab, KeyModifiers::CONTROL | KeyModifiers::SHIFT)),
            (KeyCode::BackTab, KeyModifiers::CONTROL)
        );
    }

    #[test]
    fn format_modified_backtab_keeps_shift_label() {
        assert_eq!(
            format_key_combo((KeyCode::BackTab, KeyModifiers::CONTROL)),
            "ctrl+shift+tab"
        );
        assert_eq!(
            format_key_combo((KeyCode::BackTab, KeyModifiers::CONTROL | KeyModifiers::ALT)),
            "ctrl+alt+shift+tab"
        );
    }

    #[test]
    fn shifted_punctuation_matches_enhanced_input() {
        assert!(matches(
            "?",
            &TerminalKey::new(KeyCode::Char('?'), KeyModifiers::SHIFT)
        ));
        assert!(matches(
            "?",
            &TerminalKey::new(KeyCode::Char('/'), KeyModifiers::SHIFT)
                .with_shifted_codepoint('?' as u32)
        ));
        assert!(matches(
            "!",
            &TerminalKey::new(KeyCode::Char('1'), KeyModifiers::SHIFT)
                .with_shifted_codepoint('!' as u32)
        ));
    }

    #[test]
    fn indexed_digits_accept_legacy_shifted_numbers() {
        let combo = (KeyCode::Char('3'), KeyModifiers::SHIFT);
        assert_eq!(
            indexed_key_index(
                &TerminalKey::new(KeyCode::Char('#'), KeyModifiers::empty()),
                combo
            ),
            Some(2)
        );
        assert_eq!(
            indexed_key_index(
                &TerminalKey::new(KeyCode::Char('3'), KeyModifiers::empty()),
                (KeyCode::Char('3'), KeyModifiers::empty())
            ),
            Some(2)
        );
    }
}
