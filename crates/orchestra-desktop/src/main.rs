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

use state::{ChatMsg, PlanRow, View};

/// Espace ouvert au démarrage.
const DEFAULT_SPACE: &str = "examples/apprentissage-espagnol";

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
    // Vue unique de travail : l'Assistant (conversation). Les autres vues sont Documents,
    // Agents & skills, Modifications.
    let view = use_signal(|| View::Chat);

    // État du plan (panneau + approbation inline dans la conversation).
    let plan = use_signal(Vec::<PlanRow>::new);
    let pending = use_signal(|| false);
    let approve_tx = use_signal(|| None::<UnboundedSender<bool>>);

    // État des vues Documents / Agents.
    let selected_agent = use_signal(|| 0usize);
    let doc_content = use_signal(String::new);

    // État de la conversation.
    let messages = use_signal(Vec::<ChatMsg>::new);
    let thinking = use_signal(|| false);
    let mut draft = use_signal(String::new);
    let user_tx = use_signal(|| None::<UnboundedSender<String>>);
    // Statut live des agents (encart « squad »).
    let agents_status = use_signal(std::collections::HashMap::<String, state::AgStatus>::new);
    // Fichiers modifiés par les agents (vue Modifications).
    let changes = use_signal(Vec::<state::FileChange>::new);
    // Racine de l'espace de la conversation en cours (pour la redémarrer au changement d'espace).
    let mut chat_root = use_signal(|| None::<PathBuf>);

    let start_chat = move |_| {
        if let Some(sp) = space() {
            let root = sp.root.clone();
            state::start_chat(sp, user_tx, messages, thinking, plan, pending, approve_tx, agents_status, changes);
            chat_root.set(Some(root));
        }
    };

    // Auto-démarrage : à l'entrée de l'onglet Chat, la conversation s'ouvre directement (pas de
    // bouton intermédiaire) ; elle **redémarre si l'espace actif a changé** (squad à jour).
    use_effect(move || {
        let current_root = space().map(|s| s.root.clone());
        if view() == View::Chat && (user_tx().is_none() || chat_root() != current_root) {
            if let Some(sp) = space() {
                let root = sp.root.clone();
                state::start_chat(sp, user_tx, messages, thinking, plan, pending, approve_tx, agents_status, changes);
                chat_root.set(Some(root));
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
                View::Chat => rsx! {
                    div { class: "chatwrap",
                        div { class: "actions",
                            button { onclick: start_chat, "↻ Nouvelle conversation" }
                            button { class: "go",
                                onclick: move |_| {
                                    if let Some(tx) = user_tx() {
                                        let _ = tx.send(orchestra_core::runtime::orchestrate_message(&draft()));
                                        draft.set(String::new());
                                    }
                                },
                                "▶ Objectif rapide" }
                            button {
                                onclick: move |_| {
                                    if let Some(tx) = user_tx() {
                                        let _ = tx.send(orchestra_core::runtime::cadrage_message(&draft()));
                                        draft.set(String::new());
                                    }
                                },
                                "🧭 Cadrer le projet" }
                            button {
                                onclick: move |_| {
                                    if let Some(tx) = user_tx() {
                                        let _ = tx.send(orchestra_core::runtime::comprehension_message());
                                    }
                                },
                                "🔎 Analyser le projet" }
                        }
                        p { class: "hint",
                            "Décris ton besoin dans la zone de saisie. « Objectif rapide » lance une orchestration ; « Cadrer » fait poser des questions puis rédige un brief ; « Analyser » comprend un projet existant."
                        }
                        components::SquadPanel { space, status: agents_status }
                        div { class: "worksplit",
                            div { class: "chatcol",
                                if user_tx().is_some() {
                                    {components::chat_view(messages, thinking, draft, user_tx, plan, pending, approve_tx)}
                                }
                            }
                            div { class: "sidecol",
                                components::LiveChanges { changes }
                            }
                        }
                    }
                },
                View::Documents => rsx! { components::DocumentsView { space, content: doc_content } },
                View::Agents => rsx! { components::AgentsView { space, selected: selected_agent } },
                View::Changes => rsx! { components::ChangesView { changes } },
            }

            // Barre de statut (façon VS Code).
            div { class: "statusbar",
                span { class: "sb-item",
                    if let Some(sp) = space() { "📁 {sp.config.project_name}" } else { "Aucun espace" }
                }
                span { class: "sb-item", "🎻 Orchestra IDE" }
            }
        }
    }
}
