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

use serde_json::{json, Value};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::time::sleep;

use crate::events::AgentEvent;
use crate::integrations::{self, IntegrationConn};
use crate::llm::{Block, LlmClient, Msg, ToolResult, ToolSpec};
use crate::markdown_skill::MarkdownSkill;
use crate::memory;
use crate::model::project_type::ProjectType;
use crate::model::space::ContextSpace;
use crate::skills;

/// Nom affiché du chef d'orchestre dans le flux de conversation.
const COORDINATOR: &str = "Coordinateur";

/// Nombre maximal de tours LLM ↔ outils par agent (garde-fou anti-boucle).
const MAX_TURNS: usize = 6;

/// Un agent prêt à tourner : nom, rôle, skills, et drapeau Documentaliste.
#[derive(Clone)]
struct RosterAgent {
    name: String,
    role: String,
    skills: Vec<String>,
    documentalist: bool,
}

/// Construit le roster d'un espace : agents configurés + Documentaliste si activé.
fn roster(space: &ContextSpace) -> Vec<RosterAgent> {
    let mut roster: Vec<RosterAgent> = space
        .config
        .agents
        .iter()
        .map(|a| RosterAgent {
            name: a.name.clone(),
            role: a.role.clone(),
            skills: a.skills.clone(),
            documentalist: false,
        })
        .collect();
    if space.config.documentalist_enabled {
        roster.push(RosterAgent {
            name: "Agent_Documentaliste".to_string(),
            role: String::new(),
            skills: Vec::new(),
            documentalist: true,
        });
    }
    roster
}

/// Contexte transmis à chaque tâche agent (clonable, `Send`).
#[derive(Clone)]
struct AgentContext {
    project_name: String,
    project_type: ProjectType,
    persona: Option<String>,
    /// Racine de l'espace (`.orchestra/` y vit, dont la mémoire partagée).
    root: PathBuf,
    workspace: PathBuf,
    skills: Vec<String>,
    /// Skills Markdown de l'espace (fiches d'instructions), partagés entre agents.
    md_skills: Arc<Vec<MarkdownSkill>>,
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
            root: space.root.clone(),
            workspace,
            skills: space.config.skills.clone(),
            md_skills: Arc::new(crate::markdown_skill::load_all(&space.root)),
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

    for (idx, agent) in roster(space).into_iter().enumerate() {
        let tx = tx.clone();
        let ctx = ctx.clone();
        let client = client.clone();
        tokio::spawn(run_agent(agent, idx, ctx, client, tx));
    }
    rx
}

/// Cycle de vie d'un agent : `Started`, puis travail réel ou simulé, puis `Done`.
async fn run_agent(
    agent: RosterAgent,
    idx: usize,
    ctx: AgentContext,
    client: Option<Arc<LlmClient>>,
    tx: UnboundedSender<AgentEvent>,
) {
    // Décalage de démarrage : les agents n'entrent pas tous en scène en même temps.
    sleep(Duration::from_millis(150 * idx as u64)).await;
    let _ = tx.send(AgentEvent::Started { agent: agent.name.clone() });

    let handled = match &client {
        Some(c) => match llm_agent_loop(c, &agent, &ctx, &tx).await {
            Ok(()) => true,
            Err(e) => {
                // Repli : l'API a échoué (réseau, quota…) → flux simulé pour ne pas figer.
                let _ = tx.send(AgentEvent::Log {
                    agent: agent.name.clone(),
                    msg: format!("⚠ LLM injoignable ({e}) — bascule en mode simulé"),
                });
                false
            }
        },
        None => false,
    };

    if !handled {
        simulate_agent(&agent.name, &tx).await;
    }

    let _ = tx.send(AgentEvent::Done { agent: agent.name });
}

/// Outils d'un agent : Documentaliste → outils doc ; sinon ses skills propres (repli sur
/// les skills de l'espace s'il n'en a pas) + intégrations Git/GitHub configurées.
fn agent_tools(agent: &RosterAgent, ctx: &AgentContext) -> Vec<ToolSpec> {
    let mut t = if agent.documentalist {
        skills::documentalist_tool_definitions()
    } else {
        let own = if agent.skills.is_empty() { &ctx.skills } else { &agent.skills };
        let mut v = skills::tool_specs(own); // skills exécutables assignés (registre)
        v.extend(integrations::tool_definitions(&ctx.integ)); // Git/GitHub si configurés
        v
    };
    t.extend(memory::tool_definitions()); // mémoire partagée : disponible pour tous les agents
    t
}

/// Boucle agentique autonome d'un agent (sous-tâche) : objectif implicite + tours.
async fn llm_agent_loop(
    client: &LlmClient,
    agent: &RosterAgent,
    ctx: &AgentContext,
    tx: &UnboundedSender<AgentEvent>,
) -> Result<(), crate::llm::LlmError> {
    let effective = if agent.skills.is_empty() { &ctx.skills } else { &agent.skills };
    let system = build_system_prompt(&agent.name, &agent.role, agent.documentalist, effective, ctx);
    let tools = agent_tools(agent, ctx);
    let documentalist = agent.documentalist;
    let intention = if documentalist {
        "Mets à jour la documentation du projet et produis/actualise les diagrammes Mermaid \
         pertinents. Lis les fichiers utiles, puis écris la doc et les diagrammes."
    } else {
        "Avance concrètement sur l'objectif de cet espace. Utilise tes outils quand c'est \
         utile et explique brièvement chaque action."
    };
    let mut conv: Vec<Msg> = vec![Msg::User(intention.to_string())];
    run_agent_turn(client, &system, &tools, &mut conv, &agent.name, ctx, tx).await?;
    Ok(())
}

/// Un « tour » agentique réutilisable (provider-agnostique) : le modèle raisonne, demande
/// des outils, on les exécute, on lui renvoie les résultats, jusqu'à une réponse finale ou
/// la limite de tours. Émet les événements sur `tx` et renvoie le texte produit (utile au
/// coordinateur pour récupérer le retour d'un sous-agent).
async fn run_agent_turn(
    client: &LlmClient,
    system: &str,
    tools: &[ToolSpec],
    conv: &mut Vec<Msg>,
    label: &str,
    ctx: &AgentContext,
    tx: &UnboundedSender<AgentEvent>,
) -> Result<String, crate::llm::LlmError> {
    let mut final_text = String::new();

    for _ in 0..MAX_TURNS {
        let _ = tx.send(AgentEvent::Thinking { agent: label.to_string() });
        let blocks = client.complete(system, tools, conv).await?;

        let mut calls: Vec<(String, String, Value)> = Vec::new();
        for block in &blocks {
            match block {
                Block::Text(t) => {
                    emit_log(tx, label, t.trim());
                    if !t.trim().is_empty() {
                        final_text.push_str(t.trim());
                        final_text.push('\n');
                    }
                }
                Block::ToolUse { id, name, input } => {
                    emit_log(tx, label, &format!("🔧 {name} {}", brief(input)));
                    calls.push((id.clone(), name.clone(), input.clone()));
                }
            }
        }

        if calls.is_empty() {
            return Ok(final_text); // réponse finale, sans nouvel outil
        }

        conv.push(Msg::Assistant(blocks));
        let mut results = Vec::with_capacity(calls.len());
        for (id, name, input) in calls {
            let outcome = if memory::handles(&name) {
                memory::execute(&name, &input, &ctx.root, label)
            } else if integrations::handles(&name) {
                integrations::execute(&name, &input, &ctx.workspace, &ctx.integ).await
            } else {
                skills::execute_skill(&name, &input, &ctx.workspace).await
            };
            results.push(ToolResult { id, name, content: outcome.text, is_error: outcome.is_error });
        }
        conv.push(Msg::Tool(results));
    }

    emit_log(tx, label, "limite de tours atteinte — arrêt.");
    Ok(final_text)
}

// --- Conversation avec le chef d'orchestre (coordinateur) ---------------------------

/// Poignée d'une conversation : on envoie des messages utilisateur sur `user`, on reçoit
/// les événements (réponses du coordinateur et activité des sous-agents) sur `events`.
/// Fermer `user` (le `Sender`) met fin à la conversation.
pub struct ChatHandle {
    pub user: UnboundedSender<String>,
    pub events: UnboundedReceiver<AgentEvent>,
}

/// Démarre une conversation avec le coordinateur de l'espace.
pub fn start_conversation(space: &ContextSpace) -> ChatHandle {
    start_conversation_inner(space, LlmClient::from_env().map(Arc::new))
}

/// Cœur testable : client LLM injecté (les tests passent `None`).
fn start_conversation_inner(space: &ContextSpace, client: Option<Arc<LlmClient>>) -> ChatHandle {
    let (user_tx, user_rx) = mpsc::unbounded_channel();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let ctx = AgentContext::from_space(space);
    let roster = roster(space);

    tokio::spawn(conversation_task(ctx, roster, client, user_rx, event_tx));
    ChatHandle { user: user_tx, events: event_rx }
}

/// Boucle de conversation : attend un message utilisateur, le confie au coordinateur, puis
/// recommence. Se termine quand le `Sender` utilisateur est fermé (l'UI quitte le chat).
async fn conversation_task(
    ctx: AgentContext,
    roster: Vec<RosterAgent>,
    client: Option<Arc<LlmClient>>,
    mut user_rx: UnboundedReceiver<String>,
    tx: UnboundedSender<AgentEvent>,
) {
    let _ = tx.send(AgentEvent::Started { agent: COORDINATOR.to_string() });
    let _ = tx.send(AgentEvent::Log {
        agent: COORDINATOR.to_string(),
        msg: "Prêt. Pose ta question ou donne ta consigne.".to_string(),
    });

    let system = coordinator_prompt(&ctx, &roster);
    let tools: Vec<ToolSpec> = roster.iter().map(delegation_tool).collect();
    let mut conv: Vec<Msg> = Vec::new();

    while let Some(user_msg) = user_rx.recv().await {
        // Écho du message utilisateur pour une lecture « chat » du flux.
        let _ = tx.send(AgentEvent::Log { agent: "Vous".to_string(), msg: user_msg.clone() });

        match &client {
            Some(c) => {
                conv.push(Msg::User(user_msg));
                if let Err(e) =
                    run_coordinator_turn(c, &system, &tools, &mut conv, &ctx, &roster, &tx).await
                {
                    emit_log(&tx, COORDINATOR, &format!("⚠ LLM injoignable ({e})"));
                }
            }
            None => emit_log(
                &tx,
                COORDINATOR,
                "(mode simulé) Définis ANTHROPIC_API_KEY ou GEMINI_API_KEY pour une vraie conversation.",
            ),
        }
    }

    let _ = tx.send(AgentEvent::Done { agent: COORDINATOR.to_string() });
}

/// Un tour du coordinateur : il répond et/ou délègue à des sous-agents (chaque agent est
/// exposé comme un outil). Boucle jusqu'à une réponse finale à l'utilisateur.
async fn run_coordinator_turn(
    client: &LlmClient,
    system: &str,
    tools: &[ToolSpec],
    conv: &mut Vec<Msg>,
    ctx: &AgentContext,
    roster: &[RosterAgent],
    tx: &UnboundedSender<AgentEvent>,
) -> Result<(), crate::llm::LlmError> {
    for _ in 0..MAX_TURNS {
        let _ = tx.send(AgentEvent::Thinking { agent: COORDINATOR.to_string() });
        let blocks = client.complete(system, tools, conv).await?;

        let mut calls: Vec<(String, String, Value)> = Vec::new();
        for block in &blocks {
            match block {
                Block::Text(t) => emit_log(tx, COORDINATOR, t.trim()),
                Block::ToolUse { id, name, input } => {
                    emit_log(tx, COORDINATOR, &format!("→ délègue à {name}"));
                    calls.push((id.clone(), name.clone(), input.clone()));
                }
            }
        }

        if calls.is_empty() {
            return Ok(()); // le coordinateur a répondu à l'utilisateur
        }

        conv.push(Msg::Assistant(blocks));
        let mut results = Vec::with_capacity(calls.len());
        for (id, agent, input) in calls {
            let instruction = input
                .get("instruction")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let text = run_subagent(client, ctx, roster, &agent, &instruction, tx)
                .await
                .unwrap_or_else(|e| format!("(échec du sous-agent : {e})"));
            results.push(ToolResult { id, name: agent, content: text, is_error: false });
        }
        conv.push(Msg::Tool(results));
    }
    Ok(())
}

/// Exécute un sous-agent sur une instruction du coordinateur, en émettant son activité, et
/// renvoie le texte produit (transmis au coordinateur comme résultat d'outil).
async fn run_subagent(
    client: &LlmClient,
    ctx: &AgentContext,
    roster: &[RosterAgent],
    name: &str,
    instruction: &str,
    tx: &UnboundedSender<AgentEvent>,
) -> Result<String, crate::llm::LlmError> {
    // Agent connu du roster (sinon, agent minimal au cas où le modèle invente un nom).
    let fallback = RosterAgent { name: name.to_string(), role: String::new(), skills: Vec::new(), documentalist: false };
    let agent = roster.iter().find(|a| a.name == name).unwrap_or(&fallback);
    let _ = tx.send(AgentEvent::Started { agent: name.to_string() });

    let effective = if agent.skills.is_empty() { &ctx.skills } else { &agent.skills };
    let system = build_system_prompt(&agent.name, &agent.role, agent.documentalist, effective, ctx);
    let tools = agent_tools(agent, ctx);
    let mut conv: Vec<Msg> = vec![Msg::User(instruction.to_string())];
    let text = run_agent_turn(client, &system, &tools, &mut conv, name, ctx, tx).await;

    let _ = tx.send(AgentEvent::Done { agent: name.to_string() });
    text
}

/// Outil de délégation exposé au coordinateur pour solliciter un agent (un par agent).
fn delegation_tool(agent: &RosterAgent) -> ToolSpec {
    let name = &agent.name;
    let role = if agent.role.is_empty() { "agent spécialisé" } else { &agent.role };
    ToolSpec {
        name: name.to_string(),
        description: format!(
            "Délègue une tâche à l'agent « {name} » ({role}). Fournis une instruction claire et autonome ; \
             tu recevras son compte rendu."
        ),
        parameters: json!({
            "type": "object",
            "properties": { "instruction": { "type": "string", "description": "Instruction pour l'agent" } },
            "required": ["instruction"]
        }),
    }
}

/// Prompt système du coordinateur : rôle + roster des agents délégables (avec leur rôle).
fn coordinator_prompt(ctx: &AgentContext, roster: &[RosterAgent]) -> String {
    let agents = roster
        .iter()
        .map(|a| {
            if a.role.is_empty() {
                format!("« {} »", a.name)
            } else {
                format!("« {} » ({})", a.name, a.role)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut s = format!(
        "Tu es le chef d'orchestre d'Orchestra IDE pour le projet « {} » (type : {}). Tu \
         dialogues en français avec l'utilisateur. Tu peux solliciter des agents spécialisés \
         via les outils (un par agent) : {agents}. Délègue quand c'est pertinent, synthétise \
         leurs retours, et pose des questions à l'utilisateur si besoin. Quand tu peux répondre \
         directement, fais-le sans outil. Sois concis.",
        ctx.project_name,
        ctx.project_type.label(),
    );
    if let Some(persona) = &ctx.persona {
        s.push_str("\n\n## Contexte / persona\n");
        s.push_str(persona);
    }
    s
}

/// Construit le prompt système d'un agent à partir de son nom, son rôle, ses skills
/// (les fiches Markdown assignées sont injectées sous « ## Compétences ») et l'espace.
fn build_system_prompt(
    name: &str,
    role: &str,
    documentalist: bool,
    skills: &[String],
    ctx: &AgentContext,
) -> String {
    let mut s = if documentalist {
        format!(
            "Tu es l'Agent Documentaliste d'Orchestra IDE pour le projet « {} » (type : {}). \
             Ta mission : maintenir la documentation à jour et produire des diagrammes Mermaid \
             clairs. Lis les fichiers pertinents, puis écris/mets à jour la doc \
             (Write_File_Validated) et les diagrammes (Write_Mermaid_Diagram). Réponds en \
             français, de façon concise ; n'invente pas de résultats d'outils.",
            ctx.project_name,
            ctx.project_type.label(),
        )
    } else {
        let mut s = format!(
            "Tu es « {name} », un agent de l'orchestre Orchestra IDE travaillant sur le projet \
             « {} » (type : {}). Réponds en français, de façon concise. Mène la tâche à son terme \
             en utilisant tes outils quand c'est pertinent ; n'invente pas de résultats d'outils.",
            ctx.project_name,
            ctx.project_type.label(),
        );
        if !role.is_empty() {
            s.push_str(&format!("\n\n## Ton rôle\n{role}"));
        }
        s
    };
    // Compétences : instructions des skills Markdown assignés à l'agent (le « comment faire »).
    let mut added_skills = false;
    for sk in skills {
        if let Some(md) = ctx.md_skills.iter().find(|m| m.id == *sk || m.name == *sk) {
            if !added_skills {
                s.push_str("\n\n## Compétences");
                added_skills = true;
            }
            s.push_str(&format!("\n\n### {}", md.name));
            if !md.description.is_empty() {
                s.push_str(&format!("\n_{}_", md.description));
            }
            s.push('\n');
            s.push_str(md.instructions.trim());
        }
    }
    // Mémoire partagée : rappel court seulement (pas le contenu — lu à la demande via `Recall`,
    // ce qui économise le contexte).
    s.push_str(
        "\n\n## Mémoire partagée\nTu partages une mémoire d'espace avec les autres agents. \
         Avant d'agir, utilise `Recall` (avec un mot-clé) pour relire les acquis ; consigne via \
         `Remember` tout fait, décision ou synthèse utile aux autres — plutôt que de refaire le travail.",
    );
    if let Some(persona) = &ctx.persona {
        s.push_str("\n\n## Contexte / persona\n");
        s.push_str(persona);
    }
    s
}

/// Émet une ligne de log non vide. On conserve le **texte complet** (multi-ligne) ; le
/// rendu (côté UI) se charge du retour à la ligne et du défilement. Un plafond large évite
/// seulement les cas pathologiques.
fn emit_log(tx: &UnboundedSender<AgentEvent>, agent: &str, msg: &str) {
    let msg = msg.trim();
    if msg.is_empty() {
        return;
    }
    let msg = if msg.chars().count() > 4000 {
        format!("{}…", msg.chars().take(4000).collect::<String>())
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
    } else if key.contains("documental") {
        &["lecture des sources…", "mise à jour de la doc", "diagramme Mermaid généré"]
    } else {
        &["initialisation…", "traitement en cours…", "tâche accomplie"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::config::{AgentDef, ProjectConfig};

    fn space_with_agents(agents: &[&str]) -> ContextSpace {
        ContextSpace {
            root: PathBuf::from("."),
            config: ProjectConfig {
                project_name: "Test".to_string(),
                project_type: ProjectType::Dev,
                workspace_path: None,
                documentalist_enabled: false,
                skills: vec![],
                agents: agents.iter().map(|s| AgentDef::new(*s)).collect(),
                integrations: Default::default(),
            },
            persona: None,
            adrs: vec![],
        }
    }

    #[test]
    fn memory_tools_exposed_to_every_agent() {
        let space = space_with_agents(&["Agent_Scraper"]);
        let ctx = AgentContext::from_space(&space);
        let agents = roster(&space);
        let names: Vec<_> = agent_tools(&agents[0], &ctx)
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert!(names.iter().any(|n| n == memory::REMEMBER));
        assert!(names.iter().any(|n| n == memory::RECALL));
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
                AgentEvent::Log { .. } | AgentEvent::Thinking { .. } => {}
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

    #[tokio::test]
    async fn conversation_echoes_user_and_replies_offline() {
        let space = space_with_agents(&["Agent_Tuteur"]);
        let ChatHandle { user, mut events } = start_conversation_inner(&space, None);

        user.send("bonjour".to_string()).unwrap();
        drop(user); // fin de conversation après traitement du message

        let mut saw_user = false;
        let mut saw_coordinator = false;
        while let Some(ev) = events.recv().await {
            if let AgentEvent::Log { agent, .. } = ev {
                saw_user |= agent == "Vous";
                saw_coordinator |= agent == COORDINATOR;
            }
        }
        assert!(saw_user, "le message utilisateur est ré-émis dans le flux");
        assert!(saw_coordinator, "le coordinateur répond (mode simulé hors-ligne)");
    }

    #[tokio::test]
    async fn documentalist_adds_one_agent_when_enabled() {
        // Espace sans agent classique, mais Documentaliste activé → exactement 1 agent.
        let mut space = space_with_agents(&[]);
        space.config.documentalist_enabled = true;
        let mut rx = spawn_inner(&space, None);

        let (mut started, mut done) = (0, 0);
        let mut saw_doc = false;
        while let Some(ev) = rx.recv().await {
            match ev {
                AgentEvent::Started { agent } => {
                    started += 1;
                    saw_doc |= agent.contains("Documentaliste");
                }
                AgentEvent::Done { .. } => done += 1,
                AgentEvent::Log { .. } | AgentEvent::Thinking { .. } => {}
            }
        }
        assert_eq!((started, done), (1, 1));
        assert!(saw_doc, "l'Agent Documentaliste doit être lancé");
    }
}
