//! Composants de présentation — fonctions renvoyant un `Element`. Pas de logique métier :
//! elles lisent des signaux et déclenchent les ponts de [`crate::state`].

use std::collections::HashMap;

use dioxus::prelude::*;
use orchestra_core::session::Sessions;

use tokio::sync::mpsc::UnboundedSender;

use crate::state::{self, AgStatus, ChatMsg, DesktopSession, FileActivity, KnownSpace, MsgKind, PlanRow};

/// Barre d'**onglets** (sessions) : un onglet par session ouverte, l'active mise en évidence ;
/// clic pour basculer, × pour fermer. N'apparaît qu'à partir de deux sessions.
pub fn tabs_bar(mut sessions: Signal<Sessions<DesktopSession>>) -> Element {
    let titles = sessions.read().titles();
    let active = sessions.read().active_index();
    rsx! {
        div { class: "sessiontabs",
            for (i, title) in titles.into_iter().enumerate() {
                {
                    let cls = if i == active { "stab on" } else { "stab" };
                    rsx! {
                        span { class: "{cls}",
                            button { class: "stablabel", onclick: move |_| { sessions.write().switch_to(i); }, "{title}" }
                            button { class: "stabx", onclick: move |_| { sessions.write().close(i); }, "×" }
                        }
                    }
                }
            }
        }
    }
}

/// Encart « squad » : statut live de l'Orchestrateur et des sous-agents qu'il déploie à la
/// volée, pour **voir qui travaille** pendant l'orchestration / le chat. Plus aucun agent
/// pré-câblé — le roster se compose au fil des agents qui apparaissent.
#[component]
pub fn SquadPanel(status: Signal<HashMap<String, AgStatus>>) -> Element {
    let map = status();
    let mut roster: Vec<String> = vec![orchestra_core::runtime::COORDINATOR.to_string()];
    let mut others: Vec<String> = map
        .keys()
        .filter(|k| k.as_str() != orchestra_core::runtime::COORDINATOR)
        .cloned()
        .collect();
    others.sort();
    roster.extend(others);
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

/// Rail **Checkpoints** (colonne fine à gauche) : une pastille par message de l'Orchestrateur —
/// autant de **jalons** de la conversation. Cliquer une pastille **défile** la conversation
/// jusqu'à ce message (navigationnel, sans restauration). La dernière est mise en évidence.
pub fn checkpoint_rail(messages: Signal<Vec<ChatMsg>>) -> Element {
    let msgs = messages();
    let marks: Vec<usize> = msgs
        .iter()
        .enumerate()
        .filter(|(_, m)| matches!(m.kind, MsgKind::Coordinator))
        .map(|(i, _)| i)
        .collect();
    let last = marks.last().copied();
    rsx! {
        div { class: "checkpoints",
            span { class: "cplabel", "CHECKPOINTS" }
            div { class: "cpdots",
                for idx in marks {
                    {
                        let cls = if Some(idx) == last { "cpdot on" } else { "cpdot" };
                        rsx! {
                            button { class: "{cls}", title: "Aller à ce jalon",
                                onclick: move |_| {
                                    spawn(async move {
                                        let js = format!(
                                            "var e=document.getElementById('msg-{idx}'); if(e){{e.scrollIntoView({{behavior:'smooth',block:'center'}});}}"
                                        );
                                        let _ = dioxus::document::eval(&js).await;
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Panneau **Terminal** (sous le visualiseur de code) : les commandes réellement exécutées par
/// les agents et leur sortie.
pub fn terminal_panel(sessions: Signal<Sessions<DesktopSession>>) -> Element {
    let runs = {
        let s = sessions.read();
        s.active().map(|a| a.terminal.clone()).unwrap_or_default()
    };
    rsx! {
        div { class: "termpanel",
            div { class: "termhead", "TERMINAL" }
            div { class: "termbody",
                if runs.is_empty() {
                    p { class: "muted", "Les commandes lancées par les agents s'afficheront ici." }
                }
                for (i, r) in runs.into_iter().enumerate() {
                    {
                        let mark = if r.ok { "✓" } else { "✗" };
                        rsx! {
                            div { key: "{i}", class: "termrun",
                                div { class: "termcmd", "→ {r.command}" }
                                pre { class: "termout", "{r.output}" }
                                div { class: "termexit", "{mark} {r.agent}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Rail **Tâches** (droite) : plan de l'Orchestrateur (checklist + approbation), Mémoire de
/// l'espace et Contexte (fichiers touchés). Réel : plan/mémoire/contexte viennent des données
/// vivantes de la session.
#[component]
pub fn TaskRail(
    plan: Signal<Vec<PlanRow>>,
    mut pending: Signal<bool>,
    approve_tx: Signal<Option<UnboundedSender<bool>>>,
    sessions: Signal<Sessions<DesktopSession>>,
) -> Element {
    let rows = plan();
    let done = rows.iter().filter(|r| r.status.contains('✓')).count();
    let total = rows.len();
    // Mémoire + contexte de la session active.
    let (memory, context) = {
        let s = sessions.read();
        match s.active() {
            Some(a) => {
                let mem = state::memory_entries(&a.space.root);
                let mut ctx: Vec<String> = a.activity.keys().cloned().collect();
                ctx.sort();
                (mem, ctx)
            }
            None => (Vec::new(), Vec::new()),
        }
    };
    let is_pending = pending();

    rsx! {
        div { class: "taskrail",
            // --- Plan ---
            div { class: "railhead", span { "TASK · PLAN" } span { class: "railcount", "{done}/{total} done" } }
            if rows.is_empty() {
                p { class: "muted small", "Le plan de l'Orchestrateur apparaîtra ici lorsqu'il en proposera un." }
            }
            ul { class: "plan",
                for row in rows {
                    {
                        // Icône typographique nette ; l'étape « en cours » utilise un vrai
                        // spinner CSS (via la classe, contenu vide).
                        let (icon, cls) = if row.status.contains('✓') {
                            ("✓", "planrow done")
                        } else if row.status.contains("cours") {
                            ("", "planrow running")
                        } else if row.status.contains('✗') {
                            ("✕", "planrow failed")
                        } else {
                            ("○", "planrow")
                        };
                        rsx! {
                            li { key: "{row.id}", class: "{cls}",
                                span { class: "planicon", "{icon}" }
                                div { class: "plantext",
                                    span { class: "planobj", "{row.objective}" }
                                    span { class: "planagent", "{row.agent}" }
                                }
                            }
                        }
                    }
                }
            }
            if is_pending {
                div { class: "approvecard",
                    span { class: "small", "L'Orchestrateur attend ton feu vert." }
                    button { class: "go",
                        onclick: move |_| {
                            if let Some(tx) = approve_tx() { let _ = tx.send(true); }
                            pending.set(false);
                        },
                        "Approve →" }
                }
            }

            // --- Mémoire ---
            div { class: "railhead", span { "MEMORY" } }
            if memory.is_empty() {
                p { class: "muted small", "Vide — les agents y consignent faits et décisions durables." }
            }
            for (i, m) in memory.into_iter().enumerate() {
                div { key: "m{i}", class: "memcard", "{m}" }
            }

            // --- Contexte ---
            div { class: "railhead", span { "CONTEXT · {context.len()}" } }
            if context.is_empty() {
                p { class: "muted small", "Fichiers lus/écrits par les agents pendant la session." }
            }
            for (i, c) in context.into_iter().enumerate() {
                div { key: "c{i}", class: "ctxrow", "@ {c}" }
            }
        }
    }
}

/// Barre d'espaces : saisie d'un chemin + **liste des espaces connus** (récents) à rouvrir d'un
/// clic, sans retaper le chemin. Chaque entrée peut être retirée du suivi (×).
#[component]
pub fn SpaceBar(
    sessions: Signal<Sessions<DesktopSession>>,
    known: Signal<Vec<KnownSpace>>,
) -> Element {
    let mut browsing = use_signal(|| false);
    let mut browse_dir = use_signal(orchestra_core::browser::home_dir);
    let mut creating = use_signal(|| false);
    let active = sessions.read().active().map(|s| s.space.config.project_name.clone());

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
                NewSpaceForm { sessions, known, creating }
            }

            // Espaces connus (récents) : rouvrir d'un clic (dans un onglet).
            if !known().is_empty() {
                div { class: "chips",
                    span { class: "muted", "Récents :" }
                    for k in known() {
                        { space_chip(k, sessions, known) }
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
                                    { browse_row(e, sessions, known, browse_dir, browsing) }
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
    sessions: Signal<Sessions<DesktopSession>>,
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
                        if state::open_space(sessions, known, &open.to_string_lossy()) {
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
                        if state::adopt_project(sessions, known, &adopt) {
                            browsing.set(false);
                        }
                    },
                    "reprendre (Dev)" }
            }
        }
    }
}

/// Formulaire de création d'un nouvel espace (état local). Les agents/skills se règlent ensuite
/// (l'Orchestrateur déploie ensuite sa propre équipe).
#[component]
fn NewSpaceForm(
    sessions: Signal<Sessions<DesktopSession>>,
    known: Signal<Vec<KnownSpace>>,
    mut creating: Signal<bool>,
) -> Element {
    let mut parent = use_signal(|| orchestra_core::browser::home_dir().to_string_lossy().to_string());
    let mut name = use_signal(String::new);
    let mut workspace = use_signal(String::new);
    let mut objectives = use_signal(String::new);
    let mut err = use_signal(String::new);

    rsx! {
        div { class: "newspace",
            input { class: "chatinput", value: "{parent}", placeholder: "Dossier parent",
                oninput: move |e| parent.set(e.value()) }
            input { class: "chatinput", value: "{name}", placeholder: "Nom du projet",
                oninput: move |e| name.set(e.value()) }
            input { class: "chatinput", value: "{workspace}", placeholder: "Workspace (chemin du code, optionnel)",
                oninput: move |e| workspace.set(e.value()) }
            textarea { class: "fichearea", value: "{objectives}",
                placeholder: "Objectifs / description du projet…",
                oninput: move |e| objectives.set(e.value()) }
            div { class: "actions",
                button { class: "go",
                    onclick: move |_| {
                        match state::create_space(sessions, known, &parent(), &name(), &workspace(), &objectives()) {
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

fn space_chip(
    k: KnownSpace,
    sessions: Signal<Sessions<DesktopSession>>,
    known: Signal<Vec<KnownSpace>>,
) -> Element {
    let path_open = k.path.clone();
    let path_forget = k.path.clone();
    rsx! {
        span { class: "chip",
            button { class: "chiplabel",
                onclick: move |_| { state::open_space(sessions, known, &path_open.to_string_lossy()); },
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

/// **Explorateur** (panneau gauche) : arborescence du workspace de la session active, **annotée
/// en temps réel** par l'activité des agents (« orchestre en verre ») — un liseré et un badge
/// indiquent quel agent **lit** (👁) ou **écrit** (✎) chaque fichier. Cliquer un fichier l'ouvre
/// au centre.
#[component]
pub fn FileExplorer(
    sessions: Signal<Sessions<DesktopSession>>,
    selected: Signal<Option<String>>,
) -> Element {
    let s = sessions.read();
    let Some(sess) = s.active() else {
        return rsx! { div { class: "explorer", p { class: "muted", "Aucune session ouverte." } } };
    };
    let tree = orchestra_core::explorer::tree(sess.workspace_root());
    let activity = sess.activity.clone();
    let cur = selected();
    rsx! {
        div { class: "explorer",
            ul { class: "tree",
                if tree.nodes.is_empty() {
                    li { span { class: "muted", "(workspace vide ou introuvable)" } }
                }
                for node in tree.nodes {
                    { file_row(node.clone(), activity.get(&node.rel).cloned(), cur.clone(), selected) }
                }
                if tree.truncated {
                    li { span { class: "muted", "… (arborescence tronquée)" } }
                }
            }
        }
    }
}

/// Une ligne de l'explorateur (fichier ou dossier), indentée selon la profondeur et annotée de
/// l'activité agent éventuelle.
fn file_row(
    node: orchestra_core::explorer::FileNode,
    act: Option<FileActivity>,
    selected_rel: Option<String>,
    mut selected: Signal<Option<String>>,
) -> Element {
    let pad = format!("padding-left: {}rem;", 0.55 + node.depth as f32 * 0.85);
    let is_sel = selected_rel.as_deref() == Some(node.rel.as_str());
    // Classe : dossier (non cliquable) ou fichier ; sélection ; activité (read/write).
    let mut cls = String::from(if node.is_dir { "trow dir" } else { "trow file" });
    if is_sel {
        cls.push_str(" on");
    }
    match &act {
        Some(a) if a.write => cls.push_str(" act-write"),
        Some(_) => cls.push_str(" act-read"),
        None => {}
    }
    // Badge d'activité : nom de l'agent, coloré (vert = écriture, bleu = lecture). Le liseré
    // gauche renforce la distinction — pas d'icône.
    let badge = act.map(|a| {
        if a.write {
            ("actbadge write", a.agent)
        } else {
            ("actbadge read", a.agent)
        }
    });
    let rel = node.rel.clone();
    let name = node.name.clone();
    let is_dir = node.is_dir;
    rsx! {
        li {
            div { class: "{cls}", style: "{pad}",
                if is_dir {
                    span { class: "tname", "{name}" }
                } else {
                    button { class: "tname", onclick: move |_| selected.set(Some(rel.clone())), "{name}" }
                }
                if let Some((bcls, btxt)) = badge {
                    span { class: "{bcls}", "{btxt}" }
                }
            }
        }
    }
}

/// **Panneau central** : contenu du fichier sélectionné. Si un agent l'a modifié pendant la
/// session, on montre son **diff**. Sinon, on affiche le fichier — Markdown **rendu** (avec
/// diagrammes Mermaid) pour les `.md`, texte brut sinon — et on peut l'**éditer/enregistrer**.
#[component]
pub fn CenterPane(
    sessions: Signal<Sessions<DesktopSession>>,
    selected: Signal<Option<String>>,
) -> Element {
    // Hooks EN PREMIER (règle des hooks Dioxus : toujours appelés, même hors sélection).
    let mut editing = use_signal(|| false);
    let mut draft = use_signal(String::new);
    // (Re)rend les diagrammes Mermaid quand la sélection change (Markdown).
    use_effect(move || {
        let _ = selected();
        spawn(async move {
            let _ = dioxus::document::eval(MERMAID_BOOT).await;
        });
    });

    // Données de la session active, extraites sous le verrou de lecture puis relâchées.
    let (project, root, sel, diff, content) = {
        let s = sessions.read();
        let Some(sess) = s.active() else {
            return rsx! { div { class: "filepane", div { class: "welcome",
                h2 { "Orchestra IDE" }
                p { class: "muted", "Ouvre ou crée un espace pour commencer." }
            } } };
        };
        let project = sess.space.config.project_name.clone();
        let root = sess.workspace_root().to_path_buf();
        let sel = selected();
        let diff = sel.as_ref().and_then(|rel| {
            sess.changes.iter().rev().find(|c| &c.path == rel).map(|c| c.diff.clone())
        });
        let content = sel.as_ref().and_then(|rel| state::read_file_rel(&root, rel));
        (project, root, sel, diff, content)
    };

    let Some(rel) = sel else {
        return rsx! { div { class: "filepane", div { class: "welcome",
            h2 { "🎻 {project}" }
            p { class: "muted", "Sélectionne un fichier à gauche, ou discute avec l'Orchestrateur à droite. Les fichiers que les agents lisent et modifient s'illuminent dans l'explorateur en temps réel." }
        } } };
    };

    let is_md = rel.ends_with(".md");
    let abs = root.join(&rel);

    let html = content.as_deref().map(state::render_markdown_html).unwrap_or_default();
    let raw = content.clone().unwrap_or_default();
    let raw_for_edit = raw.clone(); // capture séparée pour le bouton « Éditer » (raw sert aussi au rendu)

    rsx! {
        div { class: "filepane",
            div { class: "centerhead",
                span { class: "path", "{rel}" }
                if diff.is_none() {
                    if editing() {
                        button { class: "go",
                            onclick: move |_| {
                                if state::save_document(&abs, &draft()) { editing.set(false); }
                            },
                            "💾 Enregistrer" }
                        button { onclick: move |_| editing.set(false), "Annuler" }
                    } else {
                        button { onclick: move |_| { draft.set(raw_for_edit.clone()); editing.set(true); }, "✏ Éditer" }
                    }
                }
            }
            div { class: "centerbody",
                if let Some(d) = diff {
                    // Fichier touché par un agent → on montre le diff coloré.
                    div { class: "diff",
                        for line in d.lines() {
                            {
                                let cls = if line.starts_with("+ ") { "dl add" }
                                    else if line.starts_with("- ") { "dl del" }
                                    else { "dl ctx" };
                                rsx! { div { class: "{cls}", "{line}" } }
                            }
                        }
                    }
                } else if editing() {
                    textarea { class: "editor", value: "{draft}",
                        oninput: move |e| draft.set(e.value()) }
                } else if content.is_none() {
                    p { class: "muted", "Fichier binaire ou illisible." }
                } else if is_md {
                    div { class: "viewer markdown", dangerous_inner_html: html }
                } else {
                    pre { class: "codeview", "{raw}" }
                }
            }
        }
    }
}


/// Vue Chat : conversation avec l'Orchestrateur (bulles + saisie). Le plan/approbation vit
/// désormais dans le rail Tâches (à droite), pas ici.
pub fn chat_view(
    messages: Signal<Vec<ChatMsg>>,
    thinking: Signal<bool>,
    mut draft: Signal<String>,
    user_tx: Signal<Option<UnboundedSender<String>>>,
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

    rsx! {
        div { class: "chat",
            div { class: "messages",
                for (i, m) in messages().into_iter().enumerate() {
                    ChatBubble { key: "{i}", idx: i, msg: m }
                }
                if thinking() {
                    div { class: "bubble coord", "…" }
                }
            }
            div { class: "composer",
                textarea {
                    class: "chatinput",
                    rows: "2",
                    value: "{draft}",
                    placeholder: "Message à l'Orchestrateur…  (Entrée pour envoyer · Maj+Entrée = saut de ligne)",
                    oninput: move |e| draft.set(e.value()),
                    onkeydown: send_key,
                }
                button { class: "send", onclick: send_click, "↑" }
            }
        }
    }
}

/// Une bulle de chat. Les messages d'**agent** sont **repliés par défaut** (l'Orchestrateur
/// les résume) : une flèche ▶/▼ déroule/cache leur texte. Chaque bulle porte un `id` (`msg-N`)
/// pour la navigation par le rail Checkpoints.
#[component]
fn ChatBubble(idx: usize, msg: ChatMsg) -> Element {
    let mut expanded = use_signal(|| false);
    let anchor = format!("msg-{idx}");
    match msg.kind {
        MsgKind::User => rsx! {
            div { id: "{anchor}", class: "bubble user", div { class: "text", "{msg.text}" } }
        },
        MsgKind::Coordinator => rsx! {
            div { id: "{anchor}", class: "bubble coord",
                span { class: "who", "{msg.who}" }
                div { class: "text", "{msg.text}" }
            }
        },
        MsgKind::System => rsx! {
            div { id: "{anchor}", class: "bubble system", "{msg.who} {msg.text}" }
        },
        MsgKind::Agent => rsx! {
            div { id: "{anchor}", class: "bubble agent",
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
