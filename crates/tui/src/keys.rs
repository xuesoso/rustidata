//! Keystroke normalization: crossterm events -> visidata-style keystroke
//! strings ("j", "^R", "KEY_LEFT", "Enter", "g", "z" prefixes are handled in
//! the mainloop by concatenation).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Render a key event as a visidata-compatible keystroke string.
pub fn keystroke(ev: &KeyEvent) -> Option<String> {
    let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
    let alt = ev.modifiers.contains(KeyModifiers::ALT);
    let base = match ev.code {
        KeyCode::Char(c) => {
            if ctrl {
                format!("^{}", c.to_ascii_uppercase())
            } else {
                c.to_string()
            }
        }
        KeyCode::Enter => "Enter".into(),
        KeyCode::Esc => "Esc".into(),
        KeyCode::Backspace => "Backspace".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::BackTab => "Shift+Tab".into(),
        KeyCode::Delete => "Delete".into(),
        KeyCode::Insert => "Insert".into(),
        KeyCode::Up => "Up".into(),
        KeyCode::Down => "Down".into(),
        KeyCode::Left => "Left".into(),
        KeyCode::Right => "Right".into(),
        KeyCode::Home => "Home".into(),
        KeyCode::End => "End".into(),
        KeyCode::PageUp => "PgUp".into(),
        KeyCode::PageDown => "PgDn".into(),
        KeyCode::F(n) => format!("F{}", n),
        _ => return None,
    };
    Some(if alt { format!("Alt+{}", base) } else { base })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ks(code: KeyCode, mods: KeyModifiers) -> Option<String> {
        keystroke(&KeyEvent::new(code, mods))
    }

    #[test]
    fn plain_and_modified() {
        assert_eq!(ks(KeyCode::Char('j'), KeyModifiers::NONE).unwrap(), "j");
        assert_eq!(
            ks(KeyCode::Char('r'), KeyModifiers::CONTROL).unwrap(),
            "^R"
        );
        assert_eq!(ks(KeyCode::Enter, KeyModifiers::NONE).unwrap(), "Enter");
        assert_eq!(
            ks(KeyCode::Char('F'), KeyModifiers::SHIFT).unwrap(),
            "F"
        );
        assert_eq!(
            ks(KeyCode::Char('x'), KeyModifiers::ALT).unwrap(),
            "Alt+x"
        );
    }
}
