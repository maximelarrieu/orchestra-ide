//! État du tableau de bord, indépendant du rendu.
//!
//! Sépare la *logique* d'agrégation du flux (compteurs, historique) du *dessin*
//! (`dashboard`). On peut ainsi tester l'agrégation sans terminal ni tokio.

use orchestra_core::events::AgentEvent;
use orchestra_core::model::ContextSpace;

/// Nombre d'événements conservés dans l'historique du radar (les plus anciens sont
/// oubliés). Largement au-delà de ce qu'un écran affiche.
const HISTORY_CAP: usize = 500;

/// Phase de l'orchestre, déduite du flux d'événements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Rien n'a encore été lancé (état initial).
    Idle,
    /// Au moins un agent tourne.
    Running,
    /// L'orchestre a été lancé puis le canal s'est fermé (tous terminés).
    Finished,
}

pub struct App {
    pub space: Option<ContextSpace>,
    pub events: Vec<AgentEvent>,
    pub started: usize,
    pub done: usize,
    pub phase: Phase,
}

impl App {
    pub fn new(space: Option<ContextSpace>) -> Self {
        Self {
            space,
            events: Vec::new(),
            started: 0,
            done: 0,
            phase: Phase::Idle,
        }
    }

    /// Vrai si un Espace valide est chargé : sans lui, pas d'agents à lancer.
    pub fn can_launch(&self) -> bool {
        self.space
            .as_ref()
            .is_some_and(|s| !s.config.agents.is_empty())
    }

    /// Remet le radar à zéro et passe en `Running` (appelé quand l'utilisateur lance
    /// l'orchestre). L'historique précédent est effacé pour une lecture nette.
    pub fn begin_run(&mut self) {
        self.events.clear();
        self.started = 0;
        self.done = 0;
        self.phase = Phase::Running;
    }

    /// Intègre un événement du runtime dans l'état (compteurs + historique borné).
    pub fn on_event(&mut self, ev: AgentEvent) {
        match &ev {
            AgentEvent::Started { .. } => self.started += 1,
            AgentEvent::Done { .. } => self.done += 1,
            AgentEvent::Log { .. } => {}
        }
        self.events.push(ev);
        if self.events.len() > HISTORY_CAP {
            self.events.remove(0);
        }
    }

    /// Signalé par la boucle principale quand le canal se ferme (tous les agents finis).
    pub fn mark_finished(&mut self) {
        if self.phase == Phase::Running {
            self.phase = Phase::Finished;
        }
    }

    /// Agents encore en cours (démarrés mais pas terminés).
    pub fn running_count(&self) -> usize {
        self.started.saturating_sub(self.done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_started_and_done() {
        let mut app = App::new(None);
        app.begin_run();
        app.on_event(AgentEvent::Started { agent: "A".into() });
        app.on_event(AgentEvent::Log { agent: "A".into(), msg: "x".into() });
        app.on_event(AgentEvent::Started { agent: "B".into() });
        app.on_event(AgentEvent::Done { agent: "A".into() });

        assert_eq!(app.started, 2);
        assert_eq!(app.done, 1);
        assert_eq!(app.running_count(), 1);
        assert_eq!(app.events.len(), 4);

        app.mark_finished();
        assert_eq!(app.phase, Phase::Finished);
    }

    #[test]
    fn history_is_capped() {
        let mut app = App::new(None);
        for _ in 0..(HISTORY_CAP + 50) {
            app.on_event(AgentEvent::Log { agent: "A".into(), msg: "x".into() });
        }
        assert_eq!(app.events.len(), HISTORY_CAP);
    }

    /// L'espace d'exemple livré doit se charger ET être lançable (sinon `[1]` ne fait
    /// rien). Ce test reproduit ce que voit l'utilisateur qui ouvre cet espace.
    #[test]
    fn bundled_example_space_can_launch() {
        let example = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/recherche-immo-aix");
        let space = ContextSpace::load(&example).expect("l'exemple doit se charger");
        let app = App::new(Some(space));
        assert!(app.can_launch(), "l'exemple doit avoir des agents → [1] actif");
    }
}
