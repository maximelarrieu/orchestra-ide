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

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::time::sleep;

use crate::events::{AgentEvent, PlannedTask};
use crate::integrations::{self, IntegrationConn};
use crate::llm::{Block, LlmClient, Msg, ToolResult, ToolSpec};
use crate::markdown_skill::{self, MarkdownSkill};
use crate::memory;
use crate::model::project_type::ProjectType;
use crate::orchestration::{self, Plan};
use crate::model::space::ContextSpace;
use crate::skills;

/// Nom affiché du chef d'orchestre dans le flux de conversation.
const COORDINATOR: &str = "Coordinateur";

/// Nombre maximal de tours LLM ↔ outils par agent (garde-fou anti-boucle).
const MAX_TURNS: usize = 6;

/// Nombre maximal de manches de planification (plan initial + manches correctives) — garde-fou
/// de la re-planification itérative.
const MAX_ROUNDS: usize = 3;

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
        // Divulgation progressive : `Load_Skill` n'est exposé que si l'agent a au moins une fiche.
        if own.iter().any(|sk| ctx.md_skills.iter().any(|m| m.id == *sk || m.name == *sk)) {
            v.push(markdown_skill::tool_definition());
        }
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
            } else if markdown_skill::handles(&name) {
                markdown_skill::execute(&input, &ctx.root)
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
/// les événements sur `events`, et on **approuve** un plan proposé par l'outil `orchestrate`
/// du coordinateur via `approve` (`true` = exécuter). Fermer `user` met fin à la conversation.
pub struct ChatHandle {
    pub user: UnboundedSender<String>,
    pub events: UnboundedReceiver<AgentEvent>,
    pub approve: UnboundedSender<bool>,
}

/// Démarre une conversation avec le coordinateur de l'espace.
pub fn start_conversation(space: &ContextSpace) -> ChatHandle {
    start_conversation_inner(space, LlmClient::from_env().map(Arc::new))
}

/// Cœur testable : client LLM injecté (les tests passent `None`).
fn start_conversation_inner(space: &ContextSpace, client: Option<Arc<LlmClient>>) -> ChatHandle {
    let (user_tx, user_rx) = mpsc::unbounded_channel();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let (approve_tx, approve_rx) = mpsc::unbounded_channel();
    let ctx = AgentContext::from_space(space);
    let roster = roster(space);

    tokio::spawn(conversation_task(ctx, roster, client, user_rx, approve_rx, event_tx));
    ChatHandle { user: user_tx, events: event_rx, approve: approve_tx }
}

/// Boucle de conversation : attend un message utilisateur, le confie au coordinateur, puis
/// recommence. Se termine quand le `Sender` utilisateur est fermé (l'UI quitte le chat).
async fn conversation_task(
    ctx: AgentContext,
    roster: Vec<RosterAgent>,
    client: Option<Arc<LlmClient>>,
    mut user_rx: UnboundedReceiver<String>,
    mut approve_rx: UnboundedReceiver<bool>,
    tx: UnboundedSender<AgentEvent>,
) {
    let _ = tx.send(AgentEvent::Started { agent: COORDINATOR.to_string() });
    let _ = tx.send(AgentEvent::Log {
        agent: COORDINATOR.to_string(),
        msg: "Prêt. Pose ta question ou donne ta consigne.".to_string(),
    });

    let system = coordinator_prompt(&ctx, &roster);
    // Outils : un par agent (délégation) + `orchestrate` (plan complet validé/exécuté/corrigé).
    let mut tools: Vec<ToolSpec> = roster.iter().map(delegation_tool).collect();
    tools.push(orchestrate_tool());
    let mut conv: Vec<Msg> = Vec::new();

    while let Some(user_msg) = user_rx.recv().await {
        // Écho du message utilisateur pour une lecture « chat » du flux.
        let _ = tx.send(AgentEvent::Log { agent: "Vous".to_string(), msg: user_msg.clone() });

        match &client {
            Some(c) => {
                conv.push(Msg::User(user_msg));
                if let Err(e) =
                    run_coordinator_turn(c, &system, &tools, &mut conv, &ctx, &roster, &mut approve_rx, &tx).await
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

/// Un tour du coordinateur : il répond, délègue à des sous-agents, ou orchestre un objectif
/// complet. Boucle jusqu'à une réponse finale à l'utilisateur.
#[allow(clippy::too_many_arguments)] // boucle agentique : dépendances explicites assumées
async fn run_coordinator_turn(
    client: &LlmClient,
    system: &str,
    tools: &[ToolSpec],
    conv: &mut Vec<Msg>,
    ctx: &AgentContext,
    roster: &[RosterAgent],
    approve_rx: &mut UnboundedReceiver<bool>,
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
                    let what = if name == ORCHESTRATE { "→ orchestration d'un objectif".to_string() } else { format!("→ délègue à {name}") };
                    emit_log(tx, COORDINATOR, &what);
                    calls.push((id.clone(), name.clone(), input.clone()));
                }
            }
        }

        if calls.is_empty() {
            return Ok(()); // le coordinateur a répondu à l'utilisateur
        }

        conv.push(Msg::Assistant(blocks));
        let mut results = Vec::with_capacity(calls.len());
        for (id, name, input) in calls {
            let text = if name == ORCHESTRATE {
                // Orchestration complète inline : plan → approbation → exécution → synthèse.
                let objective = input.get("objective").and_then(Value::as_str).unwrap_or("").to_string();
                run_orchestration(Some(client), ctx, roster, &objective, approve_rx, tx).await
            } else {
                let instruction = input.get("instruction").and_then(Value::as_str).unwrap_or("").to_string();
                run_subagent(client, ctx, roster, &name, &instruction, tx)
                    .await
                    .unwrap_or_else(|e| format!("(échec du sous-agent : {e})"))
            };
            results.push(ToolResult { id, name, content: text, is_error: false });
        }
        conv.push(Msg::Tool(results));
    }
    Ok(())
}

/// Nom de l'outil d'orchestration exposé au coordinateur du chat.
const ORCHESTRATE: &str = "orchestrate";

/// Outil `orchestrate` : déclenche une orchestration complète d'un objectif depuis le chat.
fn orchestrate_tool() -> ToolSpec {
    ToolSpec {
        name: ORCHESTRATE.to_string(),
        description:
            "Planifie et exécute un objectif complexe en plusieurs étapes coordonnées (plan \
             validé par l'utilisateur, exécution parallèle, auto-correction). À utiliser pour une \
             demande nécessitant plusieurs agents ou étapes ; pour une demande simple, réponds \
             directement ou délègue à un seul agent."
                .to_string(),
        parameters: json!({
            "type": "object",
            "properties": { "objective": { "type": "string", "description": "L'objectif à orchestrer" } },
            "required": ["objective"]
        }),
    }
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
    // `name` est le slug d'outil reçu du coordinateur : on retrouve l'agent par son slug
    // (sinon agent minimal, au cas où le modèle invente un nom). Les événements/le label
    // utilisent le **vrai** nom d'agent pour rester cohérents avec le reste de l'UI.
    let fallback = RosterAgent { name: name.to_string(), role: String::new(), skills: Vec::new(), documentalist: false };
    let agent = roster.iter().find(|a| tool_slug(&a.name) == name).unwrap_or(&fallback);
    let label = agent.name.clone();
    let _ = tx.send(AgentEvent::Started { agent: label.clone() });

    let effective = if agent.skills.is_empty() { &ctx.skills } else { &agent.skills };
    let system = build_system_prompt(&agent.name, &agent.role, agent.documentalist, effective, ctx);
    let tools = agent_tools(agent, ctx);
    let mut conv: Vec<Msg> = vec![Msg::User(instruction.to_string())];
    let text = run_agent_turn(client, &system, &tools, &mut conv, &label, ctx, tx).await;

    let _ = tx.send(AgentEvent::Done { agent: label });
    text
}

/// Outil de délégation exposé au coordinateur pour solliciter un agent (un par agent).
fn delegation_tool(agent: &RosterAgent) -> ToolSpec {
    let role = if agent.role.is_empty() { "agent spécialisé" } else { &agent.role };
    ToolSpec {
        // Le nom d'outil doit respecter `^[a-zA-Z0-9_-]{1,128}$` (contrainte API) : on slugifie
        // le nom d'agent (accents/espaces → « _ »). Le vrai nom reste dans la description.
        name: tool_slug(&agent.name),
        description: format!(
            "Délègue une tâche à l'agent « {} » ({role}). Fournis une instruction claire et autonome ; \
             tu recevras son compte rendu.",
            agent.name
        ),
        parameters: json!({
            "type": "object",
            "properties": { "instruction": { "type": "string", "description": "Instruction pour l'agent" } },
            "required": ["instruction"]
        }),
    }
}

/// Normalise un nom d'agent en identifiant d'outil valide pour l'API LLM
/// (`^[a-zA-Z0-9_-]{1,128}$`) : tout caractère hors ASCII alphanumérique / `_` / `-` devient
/// « _ ». Résultat non vide et borné à 128 caractères (tous ASCII → troncature sûre).
fn tool_slug(name: &str) -> String {
    let mut slug: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    if slug.is_empty() {
        slug.push('_');
    }
    slug.truncate(128);
    slug
}

// --- Orchestration réelle : plan → (approbation) → exécution ordonnée → synthèse ---------

/// Poignée d'une orchestration : l'UI reçoit les événements (dont `PlanReady`) sur `events`,
/// et **approuve** (ou refuse) l'exécution du plan en envoyant `true`/`false` sur `approve`.
pub struct OrchestrationHandle {
    pub approve: UnboundedSender<bool>,
    pub events: UnboundedReceiver<AgentEvent>,
}

/// Orchestre un objectif : planifie, attend l'approbation, exécute les tâches en ordre
/// topologique (passage de relais via la mémoire) puis synthétise.
pub fn orchestrate(space: &ContextSpace, objective: &str) -> OrchestrationHandle {
    orchestrate_inner(space, objective, LlmClient::from_env().map(Arc::new))
}

/// Cœur testable : client LLM injecté (les tests passent `None` → plan de repli + simulé).
fn orchestrate_inner(
    space: &ContextSpace,
    objective: &str,
    client: Option<Arc<LlmClient>>,
) -> OrchestrationHandle {
    let (approve_tx, approve_rx) = mpsc::unbounded_channel();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let ctx = AgentContext::from_space(space);
    let roster = roster(space);
    tokio::spawn(orchestration_task(ctx, roster, client, objective.to_string(), approve_rx, event_tx));
    OrchestrationHandle { approve: approve_tx, events: event_rx }
}

async fn orchestration_task(
    ctx: AgentContext,
    roster: Vec<RosterAgent>,
    client: Option<Arc<LlmClient>>,
    objective: String,
    mut approve_rx: UnboundedReceiver<bool>,
    tx: UnboundedSender<AgentEvent>,
) {
    let _ = tx.send(AgentEvent::Started { agent: COORDINATOR.to_string() });
    let synthesis = run_orchestration(client.as_deref(), &ctx, &roster, &objective, &mut approve_rx, &tx).await;
    if !synthesis.trim().is_empty() {
        emit_log(&tx, COORDINATOR, &synthesis);
    }
    let _ = tx.send(AgentEvent::Done { agent: COORDINATOR.to_string() });
}

/// Boucle d'orchestration réutilisable (par `[1]` et par l'outil `orchestrate` du chat) :
/// plan → approbation → exécution par vagues → évaluation/re-planification (bornée par
/// `MAX_ROUNDS`) → synthèse. Émet PlanReady / Task* / logs, et **renvoie** la synthèse (le
/// caller décide comment la présenter). N'émet pas les événements `Started`/`Done` du coordinateur.
async fn run_orchestration(
    client: Option<&LlmClient>,
    ctx: &AgentContext,
    roster: &[RosterAgent],
    objective: &str,
    approve_rx: &mut UnboundedReceiver<bool>,
    tx: &UnboundedSender<AgentEvent>,
) -> String {
    emit_log(tx, COORDINATOR, "Planification de l'objectif…");
    let mut plan = plan_objective(client, ctx, roster, objective, tx).await;
    let mut transcript: Vec<(String, String)> = Vec::new();
    let mut round = 1usize;

    loop {
        if plan.is_empty() {
            emit_log(tx, COORDINATOR, "Aucune tâche à exécuter.");
            break;
        }
        // Proposer le plan (initial ou correctif) et attendre l'approbation utilisateur.
        let _ = tx.send(AgentEvent::PlanReady { tasks: plan_snapshot(&plan) });
        match approve_rx.recv().await {
            Some(true) => {}
            _ => {
                let msg = if round == 1 { "Plan annulé." } else { "Manche corrective annulée." };
                emit_log(tx, COORDINATOR, msg);
                break;
            }
        }

        // Exécuter la manche (vagues concurrentes) et accumuler le transcript.
        let mut round_out = run_waves(client, ctx, roster, &plan, tx).await;
        transcript.append(&mut round_out);

        // Re-planification itérative : uniquement avec LLM et dans la limite des manches.
        let Some(c) = client else { break };
        if round >= MAX_ROUNDS {
            emit_log(tx, COORDINATOR, "Limite de manches atteinte — synthèse de l'état courant.");
            break;
        }
        match evaluate_objective(c, ctx, roster, objective, &transcript).await {
            Some(corrective) => {
                round += 1;
                emit_log(tx, COORDINATOR, &format!("Objectif non atteint — manche corrective {round}."));
                plan = corrective;
                continue;
            }
            None => {
                emit_log(tx, COORDINATOR, "Objectif jugé atteint.");
                break;
            }
        }
    }

    synthesize(client, ctx, &transcript).await
}

/// Aperçu d'un plan pour l'UI (panneau Plan).
fn plan_snapshot(plan: &Plan) -> Vec<PlannedTask> {
    plan.tasks
        .iter()
        .map(|t| PlannedTask {
            id: t.id.clone(),
            agent: t.agent.clone(),
            objective: t.objective.clone(),
            depends_on: t.depends_on.clone(),
        })
        .collect()
}

/// Établit un plan : via le LLM (outil `submit_plan`) si une clé est présente et que le plan
/// est valide, sinon pipeline linéaire de repli sur le roster.
async fn plan_objective(
    client: Option<&LlmClient>,
    ctx: &AgentContext,
    roster: &[RosterAgent],
    objective: &str,
    tx: &UnboundedSender<AgentEvent>,
) -> Plan {
    let agents: Vec<String> = roster.iter().map(|a| a.name.clone()).collect();
    if let Some(c) = client {
        if let Some(plan) = plan_via_llm(c, ctx, roster, objective).await {
            match plan.validate(&agents) {
                Ok(()) => {
                    emit_log(tx, COORDINATOR, &format!("Plan établi : {} étape(s).", plan.tasks.len()));
                    return plan;
                }
                Err(e) => emit_log(tx, COORDINATOR, &format!("Plan LLM invalide ({e}) — repli linéaire.")),
            }
        }
    }
    orchestration::fallback_plan(&agents, objective)
}

/// Demande au LLM de décomposer l'objectif via l'outil `submit_plan`.
async fn plan_via_llm(
    client: &LlmClient,
    ctx: &AgentContext,
    roster: &[RosterAgent],
    objective: &str,
) -> Option<Plan> {
    let agents_desc = roster
        .iter()
        .map(|a| if a.role.is_empty() { format!("- {}", a.name) } else { format!("- {} ({})", a.name, a.role) })
        .collect::<Vec<_>>()
        .join("\n");
    let system = format!(
        "Tu es le chef d'orchestre du projet « {} » (type : {}). Décompose l'objectif en tâches \
         confiées aux agents ci-dessous (utilise EXACTEMENT ces noms), reliées par `depends_on` \
         (ids des tâches prérequises). Une tâche par contribution réelle, pas plus. Appelle \
         l'outil `submit_plan`.\n\nAgents :\n{agents_desc}",
        ctx.project_name,
        ctx.project_type.label(),
    );
    let blocks = client.complete(&system, &[submit_plan_tool()], &[Msg::User(objective.to_string())]).await.ok()?;
    for b in blocks {
        if let Block::ToolUse { name, input, .. } = b {
            if name == "submit_plan" {
                return orchestration::parse_plan(&input);
            }
        }
    }
    None
}

/// Définition de l'outil `submit_plan` partagée par la planification et l'évaluation.
fn submit_plan_tool() -> ToolSpec {
    ToolSpec {
        name: "submit_plan".to_string(),
        description: "Soumets le plan décomposé en tâches ordonnées par dépendances.".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string", "description": "Identifiant court, ex. t1" },
                            "agent": { "type": "string", "description": "Nom exact de l'agent assigné" },
                            "objective": { "type": "string", "description": "Ce que l'agent doit accomplir" },
                            "depends_on": { "type": "array", "items": { "type": "string" }, "description": "Ids des tâches prérequises" }
                        },
                        "required": ["id", "agent", "objective"]
                    }
                }
            },
            "required": ["tasks"]
        }),
    }
}

/// Évalue si l'objectif est atteint au vu du transcript. Renvoie `Some(plan correctif)` si des
/// tâches manquent (manche supplémentaire), `None` si l'objectif est jugé atteint.
async fn evaluate_objective(
    client: &LlmClient,
    ctx: &AgentContext,
    roster: &[RosterAgent],
    objective: &str,
    transcript: &[(String, String)],
) -> Option<Plan> {
    let agents: Vec<String> = roster.iter().map(|a| a.name.clone()).collect();
    let agents_desc = roster
        .iter()
        .map(|a| if a.role.is_empty() { format!("- {}", a.name) } else { format!("- {} ({})", a.name, a.role) })
        .collect::<Vec<_>>()
        .join("\n");
    let joined = transcript
        .iter()
        .map(|(id, o)| format!("### {id}\n{}", clip(o, 1500)))
        .collect::<Vec<_>>()
        .join("\n\n");
    let system = format!(
        "Tu es le chef d'orchestre du projet « {} ». Au vu des comptes rendus, évalue si \
         l'objectif est ATTEINT. S'il l'est, réponds simplement « OK » sans appeler d'outil. \
         Sinon, appelle `submit_plan` avec UNIQUEMENT les tâches CORRECTIVES manquantes (mêmes \
         agents, nouveaux ids).\n\nObjectif : {objective}\n\nAgents :\n{agents_desc}",
        ctx.project_name,
    );
    let blocks = client
        .complete(&system, &[submit_plan_tool()], &[Msg::User(format!("Comptes rendus :\n\n{joined}"))])
        .await
        .ok()?;
    for b in blocks {
        if let Block::ToolUse { name, input, .. } = b {
            if name == "submit_plan" {
                if let Some(plan) = orchestration::parse_plan(&input) {
                    if !plan.is_empty() && plan.validate(&agents).is_ok() {
                        return Some(plan); // tâches correctives → nouvelle manche
                    }
                }
            }
        }
    }
    None // objectif atteint (ou correctif inexploitable → on s'arrête)
}

/// Exécute UNE manche du plan par **vagues concurrentes** : à chaque vague, toutes les tâches
/// dont les dépendances sont satisfaites s'exécutent **en parallèle** (les indépendantes
/// avancent ensemble). Chaque tâche reçoit en contexte les sorties de ses dépendances et trace
/// son résultat en mémoire (hand-off). Renvoie le transcript ordonné `(id, sortie)` de la manche
/// (la synthèse et la re-planification sont gérées par l'appelant).
async fn run_waves(
    client: Option<&LlmClient>,
    ctx: &AgentContext,
    roster: &[RosterAgent],
    plan: &Plan,
    tx: &UnboundedSender<AgentEvent>,
) -> Vec<(String, String)> {
    let total = plan.tasks.len();
    let mut outputs: HashMap<String, String> = HashMap::new();
    let mut transcript: Vec<(String, String)> = Vec::with_capacity(total);

    while transcript.len() < total {
        // Tâches prêtes : non terminées et dont toutes les dépendances sont terminées.
        let ready: Vec<&orchestration::Task> = plan
            .tasks
            .iter()
            .filter(|t| !outputs.contains_key(&t.id) && t.depends_on.iter().all(|d| outputs.contains_key(d)))
            .collect();
        if ready.is_empty() {
            emit_log(tx, COORDINATOR, "Plan bloqué (dépendances non satisfiables) — arrêt.");
            break;
        }

        // Une vague : on lance toutes les tâches prêtes concurremment (futures sur la même tâche
        // tokio — la concurrence suffit pour des appels LLM I/O-bound).
        let waves = ready.iter().map(|task| {
            let id = task.id.clone();
            let agent = task.agent.clone();
            let slug = tool_slug(&task.agent);
            // Instruction construite ici (contexte des dépendances déjà disponibles).
            let mut instruction = task.objective.clone();
            if !task.depends_on.is_empty() {
                instruction.push_str("\n\n## Contexte des étapes précédentes");
                for dep in &task.depends_on {
                    if let Some(out) = outputs.get(dep) {
                        instruction.push_str(&format!("\n\n### {dep}\n{}", clip(out, 1500)));
                    }
                }
            }
            async move {
                let _ = tx.send(AgentEvent::TaskStarted { id: id.clone(), agent: agent.clone() });
                let output = match client {
                    Some(c) => run_subagent(c, ctx, roster, &slug, &instruction, tx)
                        .await
                        .unwrap_or_else(|e| format!("(échec du sous-agent : {e})")),
                    None => {
                        // Hors-ligne : montrer le flux (Started/Done) sans appel LLM.
                        let _ = tx.send(AgentEvent::Started { agent: agent.clone() });
                        sleep(Duration::from_millis(120)).await;
                        emit_log(tx, &agent, "(simulé) — définis une clé API pour une exécution réelle.");
                        let _ = tx.send(AgentEvent::Done { agent: agent.clone() });
                        format!("(simulé) {agent} : {instruction}")
                    }
                };
                (id, agent, output)
            }
        });

        // `join_all` préserve l'ordre des entrées → traitement déterministe des résultats.
        for (id, agent, output) in futures::future::join_all(waves).await {
            let _ = memory::append(&ctx.root, &agent, &format!("[{id}] {}", first_line(&output)));
            let _ = tx.send(AgentEvent::TaskDone { id: id.clone() });
            outputs.insert(id.clone(), output.clone());
            transcript.push((id, output));
        }
    }

    transcript
}

/// Synthèse finale des comptes rendus (toutes manches confondues). **Renvoie** le texte (le
/// caller l'émet sur le radar ou le transmet au coordinateur du chat).
async fn synthesize(
    client: Option<&LlmClient>,
    ctx: &AgentContext,
    transcript: &[(String, String)],
) -> String {
    let joined = transcript
        .iter()
        .map(|(id, o)| format!("### {id}\n{}", clip(o, 2000)))
        .collect::<Vec<_>>()
        .join("\n\n");
    match client {
        Some(c) => {
            let system = format!(
                "Tu es le chef d'orchestre du projet « {} ». Rédige en français, de façon concise, \
                 une synthèse finale des comptes rendus ci-dessous, orientée vers l'utilisateur.",
                ctx.project_name,
            );
            match c.complete(&system, &[], &[Msg::User(format!("Comptes rendus :\n\n{joined}"))]).await {
                Ok(blocks) => blocks
                    .into_iter()
                    .filter_map(|b| if let Block::Text(t) = b { Some(t.trim().to_string()) } else { None })
                    .collect::<Vec<_>>()
                    .join("\n"),
                Err(_) => joined, // repli : comptes rendus bruts
            }
        }
        None => format!("Synthèse (simulée) — étapes réalisées :\n{joined}"),
    }
}

/// Tronque une chaîne à `max` caractères (UTF-8 sûr), avec « … » si coupée.
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

/// Première ligne non vide d'un texte, tronquée — pour une trace mémoire compacte.
fn first_line(s: &str) -> String {
    let line = s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    clip(line, 200)
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
         via les outils (un par agent) : {agents}. Pour une demande simple, réponds directement \
         ou délègue à UN agent. Pour un objectif complexe nécessitant plusieurs étapes \
         coordonnées, utilise l'outil `orchestrate` (il planifie, fait valider le plan par \
         l'utilisateur, exécute en parallèle et corrige jusqu'à l'objectif). Synthétise les \
         retours, pose des questions si besoin. Sois concis.",
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
    // Compétences (fiches) : divulgation progressive. On ne met que nom + description dans le
    // prompt ; l'agent charge les instructions détaillées à la demande via `Load_Skill`. Cela
    // évite de payer le corps de chaque fiche à chaque appel.
    let assigned: Vec<&MarkdownSkill> = skills
        .iter()
        .filter_map(|sk| ctx.md_skills.iter().find(|m| m.id == *sk || m.name == *sk))
        .collect();
    if !assigned.is_empty() {
        s.push_str(
            "\n\n## Compétences\nTu disposes des compétences ci-dessous. Appelle `Load_Skill` \
             avec l'`id` indiqué pour obtenir la procédure détaillée quand tu en as besoin.",
        );
        for m in assigned {
            s.push_str(&format!("\n- **{}** (id `{}`)", m.name, m.id));
            if !m.description.is_empty() {
                s.push_str(&format!(" — {}", m.description));
            }
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
    fn delegation_tool_name_is_api_valid_for_accented_agents() {
        let valid = |s: &str| !s.is_empty() && s.len() <= 128
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');

        // Le nom d'agent peut contenir accents/espaces ; le nom d'OUTIL doit rester ASCII-safe.
        let agent = RosterAgent {
            name: "Agent Modélisateur & Co".to_string(),
            role: String::new(),
            skills: Vec::new(),
            documentalist: false,
        };
        let tool = delegation_tool(&agent);
        assert!(valid(&tool.name), "nom d'outil invalide : {}", tool.name);
        // La description conserve le vrai nom (lisibilité).
        assert!(tool.description.contains("Agent Modélisateur & Co"));

        // Le slug retrouve bien l'agent (dispatch).
        assert_eq!(tool_slug(&agent.name), tool.name);
        assert!(valid(&tool_slug("")) && valid(&tool_slug("é")));
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

    #[test]
    fn load_skill_exposed_only_when_a_fiche_is_assigned() {
        // Espace sans fiche → pas de Load_Skill.
        let space = space_with_agents(&["A"]);
        let ctx = AgentContext::from_space(&space);
        let agents = roster(&space);
        let none: Vec<_> = agent_tools(&agents[0], &ctx).into_iter().map(|t| t.name).collect();
        assert!(!none.iter().any(|n| n == markdown_skill::LOAD_SKILL));

        // Espace avec une fiche assignée → Load_Skill exposé.
        let dir = std::env::temp_dir().join(format!("orch-rt-fiche-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".orchestra")).unwrap();
        markdown_skill::create(&dir, "Quiz", "fait un quiz").unwrap();
        let mut space2 = space_with_agents(&["A"]);
        space2.root = dir.clone();
        space2.config.skills = vec!["Quiz".to_string()];
        let ctx2 = AgentContext::from_space(&space2);
        let agents2 = roster(&space2);
        let with: Vec<_> = agent_tools(&agents2[0], &ctx2).into_iter().map(|t| t.name).collect();
        assert!(with.iter().any(|n| n == markdown_skill::LOAD_SKILL));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Espace à racine temporaire (pour que la mémoire ne pollue pas le dépôt en test).
    fn space_with_temp_root(agents: &[&str], tag: &str) -> (ContextSpace, PathBuf) {
        let dir = std::env::temp_dir().join(format!("orch-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".orchestra")).unwrap();
        let mut space = space_with_agents(agents);
        space.root = dir.clone();
        (space, dir)
    }

    #[tokio::test]
    async fn orchestrate_offline_runs_plan_after_approval() {
        let (space, dir) = space_with_temp_root(&["A", "B"], "orch-ok");
        let mut handle = orchestrate_inner(&space, "objectif", None);
        handle.approve.send(true).unwrap(); // approbation (bufferisée jusqu'à la réception)

        let (mut plan_ready, mut task_done, mut coord_done) = (false, 0, false);
        while let Some(ev) = handle.events.recv().await {
            match ev {
                AgentEvent::PlanReady { tasks } => {
                    assert_eq!(tasks.len(), 2); // pipeline linéaire A → B
                    assert_eq!(tasks[1].depends_on, vec!["t1"]);
                    plan_ready = true;
                }
                AgentEvent::TaskDone { .. } => task_done += 1,
                AgentEvent::Done { agent } if agent == COORDINATOR => {
                    coord_done = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(plan_ready && coord_done);
        assert_eq!(task_done, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_plan_parallel_waves_respect_dependencies() {
        // Diamant : t1 → {t2, t3} (indépendantes, même vague) → t4.
        let (space, dir) = space_with_temp_root(&["A"], "exec-diamond");
        let ctx = AgentContext::from_space(&space);
        let roster = roster(&space);
        let plan = Plan::new(vec![
            orchestration::Task::new("t1", "A", "o", vec![]),
            orchestration::Task::new("t2", "A", "o", vec!["t1".into()]),
            orchestration::Task::new("t3", "A", "o", vec!["t1".into()]),
            orchestration::Task::new("t4", "A", "o", vec!["t2".into(), "t3".into()]),
        ]);

        let (tx, mut rx) = mpsc::unbounded_channel();
        let transcript = run_waves(None, &ctx, &roster, &plan, &tx).await; // hors-ligne
        drop(tx);
        assert_eq!(transcript.len(), 4);

        let mut done = Vec::new();
        while let Some(ev) = rx.recv().await {
            if let AgentEvent::TaskDone { id } = ev {
                done.push(id);
            }
        }
        assert_eq!(done.len(), 4);
        assert_eq!(done.first().unwrap(), "t1", "la racine s'exécute en premier");
        assert_eq!(done.last().unwrap(), "t4", "la jointure s'exécute en dernier");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn orchestrate_cancel_runs_no_task() {
        let (space, dir) = space_with_temp_root(&["A", "B"], "orch-cancel");
        let mut handle = orchestrate_inner(&space, "objectif", None);
        handle.approve.send(false).unwrap(); // refus

        let mut task_started = 0;
        while let Some(ev) = handle.events.recv().await {
            match ev {
                AgentEvent::TaskStarted { .. } => task_started += 1,
                AgentEvent::Done { agent } if agent == COORDINATOR => break,
                _ => {}
            }
        }
        assert_eq!(task_started, 0, "aucune tâche ne doit s'exécuter si le plan est refusé");
        let _ = std::fs::remove_dir_all(&dir);
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
                _ => {}
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
        let ChatHandle { user, mut events, .. } = start_conversation_inner(&space, None);

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
                _ => {}
            }
        }
        assert_eq!((started, done), (1, 1));
        assert!(saw_doc, "l'Agent Documentaliste doit être lancé");
    }
}
