//! Sessions de travail — le **système d'onglets** d'Orchestra IDE.
//!
//! Une *session* est un onglet : un Espace ouvert avec son propre **contexte** et son propre
//! **historique** (conversation, statuts d'agents, modifications…). On peut en avoir plusieurs
//! ouvertes et **basculer de l'une à l'autre** comme dans un éditeur.
//!
//! Ce module ne porte que la **mécanique d'onglets** (ouvrir, fermer, activer, suivant/précédent),
//! agnostique de l'affichage : chaque UI définit son propre type d'onglet (avec canaux,
//! transcription, sélection…) et implémente [`Tabbed`] pour donner son titre et sa racine. La
//! logique vit ici, testée une seule fois, et les deux interfaces (TUI, desktop) s'en servent à
//! l'identique — c'est la règle de parité du projet.

use std::path::Path;

/// Ce qu'un onglet doit savoir dire de lui-même pour être géré : son **titre** (affiché sur
/// l'onglet) et sa **racine** sur disque (pour dédupliquer — rouvrir un Espace déjà ouvert
/// réactive son onglet au lieu d'en créer un doublon).
pub trait Tabbed {
    /// Titre d'affichage de l'onglet.
    fn title(&self) -> String;
    /// Racine de l'Espace, identifiant unique de l'onglet.
    fn root(&self) -> &Path;
}

/// Collection ordonnée d'onglets + l'onglet actif. Toujours cohérente : `active` pointe sur un
/// onglet existant tant qu'il en reste un.
pub struct Sessions<T> {
    tabs: Vec<T>,
    active: usize,
}

impl<T> Default for Sessions<T> {
    fn default() -> Self {
        Self { tabs: Vec::new(), active: 0 }
    }
}

impl<T> Sessions<T> {
    /// Aucune session ouverte.
    pub fn new() -> Self {
        Self::default()
    }

    /// Vrai s'il n'y a aucun onglet.
    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    /// Nombre d'onglets ouverts.
    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    /// Index de l'onglet actif (0 si aucun).
    pub fn active_index(&self) -> usize {
        self.active
    }

    /// L'onglet actif (`None` si aucun onglet).
    pub fn active(&self) -> Option<&T> {
        self.tabs.get(self.active)
    }

    /// L'onglet actif en écriture (`None` si aucun onglet).
    pub fn active_mut(&mut self) -> Option<&mut T> {
        self.tabs.get_mut(self.active)
    }

    /// Onglet par index.
    pub fn get(&self, index: usize) -> Option<&T> {
        self.tabs.get(index)
    }

    /// Onglet par index, en écriture (utile pour muter une session **non active** — p.ex. quand
    /// un flux d'événements arrive pour une session en arrière-plan).
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.tabs.get_mut(index)
    }

    /// Itère sur les onglets, dans l'ordre (pour dessiner la barre d'onglets).
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.tabs.iter()
    }

    /// Ferme l'onglet `index`. L'onglet actif reste cohérent : on garde l'onglet voisin de
    /// gauche. Sans effet si `index` est hors limites.
    pub fn close(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.active = 0;
        } else if self.active > index || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1).min(self.tabs.len() - 1);
        }
    }

    /// Ferme l'onglet actif (raccourci de [`Self::close`]).
    pub fn close_active(&mut self) {
        self.close(self.active);
    }

    /// Active l'onglet `index` (borné : sans effet si hors limites).
    pub fn switch_to(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active = index;
        }
    }

    /// Passe à l'onglet suivant (cyclique).
    pub fn next(&mut self) {
        if !self.tabs.is_empty() {
            self.active = (self.active + 1) % self.tabs.len();
        }
    }

    /// Passe à l'onglet précédent (cyclique).
    pub fn prev(&mut self) {
        if !self.tabs.is_empty() {
            self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
        }
    }
}

impl<T: Tabbed> Sessions<T> {
    /// Ouvre `tab` : si un onglet a **déjà** la même racine, on **réactive l'onglet existant**
    /// (son contexte/historique sont préservés — `tab` est ignoré) ; sinon `tab` est ajouté en
    /// fin et activé. Renvoie l'index de l'onglet actif.
    pub fn open(&mut self, tab: T) -> usize {
        if let Some(i) = self.tabs.iter().position(|t| t.root() == tab.root()) {
            self.active = i;
        } else {
            self.tabs.push(tab);
            self.active = self.tabs.len() - 1;
        }
        self.active
    }

    /// Titres d'affichage, dans l'ordre.
    pub fn titles(&self) -> Vec<String> {
        self.tabs.iter().map(|t| t.title()).collect()
    }

    /// Index de l'onglet ayant cette racine, s'il existe.
    pub fn index_of(&self, root: &Path) -> Option<usize> {
        self.tabs.iter().position(|t| t.root() == root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Onglet minimal pour les tests.
    struct T {
        title: String,
        root: PathBuf,
    }
    impl T {
        fn new(title: &str, root: &str) -> Self {
            Self { title: title.to_string(), root: PathBuf::from(root) }
        }
    }
    impl Tabbed for T {
        fn title(&self) -> String {
            self.title.clone()
        }
        fn root(&self) -> &Path {
            &self.root
        }
    }

    #[test]
    fn opens_and_activates_new_tabs() {
        let mut s: Sessions<T> = Sessions::new();
        assert!(s.is_empty());
        s.open(T::new("A", "/a"));
        s.open(T::new("B", "/b"));
        assert_eq!(s.len(), 2);
        assert_eq!(s.active_index(), 1); // le dernier ouvert est actif
        assert_eq!(s.active().unwrap().title(), "B");
        assert_eq!(s.titles(), vec!["A", "B"]);
    }

    #[test]
    fn reopening_same_root_activates_existing_tab_without_replacing() {
        let mut s: Sessions<T> = Sessions::new();
        s.open(T::new("Original", "/a"));
        s.open(T::new("B", "/b"));
        s.open(T::new("Rechargé", "/a")); // même racine que le 1er
        assert_eq!(s.len(), 2, "pas de doublon");
        assert_eq!(s.active_index(), 0, "l'onglet existant redevient actif");
        // L'onglet d'origine est préservé (contexte/historique intacts), pas remplacé.
        assert_eq!(s.active().unwrap().title(), "Original");
    }

    #[test]
    fn switching_and_cycling() {
        let mut s: Sessions<T> = Sessions::new();
        s.open(T::new("A", "/a"));
        s.open(T::new("B", "/b"));
        s.open(T::new("C", "/c"));
        s.switch_to(0);
        assert_eq!(s.active_index(), 0);
        s.prev();
        assert_eq!(s.active_index(), 2, "précédent depuis 0 → dernier (cyclique)");
        s.next();
        assert_eq!(s.active_index(), 0, "suivant depuis dernier → 0 (cyclique)");
        s.switch_to(9); // hors limites → sans effet
        assert_eq!(s.active_index(), 0);
    }

    #[test]
    fn closing_keeps_active_coherent() {
        let mut s: Sessions<T> = Sessions::new();
        s.open(T::new("A", "/a"));
        s.open(T::new("B", "/b"));
        s.open(T::new("C", "/c"));
        s.switch_to(2); // actif = C
        s.close(1); // ferme B → [A, C], actif décalé à 1 (toujours C)
        assert_eq!(s.titles(), vec!["A", "C"]);
        assert_eq!(s.active_index(), 1);
        assert_eq!(s.active().unwrap().title(), "C");

        s.close_active(); // ferme C → [A], actif = 0
        assert_eq!(s.titles(), vec!["A"]);
        assert_eq!(s.active_index(), 0);

        s.close(0); // plus rien
        assert!(s.is_empty());
        assert!(s.active().is_none());
    }

    #[test]
    fn closing_before_active_shifts_active_left() {
        let mut s: Sessions<T> = Sessions::new();
        s.open(T::new("A", "/a"));
        s.open(T::new("B", "/b"));
        s.open(T::new("C", "/c"));
        s.switch_to(2); // actif = C (index 2)
        s.close(0); // ferme A → [B, C], actif suit C à l'index 1
        assert_eq!(s.active().unwrap().title(), "C");
        assert_eq!(s.active_index(), 1);
    }
}
