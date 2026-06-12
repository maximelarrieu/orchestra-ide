//! Mémoire partagée d'un Espace de Contexte — le « tableau noir » des agents.
//!
//! Stockée dans `.orchestra/memory.md` (Markdown lisible par l'humain, durable d'une session
//! à l'autre), elle laisse les agents **consigner des faits, décisions et synthèses** que les
//! autres relisent — au lieu de relire les fichiers bruts et de refaire le travail. C'est
//! aussi un **levier d'économie de tokens** : un agent résume une source volumineuse une fois,
//! les autres lisent la synthèse (`Recall`), pas la source.
//!
//! Deux primitives universelles, exposées à tous les agents :
//! - [`REMEMBER`] — ajoute une note ;
//! - [`RECALL`] — relit la mémoire, avec un filtre par mot-clé optionnel (économise le contexte).

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::error::OrchestraError;
use crate::llm::ToolSpec;
use crate::skills::SkillOutcome;

/// Outil : consigner une note dans la mémoire partagée.
pub const REMEMBER: &str = "Remember";
/// Outil : relire la mémoire partagée (filtre optionnel).
pub const RECALL: &str = "Recall";

/// Chemin du fichier de mémoire d'un espace : `.orchestra/memory.md`.
pub fn memory_path(root: &Path) -> PathBuf {
    root.join(".orchestra").join("memory.md")
}

/// Lit la mémoire de l'espace (chaîne vide si le fichier n'existe pas encore).
pub fn read(root: &Path) -> String {
    fs::read_to_string(memory_path(root)).unwrap_or_default()
}

/// Ajoute une note attribuée à `agent`. Crée le fichier (avec en-tête) au besoin. Les notes
/// sont numérotées (`#N`) pour un ordre déterministe, sans dépendance à une horloge.
pub fn append(root: &Path, agent: &str, note: &str) -> Result<(), OrchestraError> {
    let note = note.trim();
    if note.is_empty() {
        return Ok(()); // rien à consigner — pas d'entrée vide
    }
    let path = memory_path(root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut content = fs::read_to_string(&path).unwrap_or_default();
    if content.is_empty() {
        content.push_str(
            "# Mémoire de l'espace\n\n\
             Notes partagées entre agents (faits, décisions, synthèses), durables entre sessions.\n\n",
        );
    }
    let n = content.lines().filter(|l| l.starts_with("- [#")).count() + 1;
    // Note sur une seule ligne (on neutralise les retours pour garder une entrée = une ligne).
    let note = note.replace('\n', " ");
    content.push_str(&format!("- [#{n} · {agent}] {note}\n"));
    fs::write(&path, content)?;
    Ok(())
}

/// Vrai si `name` est une primitive de mémoire (aiguillage côté runtime).
pub fn handles(name: &str) -> bool {
    name == REMEMBER || name == RECALL
}

/// Définitions d'outils de la mémoire, à exposer à chaque agent.
pub fn tool_definitions() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: REMEMBER.to_string(),
            description:
                "Consigne une note durable dans la mémoire partagée de l'espace (un fait, une \
                 décision, une synthèse) — lisible par les autres agents et les prochaines \
                 sessions. Privilégie des notes courtes et utiles."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "note": { "type": "string", "description": "La note à mémoriser (concise)" } },
                "required": ["note"]
            }),
        },
        ToolSpec {
            name: RECALL.to_string(),
            description:
                "Relit la mémoire partagée de l'espace. Fournis un mot-clé (`query`) pour ne \
                 récupérer que les notes pertinentes et économiser le contexte."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "query": { "type": "string", "description": "Mot-clé optionnel pour filtrer les notes" } }
            }),
        },
    ]
}

/// Exécute une primitive de mémoire. `root` est la racine de l'espace, `agent` l'auteur.
pub fn execute(name: &str, input: &Value, root: &Path, agent: &str) -> SkillOutcome {
    match name {
        REMEMBER => {
            let Some(note) = input.get("note").and_then(Value::as_str) else {
                return SkillOutcome::err("paramètre `note` manquant.");
            };
            match append(root, agent, note) {
                Ok(()) => SkillOutcome::ok("Note ajoutée à la mémoire de l'espace."),
                Err(e) => SkillOutcome::err(format!("écriture mémoire impossible : {e}")),
            }
        }
        RECALL => {
            let mem = read(root);
            if mem.trim().is_empty() {
                return SkillOutcome::ok("(mémoire vide)");
            }
            match input.get("query").and_then(Value::as_str) {
                Some(q) if !q.trim().is_empty() => {
                    let needle = q.to_lowercase();
                    let hits: Vec<&str> = mem
                        .lines()
                        .filter(|l| l.starts_with("- [#") && l.to_lowercase().contains(&needle))
                        .collect();
                    if hits.is_empty() {
                        SkillOutcome::ok(format!("(aucune note ne mentionne « {q} »)"))
                    } else {
                        SkillOutcome::ok(hits.join("\n"))
                    }
                }
                _ => SkillOutcome::ok(mem),
            }
        }
        other => SkillOutcome::err(format!("outil mémoire inconnu : {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("orch-mem-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".orchestra")).unwrap();
        dir
    }

    #[test]
    fn append_creates_header_and_numbers_entries() {
        let root = tmp();
        append(&root, "Agent_A", "premier fait").unwrap();
        append(&root, "Agent_B", "deuxième fait").unwrap();
        let mem = read(&root);
        assert!(mem.contains("# Mémoire de l'espace"));
        assert!(mem.contains("- [#1 · Agent_A] premier fait"));
        assert!(mem.contains("- [#2 · Agent_B] deuxième fait"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn append_ignores_empty_note() {
        let root = tmp();
        append(&root, "Agent_A", "   ").unwrap();
        assert!(read(&root).is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn recall_filters_by_query() {
        let root = tmp();
        append(&root, "Agent_A", "budget max 350k€").unwrap();
        append(&root, "Agent_A", "secteur préféré : Aix centre").unwrap();

        let all = execute(RECALL, &json!({}), &root, "x");
        assert!(!all.is_error && all.text.contains("budget") && all.text.contains("secteur"));

        let filtered = execute(RECALL, &json!({ "query": "budget" }), &root, "x");
        assert!(filtered.text.contains("budget"));
        assert!(!filtered.text.contains("secteur"));

        let miss = execute(RECALL, &json!({ "query": "piscine" }), &root, "x");
        assert!(miss.text.contains("aucune note"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn remember_via_execute_then_recall() {
        let root = tmp();
        let out = execute(REMEMBER, &json!({ "note": "synthèse: 12 annonces retenues" }), &root, "Agent_Filtrage");
        assert!(!out.is_error);
        let recall = execute(RECALL, &json!({ "query": "annonces" }), &root, "x");
        assert!(recall.text.contains("12 annonces retenues"));
        assert!(recall.text.contains("Agent_Filtrage"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn recall_empty_memory() {
        let root = tmp();
        let out = execute(RECALL, &json!({}), &root, "x");
        assert!(!out.is_error && out.text.contains("mémoire vide"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn remember_requires_note() {
        let root = tmp();
        assert!(execute(REMEMBER, &json!({}), &root, "x").is_error);
        let _ = fs::remove_dir_all(&root);
    }
}
