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

use dioxus::desktop::tao::dpi::LogicalSize;
use dioxus::desktop::{Config, WindowBuilder};
use dioxus::prelude::*;
use orchestra_core::model::ContextSpace;
use orchestra_core::session::Sessions;
use std::collections::HashMap;
use tokio::sync::mpsc::UnboundedSender;

use state::{ChatMsg, DesktopSession, PlanRow};

fn main() {
    // Taille de fenêtre par défaut raisonnable (tient dans un 1920×1080 sans réajuster) ;
    // redimensionnable, avec un minimum utilisable.
    let window = WindowBuilder::new()
        .with_title("Orchestra IDE")
        .with_inner_size(LogicalSize::new(1280.0, 820.0))
        .with_min_inner_size(LogicalSize::new(900.0, 600.0));
    dioxus::LaunchBuilder::new()
        .with_cfg(Config::new().with_window(window))
        .launch(app);
}

fn app() -> Element {
    // Sessions (onglets) : une par Espace ouvert, chacune avec son propre contexte et son
    // historique. On démarre **sans** session : l'utilisateur ouvre ou crée un espace via la
    // barre d'espaces (les récents s'y retrouvent aussi).
    let sessions = use_signal(Sessions::<DesktopSession>::new);
    // Espaces connus (récents).
    let known = use_signal(state::known_spaces);
    // Fichier ouvert au centre (chemin relatif au workspace de la session active).
    let selected = use_signal(|| None::<String>);

    // Projections « à plat » de la session ACTIVE pour le panneau de conversation. Rafraîchies à
    // chaque mutation du store (événement d'agent, changement d'onglet). Les sessions en
    // arrière-plan accumulent leur état dans le store sans perturber l'affichage courant.
    let mut space = use_signal(|| None::<ContextSpace>);
    let mut messages = use_signal(Vec::<ChatMsg>::new);
    let mut thinking = use_signal(|| false);
    let mut plan = use_signal(Vec::<PlanRow>::new);
    let mut pending = use_signal(|| false);
    let mut approve_tx = use_signal(|| None::<UnboundedSender<bool>>);
    let mut user_tx = use_signal(|| None::<UnboundedSender<String>>);
    let mut agents_status = use_signal(HashMap::<String, state::AgStatus>::new);
    let draft = use_signal(String::new);

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
            }
            None => space.set(None),
        }
    });

    // Auto-démarrage : la session active ouvre sa conversation dès qu'elle existe.
    use_effect(move || {
        let (idx, need) = {
            let s = sessions.read();
            (s.active_index(), s.active().map(|a| !a.started).unwrap_or(false))
        };
        if need {
            state::start_session_chat(sessions, idx);
        }
    });

    // « Nouvelle conversation » : relance la session active (contexte remis à zéro).
    let start_chat = move |_| {
        let idx = sessions.read().active_index();
        state::start_session_chat(sessions, idx);
    };

    let has_session = space().is_some();

    rsx! {
        style { {styles::CSS} }
        div { class: "app",
            // En-tête compact : titre + barre d'espaces (ouvrir/créer) + onglets.
            div { class: "topbar",
                span { class: "brand", "🎻 Orchestra IDE" }
                components::SpaceBar { sessions, known }
            }
            { components::tabs_bar(sessions) }

            // Shell 3 panneaux (façon Cursor) : explorateur · centre · conversation.
            div { class: "ide",
                div { class: "pane left",
                    components::FileExplorer { sessions, selected }
                }
                div { class: "pane center",
                    components::CenterPane { sessions, selected }
                }
                div { class: "pane right",
                    div { class: "righthead",
                        span { "Orchestrateur" }
                        button { class: "ghost", onclick: start_chat, "↻ Nouvelle conversation" }
                    }
                    components::SquadPanel { status: agents_status }
                    if has_session {
                        div { class: "actions",
                            for a in orchestra_core::runtime::quick_actions() {
                                { components::action_button(a, user_tx, draft) }
                            }
                        }
                        div { class: "chatcol",
                            if user_tx().is_some() {
                                {components::chat_view(messages, thinking, draft, user_tx, plan, pending, approve_tx)}
                            }
                        }
                    } else {
                        p { class: "hint", "Ouvre ou crée un espace pour discuter avec l'Orchestrateur." }
                    }
                }
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
