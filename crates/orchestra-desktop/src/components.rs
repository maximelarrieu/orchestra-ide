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

/// Vue Agents & skills : **menu de gestion complet** — activer le Documentaliste, choisir /
/// ajouter / éditer / supprimer des agents, et brancher / éditer leurs skills. Toutes les
/// actions passent par `orchestra-core` (mêmes opérations que le TUI).
#[component]
pub fn AgentsView(space: Signal<Option<ContextSpace>>, selected: Signal<usize>) -> Element {
    // États d'édition locaux à la vue.
    let mut rename = use_signal(|| None::<String>); // Some(buffer) = renommage en cours
    let mut role = use_signal(|| None::<String>); // Some(buffer) = édition du rôle
    let mut new_agent = use_signal(String::new); // saisie d'un agent personnalisé
    let mut new_skill = use_signal(String::new); // saisie d'une nouvelle fiche
    let mut fiche = use_signal(|| None::<(PathBuf, String)>); // éditeur de fiche ouvert
    let mut refresh = use_signal(|| 0u32); // bump après écriture disque (fiches) → recalcul
    let _ = refresh(); // abonnement : un bump force le recalcul du catalogue

    let Some(sp) = space() else {
        return rsx! { p { class: "error", "Aucun espace chargé." } };
    };
    let agents = sp.config.agents.clone();
    let doc_on = sp.config.documentalist_enabled;
    let doc_cls = if doc_on { "tab on" } else { "tab" };

    rsx! {
        div { class: "agentsmenu",
            // Bandeau : Documentaliste + ajout d'agents.
            div { class: "toolbar",
                button {
                    class: "{doc_cls}",
                    onclick: move |_| state::toggle_documentalist(space),
                    if doc_on { "📝 Documentaliste : activé" } else { "📝 Documentaliste : désactivé" }
                }
                button { onclick: move |_| state::add_suggested_agent(space), "+ Agent suggéré" }
            }
            p { class: "hint",
                "Le Documentaliste prend des notes (cours, exercices, corrections, fiches de révision) quand il est activé."
            }

            div { class: "cols",
                // Colonne gauche : liste des agents + ajout personnalisé.
                div { class: "list",
                    ul { class: "list",
                        for (i, a) in agents.iter().enumerate() {
                            { agent_item(i, a.name.clone(), i == selected().min(agents.len().saturating_sub(1)), selected) }
                        }
                    }
                    div { class: "composer",
                        input {
                            class: "chatinput",
                            value: "{new_agent}",
                            placeholder: "Nouvel agent…",
                            oninput: move |e| new_agent.set(e.value()),
                        }
                        button {
                            onclick: move |_| {
                                state::add_agent(space, &new_agent());
                                new_agent.set(String::new());
                            },
                            "Ajouter"
                        }
                    }
                }

                // Colonne droite : détail de l'agent sélectionné.
                if agents.is_empty() {
                    div { class: "detail", p { "Aucun agent. Ajoute-en un (suggéré ou personnalisé)." } }
                } else {
                    {
                        let sel = selected().min(agents.len() - 1);
                        let agent = agents[sel].clone();
                        let entries = state::skill_entries(&sp, sel);
                        // Copies possédées : on n'emprunte jamais `agent` à travers une closure.
                        let name = agent.name.clone();
                        let name_for_btn = agent.name.clone();
                        let role_val = agent.role.clone();
                        let role_for_btn = agent.role.clone();
                        let role_empty = agent.role.is_empty();
                        rsx! {
                            div { class: "detail",
                                // Nom (affichage ou renommage).
                                if let Some(buf) = rename() {
                                    div { class: "composer",
                                        input { class: "chatinput", value: "{buf}",
                                            oninput: move |e| rename.set(Some(e.value())) }
                                        button { onclick: move |_| {
                                                if let Some(v) = rename() { state::set_agent_name(space, sel, &v); }
                                                rename.set(None);
                                            }, "OK" }
                                        button { onclick: move |_| rename.set(None), "Annuler" }
                                    }
                                } else {
                                    h3 {
                                        "{name}  "
                                        button { class: "linklike",
                                            onclick: move |_| rename.set(Some(name_for_btn.clone())), "renommer" }
                                    }
                                }

                                // Rôle (affichage ou édition).
                                if let Some(buf) = role() {
                                    div { class: "composer",
                                        input { class: "chatinput", value: "{buf}", placeholder: "Rôle de l'agent…",
                                            oninput: move |e| role.set(Some(e.value())) }
                                        button { onclick: move |_| {
                                                if let Some(v) = role() { state::set_agent_role(space, sel, &v); }
                                                role.set(None);
                                            }, "OK" }
                                        button { onclick: move |_| role.set(None), "Annuler" }
                                    }
                                } else {
                                    p { class: "rolerow",
                                        span { class: "muted",
                                            if role_empty { "(rôle non défini)" } else { "{role_val}" }
                                        }
                                        "  "
                                        button { class: "linklike",
                                            onclick: move |_| role.set(Some(role_for_btn.clone())), "éditer le rôle" }
                                    }
                                }

                                button { class: "danger", onclick: move |_| state::delete_agent(space, sel),
                                    "🗑 Supprimer cet agent" }

                                h4 { "Skills" }
                                ul { class: "plan",
                                    for e in entries {
                                        { skill_row(e, space, sel, fiche, refresh) }
                                    }
                                }

                                // Créer une nouvelle fiche de skill.
                                div { class: "composer",
                                    input { class: "chatinput", value: "{new_skill}",
                                        placeholder: "Nouvelle fiche de skill…",
                                        oninput: move |e| new_skill.set(e.value()) }
                                    button { onclick: move |_| {
                                            if let Some(path) = state::create_fiche(space, &new_skill()) {
                                                let text = orchestra_core::model::load_document(&path).unwrap_or_default();
                                                fiche.set(Some((path, text)));
                                                new_skill.set(String::new());
                                                refresh.set(refresh() + 1);
                                            }
                                        }, "Créer une fiche" }
                                }

                                // Éditeur de fiche (ouvert à la demande).
                                if let Some((path, content)) = fiche() {
                                    div { class: "ficheeditor",
                                        h4 { "Édition de la fiche" }
                                        p { class: "hint", "{path:?}" }
                                        textarea { class: "fichearea", value: "{content}",
                                            oninput: move |e| {
                                                if let Some((p, _)) = fiche() { fiche.set(Some((p, e.value()))); }
                                            } }
                                        div { class: "actions",
                                            button { class: "go",
                                                onclick: move |_| {
                                                    if let Some((p, c)) = fiche() {
                                                        state::save_fiche(&p, &c);
                                                    }
                                                    fiche.set(None);
                                                    refresh.set(refresh() + 1);
                                                }, "💾 Enregistrer" }
                                            button { onclick: move |_| fiche.set(None), "Fermer" }
                                        }
                                    }
                                }
                            }
                        }
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
    // …et via la touche Entrée (Maj+Entrée = saut de ligne). Logique dupliquée : un même
    // closure ne peut être déplacé 2×.
    let send_key = move |e: KeyboardEvent| {
        // Maj+Entrée (ou autre touche) → comportement par défaut du textarea (saut de ligne).
        if e.key() != Key::Enter || e.modifiers().contains(Modifiers::SHIFT) {
            return;
        }
        e.prevent_default(); // Entrée seule = envoi : pas d'insertion de saut de ligne
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
                textarea {
                    class: "chatinput",
                    rows: "2",
                    value: "{draft}",
                    placeholder: "Écris au chef d'orchestre…  (Entrée pour envoyer · Maj+Entrée pour un saut de ligne)",
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

fn skill_row(
    e: SkillEntry,
    space: Signal<Option<ContextSpace>>,
    agent_idx: usize,
    mut fiche: Signal<Option<(PathBuf, String)>>,
    mut refresh: Signal<u32>,
) -> Element {
    let toggle_id = e.id.clone();
    let mark = if e.selected { "[x]" } else { "[ ]" };
    let badge = match e.kind {
        SkillKind::Primitive => "prim.",
        SkillKind::Fiche => "fiche",
        SkillKind::Unwired => "inact",
    };
    rsx! {
        li { class: "skillrow",
            button { class: "row", onclick: move |_| state::toggle_skill(space, agent_idx, &toggle_id),
                "{mark} {e.id} · {badge} — {e.description}"
            }
            // Brancher un skill non branché (crée sa fiche, ouvre l'éditeur).
            if e.kind == SkillKind::Unwired {
                {
                    let id = e.id.clone();
                    rsx! {
                        button { class: "linklike", onclick: move |_| {
                                if let Some(path) = state::wire_skill(space, &id) {
                                    let text = orchestra_core::model::load_document(&path).unwrap_or_default();
                                    fiche.set(Some((path, text)));
                                    refresh.set(refresh() + 1);
                                }
                            }, "brancher" }
                    }
                }
            }
            // Éditer une fiche existante.
            if e.kind == SkillKind::Fiche {
                {
                    let id = e.id.clone();
                    rsx! {
                        button { class: "linklike", onclick: move |_| {
                                if let Some(loaded) = state::load_fiche(space, &id) { fiche.set(Some(loaded)); }
                            }, "éditer" }
                    }
                }
            }
        }
    }
}
