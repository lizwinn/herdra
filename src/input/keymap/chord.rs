//! Key chords as written in a keymap, and matching them against key presses.

use crossterm::event::{KeyCode, KeyModifiers};

use crate::config::{
    format_key_combo, indexed_key_index, normalize_key_combo, parse_key_combo,
    parse_range_modifiers, terminal_key_matches_combo, KeyCombo,
};
use crate::input::TerminalKey;

/// One key chord at one level of the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Chord {
    /// A single key with modifiers, normalized.
    Key(KeyCombo),
    /// The digits 1 through 9 with the same modifiers. The digit pressed is
    /// passed to indexed actions.
    Digits(KeyModifiers),
}

impl Chord {
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err("empty key chord".to_owned());
        }
        if trimmed.ends_with('+') && trimmed.len() > 1 {
            return Err(format!(
                "incomplete key chord {trimmed:?}; quote chords that contain ; # = [ ] {{ }} / or \\, for example \"ctrl+;\""
            ));
        }
        if trimmed.contains("1..9") {
            return parse_range_modifiers(trimmed)
                .map(Self::Digits)
                .ok_or_else(|| format!("invalid key chord {trimmed:?}"));
        }
        parse_key_combo(trimmed)
            .map(|combo| Self::Key(normalize_key_combo(combo)))
            .ok_or_else(|| format!("invalid key chord {trimmed:?}"))
    }

    pub(crate) fn label(self) -> String {
        match self {
            Self::Key(combo) => format_key_combo(combo),
            Self::Digits(modifiers) => {
                let prefix = format_key_combo((KeyCode::Char('1'), modifiers));
                format!("{}..9", prefix)
            }
        }
    }

    /// Whether this chord would steal ordinary typing if bound at the top
    /// level, where keys otherwise go straight to the pane.
    pub(crate) fn intercepts_typing(self) -> bool {
        let (code, modifiers) = match self {
            Self::Key(combo) => combo,
            Self::Digits(modifiers) => (KeyCode::Char('1'), modifiers),
        };
        let unmodified = modifiers.difference(KeyModifiers::SHIFT).is_empty();
        unmodified
            && match code {
                KeyCode::Char(ch) => !ch.is_control(),
                KeyCode::Enter
                | KeyCode::Esc
                | KeyCode::Tab
                | KeyCode::BackTab
                | KeyCode::Backspace
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::Up
                | KeyCode::Down => true,
                _ => false,
            }
    }

    /// Whether two chords can match the same key press.
    pub(crate) fn overlaps(self, other: Self) -> bool {
        match (self, other) {
            (Self::Key(left), Self::Key(right)) => left == right,
            (Self::Digits(left), Self::Digits(right)) => left == right,
            (Self::Digits(modifiers), Self::Key((code, key_modifiers)))
            | (Self::Key((code, key_modifiers)), Self::Digits(modifiers)) => {
                matches!(code, KeyCode::Char('1'..='9')) && key_modifiers == modifiers
            }
        }
    }
}

/// How strictly a key press has to match a chord in one resolution pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MatchPass {
    /// Plain chords, and digit chords whose modifiers match exactly.
    Exact,
    /// Digit chords that accept legacy shifted-number symbols.
    LooseDigits,
}

/// Zero-based digit index when a digit chord matched; `Some(None)` for a plain
/// chord match.
pub(crate) fn chord_match(
    chord: Chord,
    key: &TerminalKey,
    pass: MatchPass,
) -> Option<Option<usize>> {
    match (chord, pass) {
        (Chord::Key(combo), MatchPass::Exact) => {
            terminal_key_matches_combo(key, combo).then_some(None)
        }
        (Chord::Key(_), MatchPass::LooseDigits) => None,
        (Chord::Digits(modifiers), pass) => {
            let actual = normalize_key_combo((key.code, key.modifiers)).1;
            if pass == MatchPass::Exact && actual != modifiers {
                return None;
            }
            ('1'..='9').find_map(|digit| {
                indexed_key_index(key, (KeyCode::Char(digit), modifiers)).map(Some)
            })
        }
    }
}

/// The unmodified character a key produced, used as a fallback so menus work
/// on keyboard layouts where the chord's key sits behind a modifier.
pub(crate) fn generated_character_key(key: &TerminalKey) -> Option<TerminalKey> {
    let mut characters = key.generated_text.as_deref()?.chars();
    let character = characters.next()?;
    if character.is_control() || characters.next().is_some() {
        return None;
    }
    Some(TerminalKey::new(
        KeyCode::Char(character),
        KeyModifiers::empty(),
    ))
}

/// Bar-friendly key label: arrows as glyphs, everything else as written.
pub(crate) fn display_label(chord: Chord) -> String {
    match chord {
        Chord::Key((KeyCode::Up, modifiers)) if modifiers.is_empty() => "↑".to_owned(),
        Chord::Key((KeyCode::Down, modifiers)) if modifiers.is_empty() => "↓".to_owned(),
        Chord::Key((KeyCode::Left, modifiers)) if modifiers.is_empty() => "←".to_owned(),
        Chord::Key((KeyCode::Right, modifiers)) if modifiers.is_empty() => "→".to_owned(),
        Chord::Key((KeyCode::Char(ch), KeyModifiers::SHIFT)) if ch.is_ascii_lowercase() => {
            ch.to_ascii_uppercase().to_string()
        }
        chord => chord.label(),
    }
}
