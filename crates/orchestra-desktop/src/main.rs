//! Interface graphique de bureau (Dioxus) — port GUI d'Orchestra IDE.
//!
//! L'UI **consomme directement `orchestra-core`** (aucune frontière IPC, aucun Node) : elle
//! charge/édite un Espace, lance l'orchestration et **streame les `AgentEvent`** — le même
//! contrat que le TUI. Parité avec le TUI sur les 5 piliers dev : Fichiers, Git, Docker,
//! Plan/Tâches, Agents (+ Docs, Terminal, Mémoire, Contexte).
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
use orchestra_core::docker::DockerStatus;
use orchestra_core::git::GitStatus;
use orchestra_core::model::ContextSpace;
use orchestra_core::session::Sessions;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

use state::{ChatMsg, DesktopSession, GitDiffView, PlanRow};

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
    // Depuis quand / quel agent réfléchit (indicateur « réfléchit… Ns », en-tête + bas du chat).
    let mut busy_since = use_signal(|| None::<Instant>);
    let mut busy_agent = use_signal(|| None::<String>);
    // Chrono d'affichage : incrémenté chaque seconde pour forcer le recalcul du texte « …Ns »
    // même sans nouvel `AgentEvent` (un appel LLM local peut rester silencieux plusieurs
    // dizaines de secondes). Tourne pour la durée de vie de l'app, coût négligeable.
    let mut tick = use_signal(|| 0u64);
    use_future(move || async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            tick.set(tick() + 1);
        }
    });
    let mut plan = use_signal(Vec::<PlanRow>::new);
    let mut pending = use_signal(|| false);
    let mut approve_tx = use_signal(|| None::<UnboundedSender<bool>>);
    let mut user_tx = use_signal(|| None::<UnboundedSender<String>>);
    let mut agents_status = use_signal(HashMap::<String, state::AgStatus>::new);
    let draft = use_signal(String::new);

    // État des panneaux Git / Docker / Docs (pilier « constat du projet », indépendant du fil de
    // conversation) : rafraîchis à chaque changement de session active, ou à la demande.
    let git_status = use_signal(GitStatus::default);
    let docker_status = use_signal(DockerStatus::default);
    let mut doc_selected = use_signal(|| None::<PathBuf>);
    let mut git_diff = use_signal(|| None::<GitDiffView>);

    // Projection : synchronise les signaux plats avec la session active. Les signaux sont `Copy`,
    // donc les capturer ici (par copie) n'empêche pas de les repasser aux composants plus bas.
    use_effect(move || {
        let s = sessions.read();
        match s.active() {
            Some(a) => {
                space.set(Some(a.space.clone()));
                messages.set(a.messages.clone());
                thinking.set(a.thinking);
                busy_since.set(a.busy_since);
                busy_agent.set(a.busy_agent.clone());
                plan.set(a.plan.clone());
                pending.set(a.pending);
                approve_tx.set(a.approve_tx.clone());
                user_tx.set(a.user_tx.clone());
                agents_status.set(a.status.clone());
            }
            None => {
                space.set(None);
                busy_since.set(None);
                busy_agent.set(None);
            }
        }
    });

    // Racine de travail de la session active : dépendance **minimale** pour Git/Docker (un
    // `PathBuf`, comparé par égalité) — contrairement à la projection ci-dessus, on ne veut PAS
    // relancer `git status`/`docker compose ps` à chaque événement d'agent, seulement quand
    // l'onglet actif change réellement.
    let active_root = use_memo(move || sessions.read().active().map(|a| a.workspace_root().to_path_buf()));
    use_effect(move || {
        if let Some(root) = active_root() {
            doc_selected.set(None);
            git_diff.set(None);
            state::refresh_dev_status(root, git_status, docker_status);
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

    // État d'affichage : panneaux repliables + thème.
    let mut show_explorer = use_signal(|| true);
    let mut show_viewer = use_signal(|| true);
    let mut dark = use_signal(|| true); // thème sombre par défaut (comme le template)
    let mut show_spacebar = use_signal(|| false);

    let has_session = space().is_some();
    // Lire `tick()` abonne ce calcul au chrono d'affichage : le texte se recalcule chaque
    // seconde tant qu'un appel LLM est en cours, sans dépendre d'un nouvel `AgentEvent`.
    let _ = tick();
    let busy_elapsed = busy_since().map(|t| t.elapsed().as_secs());
    let status_pill = match (busy_elapsed, has_session) {
        (Some(secs), _) => format!("réfléchit… {secs}s"),
        (None, true) => "prêt".to_string(),
        (None, false) => "—".to_string(),
    };
    let project = space().map(|s| s.config.project_name).unwrap_or_else(|| "Orchestra".into());
    let app_cls = if dark() { "app dark" } else { "app light" };
    // La barre d'espaces s'affiche à la demande (+) ou tant qu'aucune session n'est ouverte.
    let spacebar_open = show_spacebar() || !has_session;

    rsx! {
        style { {styles::CSS} }
        div { class: "{app_cls}",
            // --- Barre supérieure : onglets de session + « + » + réglages d'affichage ---
            div { class: "topbar",
                span { class: "logo" }
                span { class: "brand", "Orchestra" }
                { components::tabs_bar(sessions) }
                button { class: "tabadd", title: "Ouvrir / créer un espace",
                    onclick: move |_| show_spacebar.set(!show_spacebar()), "+" }
                div { class: "topspacer" }
                button { class: "icontoggle",
                    title: "Thème clair / sombre",
                    onclick: move |_| dark.set(!dark()),
                    if dark() { "☀" } else { "☾" } }
            }

            // Sélecteur d'espaces (ouvrir / créer / récents) — à la demande.
            if spacebar_open {
                components::SpaceBar { sessions, known }
            }

            // --- Shell : checkpoints · explorateur · conversation · visualiseur · tâches ---
            div { class: "ide",
                { components::checkpoint_rail(messages) }

                if show_explorer() {
                    div { class: "pane explorerpane",
                        div { class: "panebar",
                            span { class: "panetitle", "{project}" }
                            button { class: "panebtn", title: "Masquer l'explorateur",
                                onclick: move |_| show_explorer.set(false), "‹" }
                        }
                        components::FileExplorer { sessions, selected, doc_selected, git_diff }
                    }
                } else {
                    div { class: "stub",
                        button { class: "stubbtn", title: "Afficher l'explorateur",
                            onclick: move |_| show_explorer.set(true), "›" }
                    }
                }

                div { class: "pane conversation",
                    div { class: "convhead",
                        span { class: "agentname", "Agent — {project}" }
                        span { class: "statuspill", "{status_pill}" }
                        div { class: "convspacer" }
                        button { class: "ghost", onclick: start_chat, "↻ Nouvelle conversation" }
                    }
                    components::SquadPanel { status: agents_status }
                    if has_session {
                        div { class: "chatcol",
                            if user_tx().is_some() {
                                {components::chat_view(messages, thinking, busy_agent(), busy_elapsed, draft, user_tx)}
                            }
                        }
                    } else {
                        p { class: "hint", "Ouvre ou crée un espace (+) pour discuter avec l'Orchestrateur." }
                    }
                }

                if show_viewer() {
                    div { class: "pane viewer",
                        div { class: "panebar",
                            span { class: "panetitle", "FICHIER" }
                            div { class: "topspacer" }
                            button { class: "panebtn", title: "Masquer le visualiseur",
                                onclick: move |_| show_viewer.set(false), "›" }
                        }
                        components::CenterPane { sessions, selected, doc_selected, git_diff }
                        { components::terminal_panel(sessions) }
                    }
                } else {
                    div { class: "stub",
                        button { class: "stubbtn", title: "Afficher le visualiseur",
                            onclick: move |_| show_viewer.set(true), "‹" }
                    }
                }

                components::TaskRail {
                    plan, pending, approve_tx, sessions,
                    selected, doc_selected, git_diff, git_status, docker_status,
                }
            }

            // --- Barre de statut ---
            div { class: "statusbar",
                span { class: "sb-item",
                    if has_session { "{project}" } else { "Aucun espace" }
                }
                span { class: "sb-item",
                    if let Some(p) = state::llm_status() {
                        span { class: "dot ok" }
                        "{p}"
                    } else {
                        span { class: "dot warn" }
                        "mode simulé — clé API absente"
                    }
                }
                span { class: "sb-item", "Orchestra IDE" }
            }
        }
    }
}
