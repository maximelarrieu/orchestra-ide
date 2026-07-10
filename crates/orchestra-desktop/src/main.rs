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
use orchestra_core::session::Sessions;
use std::collections::HashMap;
use tokio::sync::mpsc::UnboundedSender;

use state::{ChatMsg, DesktopSession, PlanRow, View};

fn main() {
    dioxus::launch(app);
}

fn app() -> Element {
    // Sessions (onglets) : une par Espace ouvert, chacune avec son propre contexte et son
    // historique. On démarre **sans** session : l'utilisateur ouvre ou crée un espace via la
    // barre d'espaces (les récents s'y retrouvent aussi).
    let sessions = use_signal(Sessions::<DesktopSession>::new);
    // Espaces connus (récents).
    let known = use_signal(state::known_spaces);
    // Vue centrale : Assistant / Documents / Modifications.
    let view = use_signal(|| View::Chat);

    // Projections « à plat » de la session ACTIVE : les composants d'affichage lisent ces
    // signaux (inchangés). Ils sont rafraîchis par l'effet ci-dessous à chaque mutation du store
    // (événement d'agent, changement d'onglet). Les sessions en arrière-plan accumulent leur
    // état dans le store sans perturber l'affichage.
    let mut space = use_signal(|| None::<ContextSpace>);
    let mut messages = use_signal(Vec::<ChatMsg>::new);
    let mut thinking = use_signal(|| false);
    let mut plan = use_signal(Vec::<PlanRow>::new);
    let mut pending = use_signal(|| false);
    let mut approve_tx = use_signal(|| None::<UnboundedSender<bool>>);
    let mut user_tx = use_signal(|| None::<UnboundedSender<String>>);
    let mut agents_status = use_signal(HashMap::<String, state::AgStatus>::new);
    let mut changes = use_signal(Vec::<state::FileChange>::new);
    let draft = use_signal(String::new);
    let doc_content = use_signal(String::new);

    // Projection : synchronise les signaux plats avec la session active. Les signaux sont `Copy`,
    // donc les capturer ici (par copie) n'empêche pas de les repasser aux composants plus bas.
    use_effect(move || {
        let s = sessions.read();
        match s.active() {
            Some(a) => {
                space.set(Some(a.space.clone()));
                messages.set(a.messages.clone());
                thinking.set(a.thinking);
                plan.set(a.plan.clone());
                pending.set(a.pending);
                approve_tx.set(a.approve_tx.clone());
                user_tx.set(a.user_tx.clone());
                agents_status.set(a.status.clone());
                changes.set(a.changes.clone());
            }
            None => space.set(None),
        }
    });

    // Auto-démarrage : la session active ouvre sa conversation dès qu'on est sur l'Assistant.
    use_effect(move || {
        let (idx, need) = {
            let s = sessions.read();
            (s.active_index(), s.active().map(|a| !a.started).unwrap_or(false))
        };
        if need && view() == View::Chat {
            state::start_session_chat(sessions, idx);
        }
    });

    // « Nouvelle conversation » : relance la session active (contexte remis à zéro).
    let start_chat = move |_| {
        let idx = sessions.read().active_index();
        state::start_session_chat(sessions, idx);
    };

    rsx! {
        style { {styles::CSS} }
        div { class: "app",
            h1 { "🎻 Orchestra IDE" }

            // Sélecteur d'espaces : ouvre chaque espace dans un onglet.
            components::SpaceBar { sessions, known }

            // Barre d'onglets (sessions) — n'apparaît qu'à partir de deux sessions.
            { components::tabs_bar(sessions) }

            {components::nav(view)}

            match view() {
                View::Chat => rsx! {
                    div { class: "chatwrap",
                        div { class: "actions",
                            button { onclick: start_chat, "↻ Nouvelle conversation" }
                            for a in orchestra_core::runtime::quick_actions() {
                                { components::action_button(a, user_tx, draft) }
                            }
                        }
                        p { class: "hint",
                            "Décris ton besoin dans la zone de saisie, ou utilise une action ci-dessus — tu peux aussi simplement discuter avec le coordinateur."
                        }
                        components::SquadPanel { status: agents_status }
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
