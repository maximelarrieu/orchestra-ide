//! Runtime d'agents — fait vivre le radar.
//!
//! [`spawn`] démarre l'« orchestre » décrit par un [`ContextSpace`] : chaque agent
//! devient une tâche `tokio` qui publie des [`AgentEvent`] sur un canal
//! `tokio::sync::mpsc`. L'UI consomme le `Receiver` renvoyé, sans rien savoir des agents.
//!
//! **Phase 4a — LLM + Skills Dev.** Si `ANTHROPIC_API_KEY` est présente, chaque agent
//! mène une vraie boucle agentique Claude (tool use → exécution des Skills Dev →
//! résultat → …). Sinon — ou si l'API est injoignable — on retombe sur un flux *simulé*
//! (Phase 3) pour que l'appli reste pleinement fonctionnelle hors-ligne. La signature de
//! [`spawn`] n'a pas changé depuis la Phase 3.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::time::sleep;

use crate::events::AgentEvent;
use crate::integrations::{self, IntegrationConn};
use crate::llm::{Block, LlmClient, Msg, ToolResult};
use crate::model::project_type::ProjectType;
use crate::model::space::ContextSpace;
use crate::skills;

/// Nombre maximal de tours LLM ↔ outils par agent (garde-fou anti-boucle).
const MAX_TURNS: usize = 6;

/// Contexte transmis à chaque tâche agent (clonable, `Send`).
#[derive(Clone)]
struct AgentContext {
    project_name: String,
    project_type: ProjectType,
    persona: Option<String>,
    workspace: PathBuf,
    skills: Vec<String>,
    integ: IntegrationConn,
}

impl AgentContext {
    fn from_space(space: &ContextSpace) -> Self {
        let workspace = space
            .config
            .workspace_path
            .clone()
            .unwrap_or_else(|| space.root.clone());
        Self {
            project_name: space.config.project_name.clone(),
            project_type: space.config.project_type,
            persona: space.persona.clone(),
            workspace,
            skills: space.config.skills.clone(),
            integ: IntegrationConn::from_space(space),
        }
    }
}

/// Lance tous les agents de l'espace et renvoie le flux de leurs événements.
///
/// Le canal se ferme (`recv()` → `None`) quand tous les agents ont terminé.
pub fn spawn(space: &ContextSpace) -> UnboundedReceiver<AgentEvent> {
    spawn_inner(space, LlmClient::from_env().map(Arc::new))
}

/// Cœur testable de [`spawn`] : le client LLM est injecté (les tests passent `None` pour
/// rester hors-ligne et déterministes).
fn spawn_inner(space: &ContextSpace, client: Option<Arc<LlmClient>>) -> UnboundedReceiver<AgentEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    let ctx = AgentContext::from_space(space);

    for (idx, agent) in space.config.agents.iter().cloned().enumerate() {
        let tx = tx.clone();
        let ctx = ctx.clone();
        let client = client.clone();
        tokio::spawn(run_agent(agent, idx, ctx, client, tx));
    }
    rx
}

/// Cycle de vie d'un agent : `Started`, puis travail réel ou simulé, puis `Done`.
async fn run_agent(
    agent: String,
    idx: usize,
    ctx: AgentContext,
    client: Option<Arc<LlmClient>>,
    tx: UnboundedSender<AgentEvent>,
) {
    // Décalage de démarrage : les agents n'entrent pas tous en scène en même temps.
    sleep(Duration::from_millis(150 * idx as u64)).await;
    let _ = tx.send(AgentEvent::Started { agent: agent.clone() });

    let handled = match &client {
        Some(c) => match llm_agent_loop(c, &agent, &ctx, &tx).await {
            Ok(()) => true,
            Err(e) => {
                // Repli : l'API a échoué (réseau, quota…) → flux simulé pour ne pas figer.
                let _ = tx.send(AgentEvent::Log {
                    agent: agent.clone(),
                    msg: format!("⚠ LLM injoignable ({e}) — bascule en mode simulé"),
                });
                false
            }
        },
        None => false,
    };

    if !handled {
        simulate_agent(&agent, &tx).await;
    }

    let _ = tx.send(AgentEvent::Done { agent });
}

/// Boucle agentique réelle (provider-agnostique) : le modèle raisonne, demande des outils
/// (Skills Dev), on les exécute, on lui renvoie les résultats, jusqu'à la fin ou la limite
/// de tours.
async fn llm_agent_loop(
    client: &LlmClient,
    agent: &str,
    ctx: &AgentContext,
    tx: &UnboundedSender<AgentEvent>,
) -> Result<(), crate::llm::LlmError> {
    let system = build_system_prompt(agent, ctx);
    let mut tools = skills::dev_tool_definitions(&ctx.skills);
    tools.extend(integrations::tool_definitions(&ctx.integ)); // Git/GitHub si configurés
    let mut conv: Vec<Msg> = vec![Msg::User(
        "Avance concrètement sur l'objectif de cet espace. Utilise tes outils quand c'est \
         utile et explique brièvement chaque action."
            .to_string(),
    )];

    for _ in 0..MAX_TURNS {
        let blocks = client.complete(&system, &tools, &conv).await?;

        // Émettre le texte et repérer les appels d'outils.
        let mut calls: Vec<(String, String, Value)> = Vec::new();
        for block in &blocks {
            match block {
                Block::Text(t) => emit_log(tx, agent, t.trim()),
                Block::ToolUse { id, name, input } => {
                    emit_log(tx, agent, &format!("🔧 {name} {}", brief(input)));
                    calls.push((id.clone(), name.clone(), input.clone()));
                }
            }
        }

        if calls.is_empty() {
            return Ok(()); // l'agent a fini (réponse finale, sans nouvel outil)
        }

        // Réinjecter le tour assistant, puis exécuter les outils demandés.
        conv.push(Msg::Assistant(blocks));
        let mut results = Vec::with_capacity(calls.len());
        for (id, name, input) in calls {
            let outcome = if integrations::handles(&name) {
                integrations::execute(&name, &input, &ctx.workspace, &ctx.integ).await
            } else {
                skills::execute_skill(&name, &input, &ctx.workspace).await
            };
            results.push(ToolResult { id, name, content: outcome.text, is_error: outcome.is_error });
        }
        conv.push(Msg::Tool(results));
    }

    emit_log(tx, agent, "limite de tours atteinte — arrêt.");
    Ok(())
}

/// Construit le prompt système d'un agent à partir de l'espace de contexte.
fn build_system_prompt(agent: &str, ctx: &AgentContext) -> String {
    let mut s = format!(
        "Tu es « {agent} », un agent de l'orchestre Orchestra IDE travaillant sur le projet \
         « {} » (type : {}). Réponds en français, de façon concise. Mène la tâche à son terme \
         en utilisant tes outils quand c'est pertinent ; n'invente pas de résultats d'outils.",
        ctx.project_name,
        ctx.project_type.label(),
    );
    if let Some(persona) = &ctx.persona {
        s.push_str("\n\n## Contexte / persona\n");
        s.push_str(persona);
    }
    s
}

/// Émet une ligne de log non vide, en limitant la longueur affichée sur le radar.
fn emit_log(tx: &UnboundedSender<AgentEvent>, agent: &str, msg: &str) {
    if msg.is_empty() {
        return;
    }
    let msg = msg.lines().next().unwrap_or(msg);
    let msg = if msg.chars().count() > 200 {
        format!("{}…", msg.chars().take(200).collect::<String>())
    } else {
        msg.to_string()
    };
    let _ = tx.send(AgentEvent::Log { agent: agent.to_string(), msg });
}

/// Résumé court de l'input d'un outil pour l'affichage radar.
fn brief(input: &Value) -> String {
    let s = input.to_string();
    if s.chars().count() > 80 {
        format!("{}…", s.chars().take(80).collect::<String>())
    } else {
        s
    }
}

// --- Mode simulé (Phase 3) : repli hors-ligne ---------------------------------------

/// Joue un scénario scripté étalé dans le temps (utilisé sans clé API ou si l'API échoue).
async fn simulate_agent(agent: &str, tx: &UnboundedSender<AgentEvent>) {
    for (step, msg) in scripted_steps(agent).iter().enumerate() {
        sleep(Duration::from_millis(250 + 150 * step as u64)).await;
        let _ = tx.send(AgentEvent::Log {
            agent: agent.to_string(),
            msg: (*msg).to_string(),
        });
    }
    sleep(Duration::from_millis(200)).await;
}

/// Lignes de log jouées par un agent simulé, choisies d'après un mot-clé de son nom.
fn scripted_steps(agent: &str) -> &'static [&'static str] {
    let key = agent.to_lowercase();
    if key.contains("scrap") {
        &["connexion aux sources…", "3 pages parcourues", "27 annonces extraites"]
    } else if key.contains("filtr") {
        &["application des critères stricts…", "27 → 6 annonces retenues", "tri par pertinence terminé"]
    } else if key.contains("cod") {
        &["lecture du contexte projet…", "génération du patch proposé", "patch prêt pour relecture"]
    } else if key.contains("test") {
        &["exécution de la suite de tests…", "42 tests passés, 0 échec"]
    } else if key.contains("archi") {
        &["analyse des contraintes…", "plan d'implémentation rédigé"]
    } else {
        &["initialisation…", "traitement en cours…", "tâche accomplie"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::config::ProjectConfig;

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
    async fn each_agent_starts_and_finishes_once_offline() {
        // `None` → pas de client LLM : chemin simulé, déterministe et hors-ligne.
        let space = space_with_agents(&["Agent_Scraper", "Agent_Filtrage"]);
        let mut rx = spawn_inner(&space, None);

        let (mut started, mut done) = (0, 0);
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
        let mut rx = spawn_inner(&space, None);
        assert!(rx.recv().await.is_none(), "aucun agent → canal fermé d'emblée");
    }
}
