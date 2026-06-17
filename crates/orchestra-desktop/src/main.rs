//! Interface graphique de bureau (Dioxus) — **première tranche verticale**.
//!
//! Démontre le port GUI sans réécrire le moteur : l'UI **consomme directement
//! `orchestra-core`** (aucune frontière IPC, aucun Node). Elle charge un Espace, liste ses
//! agents, lance l'orchestration et **streame les [`AgentEvent`]** dans la fenêtre (radar +
//! panneau Plan + bouton d'approbation) — exactement le contrat que consomme déjà le TUI.
//!
//! ⚠️ Premier jet : à compiler sur une machine avec webview (WebView2 sur Windows). On
//! itérera sur d'éventuels ajustements d'API Dioxus au premier build.

use dioxus::prelude::*;
use orchestra_core::events::AgentEvent;
use orchestra_core::model::ContextSpace;
use orchestra_core::runtime;
use std::path::PathBuf;
use tokio::sync::mpsc::UnboundedSender;

/// Espace ouvert au démarrage (l'exemple fourni ; un sélecteur de dossier viendra ensuite).
const DEFAULT_SPACE: &str = "examples/recherche-immo-aix";

/// Objectif par défaut envoyé à l'orchestre (saisie libre à venir).
const DEFAULT_GOAL: &str = "Avance concrètement sur l'objectif de cet espace.";

fn main() {
    dioxus::launch(app);
}

fn app() -> Element {
    // Chargement de l'Espace une seule fois (accès disque synchrone côté cœur — hors rendu).
    let space = use_signal(|| ContextSpace::load(&PathBuf::from(DEFAULT_SPACE)).ok());
    let mut log = use_signal(Vec::<String>::new); // lignes du radar
    let mut plan = use_signal(Vec::<PlanRow>::new); // (id, agent, statut)
    let mut pending = use_signal(|| false); // un plan attend l'approbation
    let mut approve_tx = use_signal(|| None::<UnboundedSender<bool>>);

    // Lance l'orchestration et streame les événements du cœur dans l'UI.
    let launch = move |_| {
        let Some(sp) = space() else { return };
        log.set(Vec::new());
        plan.set(Vec::new());
        pending.set(false);
        spawn(async move {
            let handle = runtime::orchestrate(&sp, DEFAULT_GOAL);
            approve_tx.set(Some(handle.approve));
            let mut events = handle.events;
            while let Some(ev) = events.recv().await {
                match ev {
                    AgentEvent::PlanReady { tasks } => {
                        plan.set(
                            tasks
                                .into_iter()
                                .map(|t| PlanRow { id: t.id, agent: t.agent, status: "en attente".into() })
                                .collect(),
                        );
                        pending.set(true);
                    }
                    AgentEvent::TaskStarted { id, .. } => set_status(&mut plan, &id, "en cours"),
                    AgentEvent::TaskDone { id } => set_status(&mut plan, &id, "fait ✓"),
                    AgentEvent::TaskFailed { id, .. } => set_status(&mut plan, &id, "échec ✗"),
                    AgentEvent::Started { agent } => log.write().push(format!("▶ {agent}")),
                    AgentEvent::Done { agent } => log.write().push(format!("✔ {agent}")),
                    AgentEvent::Log { agent, msg } => log.write().push(format!("{agent} : {msg}")),
                    AgentEvent::Thinking { .. } => {}
                }
            }
        });
    };

    // Approuve le plan proposé → l'orchestration s'exécute.
    let approve = move |_| {
        if let Some(tx) = approve_tx() {
            let _ = tx.send(true);
        }
        pending.set(false);
    };

    rsx! {
        style { {CSS} }
        div { class: "app",
            h1 { "🎻 Orchestra IDE" }
            match space() {
                Some(sp) => rsx! {
                    h2 { "{sp.config.project_name}" }
                    p { class: "agents",
                        "Agents : "
                        {sp.config.agents.iter().map(|a| a.name.clone()).collect::<Vec<_>>().join(", ")}
                    }
                    div { class: "actions",
                        button { onclick: launch, "▶ Lancer l'orchestre" }
                        if pending() {
                            button { class: "go", onclick: approve, "✓ Exécuter le plan" }
                        }
                    }
                    if !plan().is_empty() {
                        h3 { "Plan" }
                        ul { class: "plan",
                            for row in plan() {
                                li { key: "{row.id}", "{row.id} · {row.agent} — {row.status}" }
                            }
                        }
                    }
                    h3 { "Radar" }
                    pre { class: "radar", {log().join("\n")} }
                },
                None => rsx! {
                    p { class: "error", "Impossible de charger l'espace « {DEFAULT_SPACE} »." }
                },
            }
        }
    }
}

/// Une ligne du panneau Plan.
#[derive(Clone, PartialEq)]
struct PlanRow {
    id: String,
    agent: String,
    status: String,
}

/// Met à jour le statut d'une tâche du plan (par id).
fn set_status(plan: &mut Signal<Vec<PlanRow>>, id: &str, status: &str) {
    let mut rows = plan.write();
    if let Some(row) = rows.iter_mut().find(|r| r.id == id) {
        row.status = status.to_string();
    }
}

const CSS: &str = r#"
    body { margin: 0; background: #0b0e14; color: #e6e6e6; }
    .app { font-family: ui-sans-serif, system-ui, sans-serif; padding: 1rem 1.25rem; }
    h1 { font-size: 1.3rem; } h2 { color: #8ab4ff; margin: .2rem 0; }
    .agents { color: #9aa; }
    .actions { margin: .6rem 0; }
    button { background: #1d2535; color: #e6e6e6; border: 1px solid #2c3a55;
             border-radius: 6px; padding: .45rem .8rem; cursor: pointer; font-size: .95rem; }
    button:hover { background: #263150; }
    button.go { background: #1f5132; border-color: #2e7d4a; margin-left: .5rem; }
    .plan { list-style: none; padding-left: 0; }
    .plan li { padding: .2rem .4rem; border-left: 3px solid #2c3a55; margin: .2rem 0; }
    .radar { background: #05070c; color: #cdd6e0; padding: .6rem; border-radius: 6px;
             max-height: 320px; overflow: auto; white-space: pre-wrap; font-size: .85rem; }
    .error { color: #ff8a8a; }
"#;
