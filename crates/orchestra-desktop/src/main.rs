//! Interface graphique de bureau (Dioxus) — port GUI d'Orchestra IDE.
//!
//! L'UI **consomme directement `orchestra-core`** (aucune frontière IPC, aucun Node) : elle
//! charge/édite un Espace, liste agents et skills, lance l'orchestration et **streame les
//! `AgentEvent`** — le même contrat que le TUI. Objectif : retrouver dans la fenêtre les
//! fonctions du terminal (orchestrer, documents, agents & skills ; chat et édition persona à venir).
//!
//! Découpage : [`state`] (état + ponts vers le cœur), [`components`] (rendu), [`styles`] (CSS).
//!
//! ⚠️ À compiler sur une machine avec webview (WebView2 sur Windows). On itère sur d'éventuels
//! ajustements d'API Dioxus au premier build.

mod components;
mod state;
mod styles;

use dioxus::prelude::*;
use orchestra_core::model::ContextSpace;
use std::path::PathBuf;
use tokio::sync::mpsc::UnboundedSender;

use state::{drive_orchestration, ChatMsg, PlanRow, View};

/// Espace ouvert au démarrage.
const DEFAULT_SPACE: &str = "examples/recherche-immo-aix";

fn main() {
    dioxus::launch(app);
}

fn app() -> Element {
    let space = use_signal(|| ContextSpace::load(&PathBuf::from(DEFAULT_SPACE)).ok());
    let space_path = use_signal(|| DEFAULT_SPACE.to_string());
    // Espaces connus (récents) : mémorise l'espace d'ouverture puis liste le registre.
    let known = use_signal(|| {
        let _ = orchestra_core::registry::remember_space(&PathBuf::from(DEFAULT_SPACE));
        state::known_spaces()
    });
    let mut objective = use_signal(|| state::DEFAULT_GOAL.to_string());
    let view = use_signal(|| View::Orchestrate);

    // État d'orchestration / plan (partagé avec le chat pour l'approbation inline).
    let log = use_signal(Vec::<String>::new);
    let plan = use_signal(Vec::<PlanRow>::new);
    let mut pending = use_signal(|| false);
    let approve_tx = use_signal(|| None::<UnboundedSender<bool>>);

    // État des vues Documents / Agents.
    let selected_agent = use_signal(|| 0usize);
    let doc_content = use_signal(String::new);

    // État du chat.
    let messages = use_signal(Vec::<ChatMsg>::new);
    let thinking = use_signal(|| false);
    let draft = use_signal(String::new);
    let user_tx = use_signal(|| None::<UnboundedSender<String>>);

    let launch = move |_| {
        if let Some(sp) = space() {
            drive_orchestration(sp, objective(), log, plan, pending, approve_tx);
        }
    };
    let approve = move |_| {
        if let Some(tx) = approve_tx() {
            let _ = tx.send(true);
        }
        pending.set(false);
    };
    let start_chat = move |_| {
        if let Some(sp) = space() {
            state::start_chat(sp, user_tx, messages, thinking, plan, pending, approve_tx);
        }
    };

    // Auto-démarrage : à l'entrée de l'onglet Chat, la conversation s'ouvre directement (pas de
    // bouton intermédiaire). Une conversation déjà en cours est conservée.
    use_effect(move || {
        if view() == View::Chat && user_tx().is_none() {
            if let Some(sp) = space() {
                state::start_chat(sp, user_tx, messages, thinking, plan, pending, approve_tx);
            }
        }
    });

    rsx! {
        style { {styles::CSS} }
        div { class: "app",
            h1 { "🎻 Orchestra IDE" }

            // Sélecteur d'espaces : chemin + espaces connus (récents) — équivalent [3] du TUI.
            components::SpaceBar { space, space_path, known }

            {components::nav(view)}

            match view() {
                View::Orchestrate => rsx! {
                    div { class: "orchestrate",
                        textarea {
                            class: "goal",
                            value: "{objective}",
                            oninput: move |e| objective.set(e.value()),
                        }
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
                    }
                },
                View::Chat => rsx! {
                    div { class: "chatwrap",
                        div { class: "actions",
                            button { onclick: start_chat, "↻ Nouvelle conversation" }
                        }
                        if user_tx().is_some() {
                            {components::chat_view(messages, thinking, draft, user_tx, plan, pending, approve_tx)}
                        }
                    }
                },
                View::Documents => rsx! { components::DocumentsView { space, content: doc_content } },
                View::Agents => rsx! { components::AgentsView { space, selected: selected_agent } },
            }
        }
    }
}
