//! Composants de présentation — fonctions renvoyant un `Element`. Pas de logique métier :
//! elles lisent des signaux et déclenchent les ponts de [`crate::state`].

use std::path::PathBuf;

use dioxus::prelude::*;
use orchestra_core::model::ContextSpace;

use tokio::sync::mpsc::UnboundedSender;

use crate::state::{self, ChatMsg, MsgKind, PlanRow, SkillEntry, SkillKind, View};

/// Barre de navigation entre les vues.
pub fn nav(mut view: Signal<View>) -> Element {
    let cur = view();
    rsx! {
        div { class: "nav",
            button { class: "{tab(cur, View::Orchestrate)}", onclick: move |_| view.set(View::Orchestrate), "Orchestrer" }
            button { class: "{tab(cur, View::Chat)}", onclick: move |_| view.set(View::Chat), "Chat" }
            button { class: "{tab(cur, View::Documents)}", onclick: move |_| view.set(View::Documents), "Documents" }
            button { class: "{tab(cur, View::Agents)}", onclick: move |_| view.set(View::Agents), "Agents & skills" }
        }
    }
}

fn tab(cur: View, this: View) -> &'static str {
    if cur == this {
        "tab on"
    } else {
        "tab"
    }
}

/// Panneau Plan : une ligne par tâche (id · agent — statut).
pub fn plan_panel(rows: &[PlanRow]) -> Element {
    rsx! {
        h3 { "Plan" }
        ul { class: "plan",
            for row in rows {
                li { key: "{row.id}", "{row.id} · {row.agent} — {row.status}" }
            }
        }
    }
}

/// Radar : le flux d'activité (lignes de log).
pub fn radar(lines: &[String]) -> Element {
    rsx! {
        h3 { "Radar" }
        pre { class: "radar", {lines.join("\n")} }
    }
}

/// Vue Documents : liste des documents de l'espace + visualiseur (lecture seule).
pub fn documents_view(space: Signal<Option<ContextSpace>>, content: Signal<String>) -> Element {
    let docs = space().map(|s| s.documents()).unwrap_or_default();
    rsx! {
        div { class: "cols",
            ul { class: "list",
                for d in docs {
                    { doc_item(d.label.clone(), d.path.clone(), content) }
                }
            }
            pre { class: "viewer", {content()} }
        }
    }
}

fn doc_item(label: String, path: PathBuf, mut content: Signal<String>) -> Element {
    rsx! {
        li {
            button { class: "row", onclick: move |_| {
                    if let Ok(text) = orchestra_core::model::load_document(&path) {
                        content.set(text);
                    }
                },
                "{label}"
            }
        }
    }
}

/// Vue Agents & skills : liste des agents + sélecteur de skills à cocher pour l'agent choisi.
pub fn agents_view(space: Signal<Option<ContextSpace>>, selected: Signal<usize>) -> Element {
    let Some(sp) = space() else {
        return rsx! { p { class: "error", "Aucun espace chargé." } };
    };
    let agents = sp.config.agents.clone();
    if agents.is_empty() {
        return rsx! { p { "Aucun agent dans cet espace." } };
    }
    let sel = selected().min(agents.len() - 1);
    let current_name = agents[sel].name.clone();
    let entries = state::skill_entries(&sp, sel);

    rsx! {
        div { class: "cols",
            ul { class: "list",
                for (i, a) in agents.iter().enumerate() {
                    { agent_item(i, a.name.clone(), i == sel, selected) }
                }
            }
            div { class: "detail",
                h3 { "Skills de « {current_name} »" }
                ul { class: "plan",
                    for e in entries {
                        { skill_row(e, space, sel) }
                    }
                }
            }
        }
    }
}

/// Vue Chat : conversation avec le coordinateur (bulles + saisie + approbation de plan inline).
#[allow(clippy::too_many_arguments)]
pub fn chat_view(
    messages: Signal<Vec<ChatMsg>>,
    thinking: Signal<bool>,
    mut draft: Signal<String>,
    user_tx: Signal<Option<UnboundedSender<String>>>,
    plan: Signal<Vec<PlanRow>>,
    mut pending: Signal<bool>,
    approve_tx: Signal<Option<UnboundedSender<bool>>>,
) -> Element {
    // Envoi via le bouton…
    let send_click = move |_| {
        let text = draft();
        if text.trim().is_empty() {
            return;
        }
        if let Some(tx) = user_tx() {
            let _ = tx.send(text);
        }
        draft.set(String::new());
    };
    // …et via la touche Entrée (logique dupliquée : un même closure ne peut être déplacé 2×).
    let send_key = move |e: KeyboardEvent| {
        if e.key() != Key::Enter {
            return;
        }
        let text = draft();
        if text.trim().is_empty() {
            return;
        }
        if let Some(tx) = user_tx() {
            let _ = tx.send(text);
        }
        draft.set(String::new());
    };
    let approve = move |_| {
        if let Some(tx) = approve_tx() {
            let _ = tx.send(true);
        }
        pending.set(false);
    };

    rsx! {
        div { class: "chat",
            div { class: "messages",
                for (i, m) in messages().into_iter().enumerate() {
                    ChatBubble { key: "{i}", msg: m }
                }
                if thinking() {
                    div { class: "bubble coord", "…" }
                }
            }
            if pending() {
                div { class: "planbox",
                    {plan_panel(&plan())}
                    button { class: "go", onclick: approve, "✓ Approuver et exécuter le plan" }
                }
            }
            div { class: "composer",
                input {
                    class: "chatinput",
                    value: "{draft}",
                    placeholder: "Écris au chef d'orchestre…  (Entrée pour envoyer)",
                    oninput: move |e| draft.set(e.value()),
                    onkeydown: send_key,
                }
                button { onclick: send_click, "Envoyer" }
            }
        }
    }
}

/// Une bulle de chat. Les messages d'**agent** sont **repliés par défaut** (le coordinateur
/// les résume) : une flèche ▶/▼ déroule/cache leur texte. Chaque bulle a son propre état
/// d'ouverture, d'où un vrai composant.
#[component]
fn ChatBubble(msg: ChatMsg) -> Element {
    let mut expanded = use_signal(|| false);
    match msg.kind {
        MsgKind::User => rsx! {
            div { class: "bubble user", div { class: "text", "{msg.text}" } }
        },
        MsgKind::Coordinator => rsx! {
            div { class: "bubble coord",
                span { class: "who", "{msg.who}" }
                div { class: "text", "{msg.text}" }
            }
        },
        MsgKind::System => rsx! {
            div { class: "bubble system", "{msg.who} {msg.text}" }
        },
        MsgKind::Agent => rsx! {
            div { class: "bubble agent",
                button { class: "disclosure", onclick: move |_| expanded.set(!expanded()),
                    if expanded() { "▼ {msg.who}" } else { "▶ {msg.who}" }
                }
                if expanded() {
                    div { class: "text", "{msg.text}" }
                }
            }
        },
    }
}

fn agent_item(index: usize, name: String, active: bool, mut selected: Signal<usize>) -> Element {
    let cls = if active { "row on" } else { "row" };
    rsx! {
        li {
            button { class: "{cls}", onclick: move |_| selected.set(index), "{name}" }
        }
    }
}

fn skill_row(e: SkillEntry, space: Signal<Option<ContextSpace>>, agent_idx: usize) -> Element {
    let id = e.id.clone();
    let mark = if e.selected { "[x]" } else { "[ ]" };
    let badge = match e.kind {
        SkillKind::Primitive => "prim.",
        SkillKind::Fiche => "fiche",
        SkillKind::Label => "inact",
    };
    rsx! {
        li {
            button { class: "row", onclick: move |_| state::toggle_skill(space, agent_idx, &id),
                "{mark} {e.id} · {badge} — {e.description}"
            }
        }
    }
}
