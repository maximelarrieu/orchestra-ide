//! Interface graphique de bureau (Dioxus) — **première tranche verticale**.
//!
//! Démontre le port GUI sans réécrire le moteur : l'UI **consomme directement
//! `orchestra-core`** (aucune frontière IPC, aucun Node). Elle charge un Espace, liste ses
//! agents, lance l'orchestration et **streame les `AgentEvent`** dans la fenêtre (radar +
//! panneau Plan + bouton d'approbation) — le même contrat que consomme déjà le TUI.
//!
//! Découpage : [`state`] (état + pont vers le cœur), [`components`] (rendu), [`styles`] (CSS).
//!
//! ⚠️ Premier jet : à compiler sur une machine avec webview (WebView2 sur Windows). On
//! itérera sur d'éventuels ajustements d'API Dioxus au premier build.

mod components;
mod state;
mod styles;

use dioxus::prelude::*;
use orchestra_core::model::ContextSpace;
use std::path::PathBuf;
use tokio::sync::mpsc::UnboundedSender;

use state::{drive_orchestration, PlanRow};

/// Espace ouvert au démarrage (un sélecteur de dossier viendra ensuite).
const DEFAULT_SPACE: &str = "examples/recherche-immo-aix";

fn main() {
    dioxus::launch(app);
}

fn app() -> Element {
    // Chargement de l'Espace une seule fois (accès disque synchrone côté cœur — hors rendu).
    let space = use_signal(|| ContextSpace::load(&PathBuf::from(DEFAULT_SPACE)).ok());
    let log = use_signal(Vec::<String>::new);
    let plan = use_signal(Vec::<PlanRow>::new);
    let mut pending = use_signal(|| false);
    let approve_tx = use_signal(|| None::<UnboundedSender<bool>>);

    // Lance l'orchestration (le pont vers le cœur vit dans `state`).
    let launch = move |_| {
        if let Some(sp) = space() {
            drive_orchestration(sp, log, plan, pending, approve_tx);
        }
    };
    // Approuve le plan proposé → l'orchestration s'exécute.
    let approve = move |_| {
        if let Some(tx) = approve_tx() {
            let _ = tx.send(true);
        }
        pending.set(false);
    };

    rsx! {
        style { {styles::CSS} }
        div { class: "app",
            h1 { "🎻 Orchestra IDE" }
            match space() {
                Some(sp) => rsx! {
                    {components::header(
                        &sp.config.project_name,
                        &sp.config.agents.iter().map(|a| a.name.clone()).collect::<Vec<_>>().join(", "),
                    )}
                    div { class: "actions",
                        button { onclick: launch, "▶ Lancer l'orchestre" }
                        if pending() {
                            button { class: "go", onclick: approve, "✓ Exécuter le plan" }
                        }
                    }
                    if !plan().is_empty() {
                        {components::plan_panel(&plan())}
                    }
                    {components::radar(&log())}
                },
                None => rsx! {
                    p { class: "error", "Impossible de charger l'espace « {DEFAULT_SPACE} »." }
                },
            }
        }
    }
}
