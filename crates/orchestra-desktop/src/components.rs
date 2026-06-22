//! Composants de présentation — fonctions renvoyant un `Element`. Pas de logique métier :
//! elles lisent des signaux et déclenchent les ponts de [`crate::state`].

use std::collections::HashMap;
use std::path::PathBuf;

use dioxus::prelude::*;
use orchestra_core::model::{ContextSpace, ProjectType};

use tokio::sync::mpsc::UnboundedSender;

use crate::state::{
    self, AgStatus, ChatMsg, FileChange, KnownSpace, MsgKind, PlanRow, SkillEntry, SkillKind, View,
};

/// Barre de navigation entre les vues.
pub fn nav(mut view: Signal<View>) -> Element {
    let cur = view();
    rsx! {
        div { class: "nav",
            button { class: "{tab(cur, View::Orchestrate)}", onclick: move |_| view.set(View::Orchestrate), "Orchestrer" }
            button { class: "{tab(cur, View::Chat)}", onclick: move |_| view.set(View::Chat), "Chat" }
            button { class: "{tab(cur, View::Documents)}", onclick: move |_| view.set(View::Documents), "Documents" }
            button { class: "{tab(cur, View::Agents)}", onclick: move |_| view.set(View::Agents), "Agents & skills" }
            button { class: "{tab(cur, View::Changes)}", onclick: move |_| view.set(View::Changes), "Modifications" }
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

/// Encart « squad » : statut live de chaque agent de l'espace (coordinateur + agents +
/// documentaliste), pour **voir qui travaille** pendant l'orchestration / le chat.
#[component]
pub fn SquadPanel(space: Signal<Option<ContextSpace>>, status: Signal<HashMap<String, AgStatus>>) -> Element {
    let Some(sp) = space() else {
        return rsx! {};
    };
    let mut roster: Vec<String> = vec![orchestra_core::runtime::COORDINATOR.to_string()];
    roster.extend(sp.config.agents.iter().map(|a| a.name.clone()));
    if sp.config.documentalist_enabled {
        roster.push("Agent_Documentaliste".to_string());
    }
    let map = status();
    rsx! {
        div { class: "squad",
            span { class: "muted", "Squad :" }
            for name in roster {
                {
                    let st = map.get(&name).copied().unwrap_or(AgStatus::Idle);
                    let icon = st.icon();
                    let label = st.label();
                    let cls = match st {
                        AgStatus::Idle => "agentchip",
                        AgStatus::Thinking => "agentchip thinking",
                        AgStatus::Working => "agentchip working",
                        AgStatus::Done => "agentchip done",
                    };
                    rsx! { span { class: "{cls}", "{icon} {name} · {label}" } }
                }
            }
        }
    }
}

/// Panneau Plan : une ligne par tâche (id · agent · statut · objectif · dépendances).
pub fn plan_panel(rows: &[PlanRow]) -> Element {
    rsx! {
        h3 { "Plan" }
        ul { class: "plan",
            for row in rows {
                {
                    let deps = if row.deps.is_empty() { String::new() } else { format!("⟸ {}", row.deps.join(", ")) };
                    rsx! {
                        li { key: "{row.id}",
                            div { class: "planhead", "{row.id} · {row.agent} — {row.status}" }
                            div { class: "planobj", "{row.objective}" }
                            if !deps.is_empty() {
                                div { class: "plandeps", "{deps}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Vue Modifications : fichiers changés par les agents (liste) + diff coloré du fichier choisi.
#[component]
pub fn ChangesView(changes: Signal<Vec<FileChange>>) -> Element {
    let mut sel = use_signal(|| 0usize);
    let list = changes();
    if list.is_empty() {
        return rsx! {
            p { class: "muted", "Aucune modification — les écritures des agents apparaîtront ici." }
        };
    }
    let idx = sel().min(list.len() - 1);
    let diff = list[idx].diff.clone();
    rsx! {
        div { class: "cols",
            ul { class: "list",
                for (i, c) in list.iter().enumerate() {
                    { change_item(i, c.path.clone(), c.added, c.removed, i == idx, sel) }
                }
            }
            div { class: "viewer",
                div { class: "diff",
                    for line in diff.lines() {
                        {
                            let cls = if line.starts_with("+ ") {
                                "dl add"
                            } else if line.starts_with("- ") {
                                "dl del"
                            } else {
                                "dl ctx"
                            };
                            rsx! { div { class: "{cls}", "{line}" } }
                        }
                    }
                }
            }
        }
    }
}

fn change_item(index: usize, path: String, added: usize, removed: usize, active: bool, mut sel: Signal<usize>) -> Element {
    let cls = if active { "row on" } else { "row" };
    rsx! {
        li {
            button { class: "{cls}", onclick: move |_| sel.set(index),
                "{path}  +{added} -{removed}" }
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

/// Barre d'espaces : saisie d'un chemin + **liste des espaces connus** (récents) à rouvrir d'un
/// clic, sans retaper le chemin. Chaque entrée peut être retirée du suivi (×).
#[component]
pub fn SpaceBar(
    space: Signal<Option<ContextSpace>>,
    space_path: Signal<String>,
    known: Signal<Vec<KnownSpace>>,
) -> Element {
    let mut browsing = use_signal(|| false);
    let mut browse_dir = use_signal(orchestra_core::browser::home_dir);
    let mut creating = use_signal(|| false);
    let active = space().map(|s| s.config.project_name.clone());

    rsx! {
        div { class: "spaces",
            div { class: "spacebar",
                if let Some(name) = active {
                    span { class: "spacename", "● {name}" }
                }
                button { onclick: move |_| creating.set(!creating()),
                    if creating() { "Fermer" } else { "➕ Nouveau space" }
                }
                button { onclick: move |_| browsing.set(!browsing()),
                    if browsing() { "Fermer le navigateur" } else { "📂 Parcourir un dossier…" }
                }
            }

            // Formulaire de création d'un nouvel espace.
            if creating() {
                NewSpaceForm { space, space_path, known, creating }
            }

            // Espaces connus (récents) : rouvrir d'un clic.
            if !known().is_empty() {
                div { class: "chips",
                    span { class: "muted", "Récents :" }
                    for k in known() {
                        { space_chip(k, space, space_path, known) }
                    }
                }
            }

            // Navigateur de dossiers : explorer et ouvrir un espace repéré, sans saisie.
            if browsing() {
                {
                    let dir = browse_dir();
                    let dir_label = dir.to_string_lossy().to_string();
                    let entries = orchestra_core::browser::browse(&dir);
                    rsx! {
                        div { class: "browser",
                            div { class: "browsebar",
                                button { class: "row",
                                    onclick: move |_| {
                                        if let Some(p) = orchestra_core::browser::parent(&browse_dir()) { browse_dir.set(p); }
                                    },
                                    "⬆ .." }
                                span { class: "muted", "{dir_label}" }
                            }
                            ul { class: "list",
                                if entries.is_empty() {
                                    li { span { class: "muted", "(dossier vide)" } }
                                }
                                for e in entries {
                                    { browse_row(e, space, space_path, known, browse_dir, browsing) }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn browse_row(
    e: orchestra_core::browser::DirEntry,
    space: Signal<Option<ContextSpace>>,
    space_path: Signal<String>,
    known: Signal<Vec<KnownSpace>>,
    mut browse_dir: Signal<std::path::PathBuf>,
    mut browsing: Signal<bool>,
) -> Element {
    let nav = e.path.clone();
    let open = e.path.clone();
    let adopt = e.path.clone();
    if e.is_space {
        rsx! {
            li { class: "skillrow",
                span { class: "row", "🧩 {e.name}" }
                button { class: "linklike",
                    onclick: move |_| {
                        if state::open_space(space, space_path, known, &open.to_string_lossy()) {
                            browsing.set(false);
                        }
                    },
                    "ouvrir" }
            }
        }
    } else {
        rsx! {
            li { class: "skillrow",
                button { class: "row", onclick: move |_| browse_dir.set(nav.clone()), "📁 {e.name}" }
                button { class: "linklike",
                    onclick: move |_| {
                        if state::adopt_project(space, space_path, known, &adopt) {
                            browsing.set(false);
                        }
                    },
                    "reprendre (Dev)" }
            }
        }
    }
}

/// Formulaire de création d'un nouvel espace (état local). Les agents/skills se règlent ensuite
/// dans l'onglet « Agents & skills » (catalogue complet selon le type de projet).
#[component]
fn NewSpaceForm(
    space: Signal<Option<ContextSpace>>,
    space_path: Signal<String>,
    known: Signal<Vec<KnownSpace>>,
    mut creating: Signal<bool>,
) -> Element {
    let mut parent = use_signal(|| orchestra_core::browser::home_dir().to_string_lossy().to_string());
    let mut name = use_signal(String::new);
    let mut kind = use_signal(|| ProjectType::Dev);
    let mut workspace = use_signal(String::new);
    let mut objectives = use_signal(String::new);
    let mut documentalist = use_signal(|| false);
    let mut err = use_signal(String::new);
    let doc_cls = if documentalist() { "tab on" } else { "tab" };

    rsx! {
        div { class: "newspace",
            input { class: "chatinput", value: "{parent}", placeholder: "Dossier parent",
                oninput: move |e| parent.set(e.value()) }
            input { class: "chatinput", value: "{name}", placeholder: "Nom du projet",
                oninput: move |e| name.set(e.value()) }
            div { class: "types",
                { type_btn(ProjectType::Dev, kind) }
                { type_btn(ProjectType::Langue, kind) }
            }
            if kind() == ProjectType::Dev {
                input { class: "chatinput", value: "{workspace}", placeholder: "Workspace (chemin du code)",
                    oninput: move |e| workspace.set(e.value()) }
            }
            textarea { class: "fichearea", value: "{objectives}",
                placeholder: "Objectifs / description du projet…",
                oninput: move |e| objectives.set(e.value()) }
            button { class: "{doc_cls}", onclick: move |_| documentalist.set(!documentalist()),
                if documentalist() { "📝 Documentaliste : activé" } else { "📝 Documentaliste : désactivé" }
            }
            div { class: "actions",
                button { class: "go",
                    onclick: move |_| {
                        match state::create_space(space, space_path, known, &parent(), &name(), kind(), &workspace(), &objectives(), documentalist()) {
                            Ok(()) => creating.set(false),
                            Err(e) => err.set(e),
                        }
                    },
                    "Créer l'espace" }
                button { onclick: move |_| creating.set(false), "Annuler" }
            }
            if !err().is_empty() {
                p { class: "error", "{err}" }
            }
        }
    }
}

fn type_btn(k: ProjectType, mut kind: Signal<ProjectType>) -> Element {
    let cls = if kind() == k { "tab on" } else { "tab" };
    let label = k.label();
    rsx! {
        button { class: "{cls}", onclick: move |_| kind.set(k), "{label}" }
    }
}

fn space_chip(
    k: KnownSpace,
    space: Signal<Option<ContextSpace>>,
    space_path: Signal<String>,
    known: Signal<Vec<KnownSpace>>,
) -> Element {
    let path_open = k.path.clone();
    let path_forget = k.path.clone();
    rsx! {
        span { class: "chip",
            button { class: "chiplabel",
                onclick: move |_| { state::open_space(space, space_path, known, &path_open.to_string_lossy()); },
                "{k.name}"
            }
            button { class: "chipx", onclick: move |_| state::forget_space_entry(known, &path_forget), "×" }
        }
    }
}

/// Charge mermaid.js (une fois, depuis le CDN) puis **rend** les blocs `.mermaid` présents. Si
/// mermaid n'est pas (encore) disponible, le bloc affiche son code source — dégradé propre.
const MERMAID_BOOT: &str = r#"
(function(){
  function render(){ try {
    window.mermaid.initialize({ startOnLoad:false, theme:'dark', securityLevel:'loose' });
    window.mermaid.run({ querySelector: '.mermaid:not([data-processed])' });
  } catch(e){} }
  if (window.mermaid) { render(); return; }
  var ex = document.getElementById('mermaid-cdn');
  if (!ex) {
    var s = document.createElement('script');
    s.id = 'mermaid-cdn';
    s.src = 'https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.min.js';
    s.onload = render;
    document.head.appendChild(s);
  } else { setTimeout(render, 300); }
})();
"#;

/// Vue Documents : liste des documents + visualiseur **Markdown rendu** (titres, listes, code,
/// tableaux) et **diagrammes Mermaid**, avec **édition** du document affiché (persona, memory…).
#[component]
pub fn DocumentsView(space: Signal<Option<ContextSpace>>, content: Signal<String>) -> Element {
    let docs = space().map(|s| s.documents()).unwrap_or_default();
    let html = state::render_markdown_html(&content());

    // Chemin du document affiché + état d'édition (textarea) + brouillon.
    let mut sel_path = use_signal(|| None::<PathBuf>);
    let mut editing = use_signal(|| false);
    let mut draft = use_signal(String::new);

    // Après chaque changement de document, (re)rend les diagrammes Mermaid dans la webview.
    use_effect(move || {
        let _ = content(); // dépendance réactive : re-déclenche au changement de contenu
        spawn(async move {
            let _ = dioxus::document::eval(MERMAID_BOOT).await;
        });
    });

    rsx! {
        div { class: "cols",
            ul { class: "list",
                for d in docs {
                    { doc_item(d.label.clone(), d.path.clone(), content, sel_path, editing) }
                }
            }
            div { class: "viewerpane",
                // Barre d'actions : éditer / enregistrer / annuler.
                div { class: "docactions",
                    if editing() {
                        button { class: "go",
                            onclick: move |_| {
                                if let Some(p) = sel_path() {
                                    if state::save_document(&p, &draft()) { content.set(draft()); }
                                }
                                editing.set(false);
                            },
                            "💾 Enregistrer" }
                        button { onclick: move |_| editing.set(false), "Annuler" }
                    } else if sel_path().is_some() {
                        button { onclick: move |_| { draft.set(content()); editing.set(true); }, "✏ Éditer" }
                    }
                }

                if editing() {
                    textarea { class: "fichearea", value: "{draft}",
                        oninput: move |e| draft.set(e.value()) }
                } else if content().is_empty() {
                    div { class: "viewer markdown", p { class: "muted", "Sélectionne un document à gauche." } }
                } else {
                    div { class: "viewer markdown", dangerous_inner_html: html }
                }
            }
        }
    }
}

fn doc_item(
    label: String,
    path: PathBuf,
    mut content: Signal<String>,
    mut sel_path: Signal<Option<PathBuf>>,
    mut editing: Signal<bool>,
) -> Element {
    rsx! {
        li {
            button { class: "row", onclick: move |_| {
                    if let Ok(text) = orchestra_core::model::load_document(&path) {
                        content.set(text);
                        sel_path.set(Some(path.clone()));
                        editing.set(false); // on quitte l'édition en changeant de document
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
