//! Runtime d'agents (Phase 3) — fait vivre le radar.
//!
//! [`spawn`] démarre l'« orchestre » décrit par un [`ContextSpace`] : chaque agent
//! devient une tâche `tokio` qui publie des [`AgentEvent`] sur un canal
//! `tokio::sync::mpsc`. L'UI consomme le `Receiver` renvoyé, sans rien savoir des agents.
//!
//! **Phase 3 = pas de LLM** : les agents sont *simulés*. Ils émettent un flux d'activité
//! crédible et étalé dans le temps pour valider toute la chaîne événementielle. Le
//! branchement d'un vrai modèle (et de Skills exécutables) arrive en Phase 4 ; la
//! signature de [`spawn`], elle, ne bougera pas.

use std::time::Duration;

use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::time::sleep;

use crate::events::AgentEvent;
use crate::model::space::ContextSpace;

/// Lance tous les agents de l'espace et renvoie le flux de leurs événements.
///
/// Le canal se ferme (le `recv()` renvoie `None`) lorsque tous les agents ont terminé :
/// l'UI sait ainsi que l'orchestre est au repos, sans drapeau supplémentaire.
pub fn spawn(space: &ContextSpace) -> UnboundedReceiver<AgentEvent> {
    let (tx, rx) = mpsc::unbounded_channel();

    for (idx, agent) in space.config.agents.iter().cloned().enumerate() {
        let tx = tx.clone();
        tokio::spawn(simulate_agent(agent, idx, tx));
    }
    // `tx` original lâché ici : une fois toutes les tâches finies, le canal se ferme.
    rx
}

/// Joue le scénario simulé d'un agent : démarrage décalé, quelques lignes de log,
/// puis fin. Les envois ignorent l'erreur « canal fermé » (UI quittée prématurément).
async fn simulate_agent(agent: String, idx: usize, tx: UnboundedSender<AgentEvent>) {
    // Décalage de démarrage : les agents n'entrent pas tous en scène en même temps.
    sleep(Duration::from_millis(150 * idx as u64)).await;
    let _ = tx.send(AgentEvent::Started {
        agent: agent.clone(),
    });

    for (step, msg) in scripted_steps(&agent).iter().enumerate() {
        sleep(Duration::from_millis(250 + 150 * step as u64)).await;
        let _ = tx.send(AgentEvent::Log {
            agent: agent.clone(),
            msg: (*msg).to_string(),
        });
    }

    sleep(Duration::from_millis(200)).await;
    let _ = tx.send(AgentEvent::Done { agent });
}

/// Lignes de log jouées par un agent. Le scénario est choisi d'après un mot-clé du nom
/// (scraper, filtrage, code, test…) avec un repli générique — assez pour donner vie au
/// radar sans rien coder de spécifique à un domaine.
fn scripted_steps(agent: &str) -> &'static [&'static str] {
    let key = agent.to_lowercase();
    if key.contains("scrap") {
        &[
            "connexion aux sources…",
            "3 pages parcourues",
            "27 annonces extraites",
        ]
    } else if key.contains("filtr") {
        &[
            "application des critères stricts…",
            "27 → 6 annonces retenues",
            "tri par pertinence terminé",
        ]
    } else if key.contains("cod") {
        &[
            "lecture du contexte projet…",
            "génération du patch proposé",
            "patch prêt pour relecture",
        ]
    } else if key.contains("test") {
        &[
            "exécution de la suite de tests…",
            "42 tests passés, 0 échec",
        ]
    } else if key.contains("archi") {
        &[
            "analyse des contraintes…",
            "plan d'implémentation rédigé",
        ]
    } else {
        &[
            "initialisation…",
            "traitement en cours…",
            "tâche accomplie",
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::config::ProjectConfig;
    use crate::model::project_type::ProjectType;
    use std::path::PathBuf;

    fn space_with_agents(agents: &[&str]) -> ContextSpace {
        ContextSpace {
            root: PathBuf::from("."),
            config: ProjectConfig {
                project_name: "Test".to_string(),
                project_type: ProjectType::Dev,
                workspace_path: None,
                documentalist_enabled: false,
                skills: vec![],
                agents: agents.iter().map(|s| s.to_string()).collect(),
                integrations: Default::default(),
            },
            persona: None,
            adrs: vec![],
        }
    }

    #[tokio::test]
    async fn each_agent_starts_and_finishes_once() {
        let space = space_with_agents(&["Agent_Scraper", "Agent_Filtrage"]);
        let mut rx = spawn(&space);

        let (mut started, mut done) = (0, 0);
        // Le canal se ferme tout seul quand les deux agents ont fini → la boucle se termine.
        while let Some(ev) = rx.recv().await {
            match ev {
                AgentEvent::Started { .. } => started += 1,
                AgentEvent::Done { .. } => done += 1,
                AgentEvent::Log { .. } => {}
            }
        }

        assert_eq!(started, 2, "chaque agent démarre une fois");
        assert_eq!(done, 2, "chaque agent termine une fois");
    }

    #[tokio::test]
    async fn empty_orchestra_closes_immediately() {
        let space = space_with_agents(&[]);
        let mut rx = spawn(&space);
        assert!(rx.recv().await.is_none(), "aucun agent → canal fermé d'emblée");
    }
}
