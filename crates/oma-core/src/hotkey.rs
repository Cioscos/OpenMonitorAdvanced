//! Global hotkey text: parsing and the canonical spelling. Portable; the
//! shell registers the parsed combination with the operating system.

use std::fmt;

/// A key combination with at least two modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: HotkeyKey,
}

/// The non-modifier key of a [`Hotkey`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HotkeyKey {
    /// `'A'..='Z'`.
    Letter(char),
    /// `0..=9`.
    Digit(u8),
    /// `1..=24`.
    Function(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyError {
    /// Empty text or segment, a repeated or misplaced modifier.
    Syntax,
    /// Fewer than two of Ctrl, Alt and Shift.
    TooFewModifiers,
    /// A key outside `A`-`Z`, `0`-`9` and `F1`-`F24`.
    UnsupportedKey,
}

fn parse_key(text: &str) -> Result<HotkeyKey, HotkeyError> {
    let mut chars = text.chars();
    let (Some(first), rest) = (chars.next(), chars.as_str()) else {
        return Err(HotkeyError::Syntax);
    };
    if rest.is_empty() {
        if first.is_ascii_alphabetic() {
            return Ok(HotkeyKey::Letter(first.to_ascii_uppercase()));
        }
        if let Some(digit) = first.to_digit(10).filter(|_| first.is_ascii_digit()) {
            return Ok(HotkeyKey::Digit(digit as u8));
        }
    }
    if first.eq_ignore_ascii_case(&'f')
        && !rest.is_empty()
        && rest.bytes().all(|b| b.is_ascii_digit())
    {
        // Leading zeros ("F01") are not a spelling of the key.
        if let Ok(n) = rest.parse::<u8>() {
            if (1..=24).contains(&n) && !rest.starts_with('0') {
                return Ok(HotkeyKey::Function(n));
            }
        }
    }
    Err(HotkeyError::UnsupportedKey)
}

/// Parses `Ctrl+Alt+Shift+R`-style text: case-insensitive, spaces around `+`
/// allowed, modifiers in any order, at least two of them.
pub fn parse_hotkey(text: &str) -> Result<Hotkey, HotkeyError> {
    let segments: Vec<&str> = text.split('+').map(str::trim).collect();
    if segments.iter().any(|s| s.is_empty()) {
        return Err(HotkeyError::Syntax);
    }
    let (key_text, modifiers) = segments.split_last().ok_or(HotkeyError::Syntax)?;
    let (mut ctrl, mut alt, mut shift) = (false, false, false);
    for modifier in modifiers {
        let slot = match modifier.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => &mut ctrl,
            "alt" => &mut alt,
            "shift" => &mut shift,
            _ => return Err(HotkeyError::Syntax),
        };
        if std::mem::replace(slot, true) {
            return Err(HotkeyError::Syntax);
        }
    }
    let key = parse_key(key_text)?;
    if [ctrl, alt, shift].into_iter().filter(|m| *m).count() < 2 {
        return Err(HotkeyError::TooFewModifiers);
    }
    Ok(Hotkey {
        ctrl,
        alt,
        shift,
        key,
    })
}

impl fmt::Display for Hotkey {
    /// The canonical spelling: modifiers as `Ctrl+Alt+Shift`, then the key.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, name) in [
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
        ] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        match self.key {
            HotkeyKey::Letter(c) => write!(f, "{c}"),
            HotkeyKey::Digit(d) => write!(f, "{d}"),
            HotkeyKey::Function(n) => write!(f, "F{n}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_default_combination() {
        assert_eq!(
            parse_hotkey("Ctrl+Alt+Shift+R"),
            Ok(Hotkey {
                ctrl: true,
                alt: true,
                shift: true,
                key: HotkeyKey::Letter('R')
            })
        );
        assert_eq!(parse_hotkey(" ctrl + alt + r "), parse_hotkey("Ctrl+Alt+R"));
    }

    #[test]
    fn display_is_canonical() {
        assert_eq!(
            parse_hotkey("shift+ctrl+r").unwrap().to_string(),
            "Ctrl+Shift+R"
        );
        assert_eq!(
            parse_hotkey("SHIFT + alt + Control + r")
                .unwrap()
                .to_string(),
            "Ctrl+Alt+Shift+R"
        );
    }

    #[test]
    fn requires_two_modifiers() {
        assert_eq!(parse_hotkey("Ctrl+R"), Err(HotkeyError::TooFewModifiers));
        assert_eq!(parse_hotkey("R"), Err(HotkeyError::TooFewModifiers));
    }

    #[test]
    fn accepts_digits_and_function_keys() {
        assert_eq!(parse_hotkey("Ctrl+Alt+7").unwrap().key, HotkeyKey::Digit(7));
        let f12 = parse_hotkey("Alt+Shift+F12").unwrap();
        assert_eq!(f12.key, HotkeyKey::Function(12));
        assert_eq!(f12.to_string(), "Alt+Shift+F12");
        assert_eq!(
            parse_hotkey("ctrl+alt+f24").unwrap().key,
            HotkeyKey::Function(24)
        );
    }

    #[test]
    fn rejects_unknown_keys() {
        for text in [
            "Ctrl+Alt+Space",
            "Ctrl+Alt+F25",
            "Ctrl+Alt+F0",
            "Ctrl+Alt+F01",
            "Ctrl+Alt+é",
        ] {
            assert_eq!(
                parse_hotkey(text),
                Err(HotkeyError::UnsupportedKey),
                "{text}"
            );
        }
    }

    #[test]
    fn rejects_empty_segments() {
        for text in ["", "Ctrl++R", "+Ctrl+Alt+R", "Ctrl+Alt+", "  "] {
            assert_eq!(parse_hotkey(text), Err(HotkeyError::Syntax), "{text:?}");
        }
    }

    #[test]
    fn rejects_repeated_or_misplaced_modifiers() {
        for text in ["Ctrl+Ctrl+R", "Ctrl+R+Alt", "Ctrl+Foo+R"] {
            assert_eq!(parse_hotkey(text), Err(HotkeyError::Syntax), "{text}");
        }
    }
}
