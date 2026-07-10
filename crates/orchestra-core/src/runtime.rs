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

use serde_json::{json, Value};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::events::AgentEvent;
use crate::integrations::{self, IntegrationConn};
use crate::llm::{Block, LlmClient, Msg, ToolResult, ToolSpec};
use crate::markdown_skill::{self, MarkdownSkill};
use crate::memory;
use crate::model::space::ContextSpace;
use crate::skills;

/// Nom affiché de l'**Orchestrateur** (agent principal) dans le flux de conversation. Public pour
/// que les UI distinguent ses messages de ceux des sous-agents qu'il déploie.
pub const COORDINATOR: &str = "Orchestrateur";

/// Nombre maximal de tours LLM ↔ outils par agent (garde-fou anti-boucle).
const MAX_TURNS: usize = 6;

/// Contexte de session transmis à l'Orchestrateur et à ses sous-agents (clonable, `Send`).
#[derive(Clone)]
struct AgentContext {
    project_name: String,
    persona: Option<String>,
    /// Racine de la session (`.orchestra/` y vit, dont la mémoire partagée).
    root: PathBuf,
    workspace: PathBuf,
    /// Fiches Markdown de la session (instructions), chargeables via `Load_Skill`.
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
            persona: space.persona.clone(),
            root: space.root.clone(),
            workspace,
            md_skills: Arc::new(crate::markdown_skill::load_all(&space.root)),
            integ: IntegrationConn::from_space(space),
        }
    }
}

/// Outils d'un **sous-agent** (déployé par l'Orchestrateur) : tous les skills exécutables +
/// intégrations Git/GitHub configurées + mémoire + `Load_Skill`. Volontairement sans `spawn_agent`
/// (seul l'Orchestrateur compose l'équipe → pas de récursion).
fn agent_tools(ctx: &AgentContext) -> Vec<ToolSpec> {
    let mut t = skills::all_tool_specs();
    t.extend(integrations::tool_definitions(&ctx.integ));
    t.extend(memory::tool_definitions());
    t.push(markdown_skill::tool_definition()); // Load_Skill (fiches de l'espace, si présentes)
    t
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
            } else if name == skills::WRITE_FILE {
                // Écriture de fichier : on capture le contenu avant/après pour émettre le diff.
                let rel = input.get("path").and_then(Value::as_str).unwrap_or("").to_string();
                let abs = ctx.workspace.join(&rel);
                let before = std::fs::read_to_string(&abs).unwrap_or_default();
                let outcome = skills::execute_skill(&name, &input, &ctx.workspace).await;
                if !outcome.is_error {
                    let after = std::fs::read_to_string(&abs).unwrap_or_default();
                    let (added, removed, diff) = crate::diff::summarize(&before, &after);
                    let _ = tx.send(AgentEvent::FileChanged {
                        agent: label.to_string(),
                        path: rel,
                        added,
                        removed,
                        diff,
                    });
                }
                outcome
            } else if name == skills::READ_FILE {
                // Lecture : on signale quel agent explore quel fichier (« orchestre en verre »).
                if let Some(rel) = input.get("path").and_then(Value::as_str) {
                    let _ = tx.send(AgentEvent::FileRead {
                        agent: label.to_string(),
                        path: rel.to_string(),
                    });
                }
                skills::execute_skill(&name, &input, &ctx.workspace).await
            } else if name == skills::EXEC_COMMAND {
                // Commande shell : on relaie la sortie vers le panneau Terminal de l'UI.
                let cmd = input.get("command").and_then(Value::as_str).unwrap_or("").to_string();
                let outcome = skills::execute_skill(&name, &input, &ctx.workspace).await;
                let _ = tx.send(AgentEvent::Terminal {
                    agent: label.to_string(),
                    command: cmd,
                    output: outcome.text.clone(),
                    ok: !outcome.is_error,
                });
                outcome
            } else if name == SET_PLAN {
                // Publie le plan dans le rail Tâches de l'UI.
                let steps = input.get("steps").and_then(Value::as_array).cloned().unwrap_or_default();
                let tasks: Vec<crate::events::PlannedTask> = steps
                    .iter()
                    .enumerate()
                    .map(|(i, s)| crate::events::PlannedTask {
                        id: (i + 1).to_string(),
                        agent: String::new(),
                        // Tolère une chaîne simple ou un objet {title}.
                        objective: s
                            .as_str()
                            .map(str::to_string)
                            .or_else(|| s.get("title").and_then(Value::as_str).map(str::to_string))
                            .unwrap_or_default(),
                        depends_on: Vec::new(),
                    })
                    .collect();
                let n = tasks.len();
                let _ = tx.send(AgentEvent::PlanReady { tasks });
                skills::SkillOutcome::ok(format!("Plan publié ({n} étape(s)) dans le rail Tâches."))
            } else if name == UPDATE_STEP {
                // Met à jour l'état d'une étape du plan → événement Task* consommé par l'UI.
                let id = input.get("step").and_then(Value::as_i64).unwrap_or(0).to_string();
                let status = input.get("status").and_then(Value::as_str).unwrap_or("");
                let ev = match status {
                    "running" => Some(AgentEvent::TaskStarted { id: id.clone(), agent: label.to_string() }),
                    "done" => Some(AgentEvent::TaskDone { id: id.clone() }),
                    "failed" => Some(AgentEvent::TaskFailed { id: id.clone(), error: String::new() }),
                    _ => None,
                };
                match ev {
                    Some(ev) => {
                        let _ = tx.send(ev);
                        skills::SkillOutcome::ok(format!("Étape {id} → {status}."))
                    }
                    None => skills::SkillOutcome::err("`status` doit valoir running, done ou failed."),
                }
            } else if name == SPAWN_AGENT {
                // L'Orchestrateur déploie un sous-agent ad hoc (seul lui a cet outil → pas de récursion).
                let role = input.get("role").and_then(Value::as_str).unwrap_or("Agent").trim().to_string();
                let role = if role.is_empty() { "Agent".to_string() } else { role };
                let instruction = input.get("instruction").and_then(Value::as_str).unwrap_or("").to_string();
                // `Box::pin` : la récursion async (run_agent_turn → sous-agent → run_agent_turn)
                // doit passer par un pointeur.
                match Box::pin(run_spawned_agent(client, &role, &instruction, ctx, tx)).await {
                    Ok(text) => skills::SkillOutcome::ok(text),
                    Err(e) => skills::SkillOutcome::err(format!("échec du sous-agent « {role} » : {e}")),
                }
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
    // Canal d'approbation conservé pour compat du contrat ; l'Orchestrateur pilote la boucle
    // conversationnellement (pas de plan formel à approuver ici).
    let (approve_tx, _approve_rx) = mpsc::unbounded_channel::<bool>();
    let ctx = AgentContext::from_space(space);

    tokio::spawn(conversation_task(ctx, client, user_rx, event_tx));
    ChatHandle { user: user_tx, events: event_rx, approve: approve_tx }
}

/// Outil `spawn_agent` : l'Orchestrateur crée un sous-agent spécialisé à la volée.
const SPAWN_AGENT: &str = "spawn_agent";

fn spawn_agent_tool() -> ToolSpec {
    ToolSpec {
        name: SPAWN_AGENT.to_string(),
        description:
            "Crée un SOUS-AGENT spécialisé pour accomplir une tâche précise, et renvoie son \
             résultat. Fournis un `role` (ex. « Architecte », « Codeur backend », « Testeur », \
             « Rédacteur doc ») et une `instruction` autonome et détaillée. Sert à déléguer, \
             paralléliser ou mobiliser une expertise — tu composes ainsi ton équipe."
                .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "role": { "type": "string", "description": "Rôle/spécialité du sous-agent" },
                "instruction": { "type": "string", "description": "Tâche autonome et détaillée à réaliser" }
            },
            "required": ["role", "instruction"]
        }),
    }
}

/// Outils de **plan** : l'Orchestrateur publie son plan (et met à jour l'avancement) dans le rail
/// « Tâches » de l'UI — au lieu de ne l'écrire qu'en prose dans le chat.
const SET_PLAN: &str = "Set_Plan";
const UPDATE_STEP: &str = "Update_Step";

fn plan_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: SET_PLAN.to_string(),
            description:
                "Publie (ou remplace) TON PLAN dans le panneau Tâches de l'interface. Appelle-le \
                 dès que tu as un plan, AVANT d'agir. Fournis `steps` : une liste ordonnée \
                 d'intitulés d'étapes courts (chaînes de texte). Tiens ensuite le plan à jour \
                 avec `Update_Step`."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "steps": {
                        "type": "array",
                        "description": "Étapes ordonnées du plan (intitulés courts)",
                        "items": { "type": "string" }
                    }
                },
                "required": ["steps"]
            }),
        },
        ToolSpec {
            name: UPDATE_STEP.to_string(),
            description:
                "Met à jour l'état d'une étape du plan (rail Tâches). `step` = numéro de l'étape \
                 (1 = première). `status` ∈ { \"running\", \"done\", \"failed\" }. Marque une étape \
                 `running` quand tu la commences, puis `done` (ou `failed`) quand elle est finie."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "step": { "type": "integer", "description": "Numéro de l'étape (à partir de 1)" },
                    "status": { "type": "string", "enum": ["running", "done", "failed"] }
                },
                "required": ["step", "status"]
            }),
        },
    ]
}

/// Jeu d'outils **complet** de l'Orchestrateur : agir directement (tous les skills exécutables),
/// intégrations Git/GitHub configurées, mémoire, chargement de fiches, plan, et déploiement d'agents.
fn orchestrator_tools(ctx: &AgentContext) -> Vec<ToolSpec> {
    let mut t = skills::all_tool_specs();
    t.extend(integrations::tool_definitions(&ctx.integ));
    t.extend(memory::tool_definitions());
    t.push(markdown_skill::tool_definition()); // Load_Skill toujours disponible
    t.extend(plan_tools());
    t.push(spawn_agent_tool());
    t
}

/// Prompt système de l'Orchestrateur : agent principal, très outillé, piloté par la boucle
/// **Perceive → Think → Act → Check**, capable de déployer une équipe (`spawn_agent`).
fn orchestrator_prompt(ctx: &AgentContext) -> String {
    let persona = ctx
        .persona
        .as_deref()
        .filter(|p| !p.trim().is_empty())
        .unwrap_or("(contexte non renseigné — demande-le à l'utilisateur si nécessaire)");
    format!(
        "Tu es l'**Orchestrateur** d'Orchestra IDE, l'agent principal de la session « {name} ».\n\
         Dossier de travail : {ws}\n\n\
         # Rôle\n\
         Agent autonome, polyvalent et robuste. Tu AGIS directement (lire/écrire des fichiers, \
         exécuter des commandes shell, chercher sur le web, tenir une mémoire) ET tu DÉPLOIES UNE \
         ÉQUIPE : crée des sous-agents spécialisés à la volée via `spawn_agent` (tu choisis leur \
         rôle et leur donnes une instruction claire), puis tu coordonnes leurs résultats. Tu réponds \
         en français, de façon concise, et tu n'inventes jamais de résultat d'outil.\n\n\
         # Boucle de travail : Perceive → Think → Act → Check (itère)\n\
         1. **Perceive** — rassemble le contexte : liste et lis les fichiers utiles, `Recall` la \
            mémoire, inspecte le workspace. Vérifie plutôt que supposer.\n\
         2. **Think** — établis un plan court et explicite (étapes, découpage, quels sous-agents). \
            **Publie-le systématiquement via `Set_Plan`** (une étape = un titre court) : il s'affiche \
            dans le panneau « Tâches » de l'interface. C'est ta feuille de route visible.\n\
         3. **Act** — exécute : toi-même pour le simple, `spawn_agent` pour le spécialisé ou le \
            parallélisable. Fais des changements petits et sûrs. **Tiens le plan à jour** : marque \
            l'étape en cours avec `Update_Step(step, \"running\")`, puis `\"done\"` (ou `\"failed\"`) \
            une fois terminée. L'utilisateur suit ainsi l'avancement en direct.\n\
         4. **Check** — vérifie (relis les fichiers modifiés, lance les tests/commandes, confronte \
            aux critères). Si c'est incomplet ou faux, **repars en Perceive** et itère. Consigne les \
            décisions et l'avancement en mémoire (`Remember`).\n\n\
         # Principes\n\
         - Transparent : explique brièvement chaque action et chaque délégation.\n\
         - Demande à l'utilisateur pour toute décision qui lui revient ou toute info manquante.\n\
         - Documente au fil de l'eau (`docs/…`) et garde une trace des changements.\n\
         - Prudence sur les actions destructrices ; privilégie des étapes réversibles.\n\n\
         # Contexte de la session\n{persona}",
        name = ctx.project_name,
        ws = ctx.workspace.display(),
    )
}

/// Boucle de conversation avec l'**Orchestrateur** : à chaque message utilisateur, l'agent
/// principal mène sa boucle PTAC (agir directement + `spawn_agent`). Se termine quand le `Sender`
/// utilisateur est fermé. L'historique `conv` est conservé entre les messages.
async fn conversation_task(
    ctx: AgentContext,
    client: Option<Arc<LlmClient>>,
    mut user_rx: UnboundedReceiver<String>,
    tx: UnboundedSender<AgentEvent>,
) {
    let _ = tx.send(AgentEvent::Started { agent: COORDINATOR.to_string() });
    let _ = tx.send(AgentEvent::Log {
        agent: COORDINATOR.to_string(),
        msg: "Prêt. Décris ton objectif ou pose ta question.".to_string(),
    });

    let system = orchestrator_prompt(&ctx);
    let tools = orchestrator_tools(&ctx);
    let mut conv: Vec<Msg> = Vec::new();

    while let Some(user_msg) = user_rx.recv().await {
        // Écho du message utilisateur pour une lecture « chat » du flux.
        let _ = tx.send(AgentEvent::Log { agent: "Vous".to_string(), msg: user_msg.clone() });

        match &client {
            Some(c) => {
                conv.push(Msg::User(user_msg));
                if let Err(e) =
                    run_agent_turn(c, &system, &tools, &mut conv, COORDINATOR, &ctx, &tx).await
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

/// Déploie un **sous-agent** ad hoc (créé par l'Orchestrateur via `spawn_agent`) : rôle → prompt,
/// jeu d'outils complet (sans `spawn_agent`, donc pas de récursion), exécute l'instruction et
/// renvoie son résultat. Émet Started/Done pour que l'UI voie l'équipe travailler.
async fn run_spawned_agent(
    client: &LlmClient,
    role: &str,
    instruction: &str,
    ctx: &AgentContext,
    tx: &UnboundedSender<AgentEvent>,
) -> Result<String, crate::llm::LlmError> {
    let _ = tx.send(AgentEvent::Started { agent: role.to_string() });
    let system = build_system_prompt(role, ctx);
    let tools = agent_tools(ctx); // pas de spawn_agent → pas de récursion infinie
    let mut conv: Vec<Msg> = vec![Msg::User(instruction.to_string())];
    let text = run_agent_turn(client, &system, &tools, &mut conv, role, ctx, tx).await;
    let _ = tx.send(AgentEvent::Done { agent: role.to_string() });
    text
}

// --- Orchestration one-shot (bouton « Objectif rapide ») ---------------------------------

/// Poignée d'une orchestration : l'UI reçoit les événements sur `events`.
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

/// Cœur testable : client LLM injecté (les tests passent `None` → mode simulé).
fn orchestrate_inner(
    space: &ContextSpace,
    objective: &str,
    client: Option<Arc<LlmClient>>,
) -> OrchestrationHandle {
    // Le canal d'approbation est conservé pour compat du contrat (l'Orchestrateur pilote
    // conversationnellement, sans plan formel à approuver).
    let (approve_tx, _approve_rx) = mpsc::unbounded_channel::<bool>();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let ctx = AgentContext::from_space(space);
    tokio::spawn(orchestration_task(ctx, client, objective.to_string(), event_tx));
    OrchestrationHandle { approve: approve_tx, events: event_rx }
}

/// Exécution **one-shot** de l'Orchestrateur PTAC sur un objectif (bouton « Objectif rapide »).
async fn orchestration_task(
    ctx: AgentContext,
    client: Option<Arc<LlmClient>>,
    objective: String,
    tx: UnboundedSender<AgentEvent>,
) {
    let _ = tx.send(AgentEvent::Started { agent: COORDINATOR.to_string() });
    match &client {
        Some(c) => {
            let system = orchestrator_prompt(&ctx);
            let tools = orchestrator_tools(&ctx);
            let mut conv = vec![Msg::User(objective)];
            if let Err(e) = run_agent_turn(c, &system, &tools, &mut conv, COORDINATOR, &ctx, &tx).await {
                emit_log(&tx, COORDINATOR, &format!("⚠ LLM injoignable ({e})"));
            }
        }
        None => emit_log(
            &tx,
            COORDINATOR,
            "(mode simulé) Définis ANTHROPIC_API_KEY ou GEMINI_API_KEY pour une exécution réelle.",
        ),
    }
    let _ = tx.send(AgentEvent::Done { agent: COORDINATOR.to_string() });
}

/// Construit le prompt système d'un sous-agent à partir de son nom et de son rôle (les fiches
/// Markdown de l'espace sont annoncées sous « ## Compétences », chargeables via `Load_Skill`).
fn build_system_prompt(role: &str, ctx: &AgentContext) -> String {
    let mut s = format!(
        "Tu es un **sous-agent** « {role} » déployé par l'Orchestrateur pour la session « {} ». \
         Dossier de travail : {}. Réponds en français, de façon concise. Mène ta tâche à son terme \
         en utilisant tes outils quand c'est pertinent ; n'invente jamais de résultat d'outil.",
        ctx.project_name,
        ctx.workspace.display(),
    );
    // Fiches de l'espace (divulgation progressive) : on n'annonce que nom + description ; le corps
    // se charge à la demande via `Load_Skill`.
    if !ctx.md_skills.is_empty() {
        s.push_str(
            "\n\n## Compétences (fiches)\nAppelle `Load_Skill` avec l'`id` indiqué pour la procédure détaillée :",
        );
        for m in ctx.md_skills.iter() {
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


#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::config::ProjectConfig;

    fn tmp_space(tag: &str) -> ContextSpace {
        let root = std::env::temp_dir().join(format!("orch-rt-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".orchestra")).unwrap();
        ContextSpace {
            root,
            config: ProjectConfig {
                project_name: "Test".into(),
                workspace_path: None,
                integrations: Default::default(),
            },
            persona: None,
            adrs: vec![],
        }
    }

    #[test]
    fn orchestrator_is_fully_equipped_with_spawn() {
        let ctx = AgentContext::from_space(&tmp_space("tools"));
        let names: Vec<_> = orchestrator_tools(&ctx).into_iter().map(|t| t.name).collect();
        assert!(names.iter().any(|n| n == SPAWN_AGENT), "spawn_agent exposé");
        assert!(names.iter().any(|n| n == SET_PLAN), "Set_Plan exposé");
        assert!(names.iter().any(|n| n == UPDATE_STEP), "Update_Step exposé");
        assert!(names.iter().any(|n| n == skills::READ_FILE));
        assert!(names.iter().any(|n| n == skills::EXEC_COMMAND));
        assert!(names.iter().any(|n| n == memory::REMEMBER));
        // Un sous-agent est outillé mais NE peut PAS spawner ni piloter le plan (pas de récursion).
        let sub: Vec<_> = agent_tools(&ctx).into_iter().map(|t| t.name).collect();
        assert!(sub.iter().any(|n| n == skills::WRITE_FILE));
        assert!(!sub.iter().any(|n| n == SPAWN_AGENT));
        assert!(!sub.iter().any(|n| n == SET_PLAN));
    }

    #[tokio::test]
    async fn quick_orchestrate_offline_starts_and_finishes() {
        let space = tmp_space("orch");
        let mut handle = orchestrate_inner(&space, "objectif", None);
        let (mut started, mut done) = (false, false);
        while let Some(ev) = handle.events.recv().await {
            match ev {
                AgentEvent::Started { agent } if agent == COORDINATOR => started = true,
                AgentEvent::Done { agent } if agent == COORDINATOR => { done = true; break; }
                _ => {}
            }
        }
        assert!(started && done);
        let _ = std::fs::remove_dir_all(&space.root);
    }
}
