//! Composants de présentation — fonctions renvoyant un `Element`. Pas de logique métier :
//! elles lisent des signaux et déclenchent les ponts de [`crate::state`].

use std::path::PathBuf;

use dioxus::prelude::*;
use orchestra_core::model::ContextSpace;

use crate::state::{self, PlanRow, SkillEntry, SkillKind, View};

/// Barre de navigation entre les vues.
pub fn nav(mut view: Signal<View>) -> Element {
    let cur = view();
    rsx! {
        div { class: "nav",
            button { class: "{tab(cur, View::Orchestrate)}", onclick: move |_| view.set(View::Orchestrate), "Orchestrer" }
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
pub fn agents_view(space: Signal<Option<ContextSpace>>, mut selected: Signal<usize>) -> Element {
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
