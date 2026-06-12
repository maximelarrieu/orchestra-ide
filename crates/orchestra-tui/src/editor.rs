//! Mini éditeur de texte multi-ligne, pour modifier le persona depuis l'interface.
//!
//! Logique pure (aucun rendu, aucune E/S) : un tampon `lignes × colonnes` en `char`
//! (UTF-8 sûr, accents compris) et un curseur. Le rendu vit dans `dashboard`, la sauvegarde
//! dans le cœur. Volontairement minimal : insertion, retour arrière, navigation aux flèches.

pub struct Editor {
    lines: Vec<Vec<char>>,
    cy: usize, // ligne courante
    cx: usize, // colonne (indice de caractère dans la ligne)
    dirty: bool,
}

impl Editor {
    /// Construit l'éditeur depuis un texte (découpé en lignes sur `\n`).
    pub fn from_str(s: &str) -> Self {
        let mut lines: Vec<Vec<char>> = s.split('\n').map(|l| l.chars().collect()).collect();
        if lines.is_empty() {
            lines.push(Vec::new());
        }
        Self { lines, cy: 0, cx: 0, dirty: false }
    }

    /// Reconstruit le texte complet.
    pub fn to_text(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn line_len(&self) -> usize {
        self.lines[self.cy].len()
    }

    pub fn insert_char(&mut self, c: char) {
        self.lines[self.cy].insert(self.cx, c);
        self.cx += 1;
        self.dirty = true;
    }

    pub fn newline(&mut self) {
        let tail = self.lines[self.cy].split_off(self.cx);
        self.lines.insert(self.cy + 1, tail);
        self.cy += 1;
        self.cx = 0;
        self.dirty = true;
    }

    pub fn backspace(&mut self) {
        if self.cx > 0 {
            self.lines[self.cy].remove(self.cx - 1);
            self.cx -= 1;
            self.dirty = true;
        } else if self.cy > 0 {
            // Fusion avec la ligne précédente.
            let cur = self.lines.remove(self.cy);
            self.cy -= 1;
            self.cx = self.lines[self.cy].len();
            self.lines[self.cy].extend(cur);
            self.dirty = true;
        }
    }

    pub fn left(&mut self) {
        if self.cx > 0 {
            self.cx -= 1;
        } else if self.cy > 0 {
            self.cy -= 1;
            self.cx = self.line_len();
        }
    }

    pub fn right(&mut self) {
        if self.cx < self.line_len() {
            self.cx += 1;
        } else if self.cy + 1 < self.lines.len() {
            self.cy += 1;
            self.cx = 0;
        }
    }

    pub fn up(&mut self) {
        if self.cy > 0 {
            self.cy -= 1;
            self.cx = self.cx.min(self.line_len());
        }
    }

    pub fn down(&mut self) {
        if self.cy + 1 < self.lines.len() {
            self.cy += 1;
            self.cx = self.cx.min(self.line_len());
        }
    }

    pub fn home(&mut self) {
        self.cx = 0;
    }

    pub fn end(&mut self) {
        self.cx = self.line_len();
    }

    /// Position du curseur `(ligne, colonne)` pour le rendu.
    pub fn cursor(&self) -> (usize, usize) {
        (self.cy, self.cx)
    }

    pub fn lines(&self) -> &[Vec<char>] {
        &self.lines
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_newline_backspace_round_trip() {
        let mut ed = Editor::from_str("ab");
        ed.end(); // curseur après 'b'
        ed.insert_char('c'); // "abc"
        ed.newline(); // "abc\n"
        ed.insert_char('x'); // "abc\nx"
        assert_eq!(ed.to_text(), "abc\nx");
        ed.backspace(); // "abc\n"
        ed.backspace(); // fusion → "abc"
        assert_eq!(ed.to_text(), "abc");
        assert!(ed.is_dirty());
    }

    #[test]
    fn navigation_clamps_within_bounds() {
        let mut ed = Editor::from_str("court\nplus longue");
        ed.up(); // déjà en haut → reste
        assert_eq!(ed.cursor(), (0, 0));
        ed.end(); // fin de "court" (col 5)
        ed.down(); // ligne 1, col min(5, len)
        assert_eq!(ed.cursor(), (1, 5));
        ed.left();
        assert_eq!(ed.cursor(), (1, 4));
    }

    #[test]
    fn handles_utf8_accents() {
        let mut ed = Editor::from_str("éà");
        ed.end();
        ed.insert_char('ü');
        assert_eq!(ed.to_text(), "éàü");
        ed.backspace();
        assert_eq!(ed.to_text(), "éà");
    }
}
