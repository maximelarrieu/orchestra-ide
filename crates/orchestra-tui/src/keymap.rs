//! Key bindings.
//!
//! The user's zellij config binds `Alt+h/j/k/l`, `Ctrl+b` and `Ctrl+g`, so the
//! TUI never uses `Alt` and avoids those two chords: every binding here is a
//! plain key or a `Ctrl` chord outside that set.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Quit,
    Help,
    Refresh,
    /// Screens 1..6.
    Screen(u8),
    NextScreen,
    PrevScreen,
    Up,
    Down,
    Left,
    Right,
    Top,
    Bottom,
    Select,
    Back,
    /// Start typing a `:` command.
    CommandPalette,
    /// A printable character while a text field has focus.
    Char(char),
    Backspace,
    /// Enter: newline in a multi-line field, validation elsewhere.
    Submit,
    /// Ctrl-S: confirm the whole form.
    Accept,
    /// Tab: next field of a form.
    NextField,
    Cancel,
}

/// Translate a key press outside text input.
pub fn map(key: KeyEvent) -> Option<Action> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    // Alt belongs to zellij: never consume it.
    if key.modifiers.contains(KeyModifiers::ALT) {
        return None;
    }
    Some(match (key.code, ctrl) {
        (KeyCode::Char('q'), false) => Action::Back,
        (KeyCode::Char('Q'), false) => Action::Quit,
        (KeyCode::Char('c'), true) => Action::Quit,
        (KeyCode::Char('?'), false) => Action::Help,
        (KeyCode::Char('R'), false) => Action::Refresh,
        (KeyCode::Char(c @ '1'..='6'), false) => Action::Screen(c as u8 - b'0'),
        (KeyCode::Tab, false) => Action::NextScreen,
        (KeyCode::BackTab, _) => Action::PrevScreen,
        (KeyCode::Char('j'), false) | (KeyCode::Down, false) => Action::Down,
        (KeyCode::Char('k'), false) | (KeyCode::Up, false) => Action::Up,
        (KeyCode::Char('h'), false) | (KeyCode::Left, false) => Action::Left,
        (KeyCode::Char('l'), false) | (KeyCode::Right, false) => Action::Right,
        (KeyCode::Char('g'), false) => Action::Top,
        (KeyCode::Char('G'), false) => Action::Bottom,
        (KeyCode::Home, false) => Action::Top,
        (KeyCode::End, false) => Action::Bottom,
        (KeyCode::Enter, false) => Action::Select,
        (KeyCode::Esc, _) => Action::Cancel,
        (KeyCode::Char(':'), false) => Action::CommandPalette,
        // Anything else printable is offered to the current screen, which is
        // how per-screen keys work without a second key table.
        (KeyCode::Char(c), false) => Action::Char(c),
        _ => return None,
    })
}

/// Translate a key press while a text field has focus.
pub fn map_input(key: KeyEvent) -> Option<Action> {
    if key.modifiers.contains(KeyModifiers::ALT) {
        return None;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    Some(match (key.code, ctrl) {
        (KeyCode::Char('c'), true) => Action::Quit,
        // Ctrl-S confirms: Ctrl-Enter is not distinguishable in most terminals.
        (KeyCode::Char('s'), true) => Action::Accept,
        (KeyCode::Esc, _) => Action::Cancel,
        (KeyCode::Tab, _) => Action::NextField,
        (KeyCode::Enter, _) => Action::Submit,
        (KeyCode::Backspace, _) => Action::Backspace,
        (KeyCode::Char(c), false) => Action::Char(c),
        _ => return None,
    })
}

/// One line of the help overlay.
pub const HELP: &[(&str, &str)] = &[
    ("1..6", "aller à un écran"),
    ("Tab / Maj-Tab", "écran suivant / précédent"),
    ("h j k l", "se déplacer"),
    ("g / G", "début / fin"),
    ("Entrée", "ouvrir"),
    ("q", "retour"),
    (":", "palette de commandes"),
    ("R", "rafraîchir"),
    ("?", "cette aide"),
    ("Q / Ctrl-C", "quitter"),
    ("", ""),
    ("écran Coût", ""),
    ("m", "changer le regroupement"),
    ("p", "changer la période"),
    ("u", "inclure/exclure les sessions libres"),
    ("", ""),
    ("Tableau / Ticket", ""),
    ("n", "nouveau ticket"),
    ("p", "planifier le ticket"),
    ("a", "relire l'équipe proposée"),
    ("", ""),
    ("saisie", ""),
    ("Tab", "champ suivant"),
    ("Ctrl-S", "valider"),
    ("Échap", "annuler"),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn alt(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn vim_keys_move() {
        assert_eq!(map(key('j')), Some(Action::Down));
        assert_eq!(map(key('k')), Some(Action::Up));
        assert_eq!(map(key('h')), Some(Action::Left));
        assert_eq!(map(key('l')), Some(Action::Right));
    }

    #[test]
    fn alt_is_left_to_zellij() {
        for c in ['h', 'j', 'k', 'l', 'n', 'o', 'p', 'f'] {
            assert_eq!(map(alt(c)), None, "Alt+{c} doit revenir à zellij");
            assert_eq!(map_input(alt(c)), None);
        }
    }

    #[test]
    fn zellij_ctrl_chords_are_not_taken() {
        // Ctrl+b (tmux mode) and Ctrl+g (locked) must fall through.
        assert_eq!(map(ctrl('b')), None);
        assert_eq!(map(ctrl('g')), None);
        // Ctrl+c is ours: it quits.
        assert_eq!(map(ctrl('c')), Some(Action::Quit));
    }

    #[test]
    fn unmapped_letters_reach_the_current_screen() {
        // Screen-specific keys (m, p, u on the cost view) arrive as characters.
        assert_eq!(map(key('m')), Some(Action::Char('m')));
        assert_eq!(map(key('p')), Some(Action::Char('p')));
        assert_eq!(map(key('u')), Some(Action::Char('u')));
        // Without stealing the global bindings.
        assert_eq!(map(key('j')), Some(Action::Down));
        assert_eq!(map(key('q')), Some(Action::Back));
        assert_eq!(map(key('G')), Some(Action::Bottom));
    }

    #[test]
    fn screens_are_numbered() {
        assert_eq!(map(key('1')), Some(Action::Screen(1)));
        assert_eq!(map(key('6')), Some(Action::Screen(6)));
        // There is no seventh screen; the digit falls through to the screen.
        assert_eq!(map(key('7')), Some(Action::Char('7')));
    }

    #[test]
    fn forms_have_their_own_keys() {
        assert_eq!(map_input(ctrl('s')), Some(Action::Accept));
        assert_eq!(
            map_input(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            Some(Action::NextField)
        );
        // Enter stays available for a newline inside a brief.
        assert_eq!(
            map_input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Some(Action::Submit)
        );
    }

    #[test]
    fn input_mode_types_characters() {
        assert_eq!(map_input(key('a')), Some(Action::Char('a')));
        assert_eq!(map_input(key('j')), Some(Action::Char('j')));
        assert_eq!(
            map_input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Some(Action::Submit)
        );
        assert_eq!(
            map_input(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Some(Action::Cancel)
        );
    }
}
