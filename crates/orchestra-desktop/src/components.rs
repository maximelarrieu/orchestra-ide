//! Composants de présentation (sans logique métier) — de simples fonctions renvoyant un
//! `Element`. Elles pourront devenir des `#[component]` avec props quand elles grossiront.

use dioxus::prelude::*;

use crate::state::PlanRow;

/// En-tête : nom du projet + liste des agents.
pub fn header(project_name: &str, agents: &str) -> Element {
    rsx! {
        h2 { "{project_name}" }
        p { class: "agents", "Agents : {agents}" }
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
