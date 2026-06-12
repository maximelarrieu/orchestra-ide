//! Skills définis par fichier — le modèle « skill = dossier + `SKILL.md` ».
//!
//! Contrairement aux **primitives** exécutables ([`crate::skills`], du code Rust : lire un
//! fichier, lancer une commande, `Web_Fetch`…), un *skill Markdown* est une **fiche
//! d'instructions** déposée dans `.orchestra/skills/<id>/SKILL.md`. Aucun code, aucune
//! recompilation : on dépose (ou on crée depuis l'interface) un Markdown, et tout agent à
//! qui ce skill est assigné voit ses instructions injectées dans son prompt système.
//!
//! Le fichier suit la convention des *Agent Skills* : un en-tête `---` (clés `name` /
//! `description`) suivi du corps Markdown — les instructions proprement dites.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::error::OrchestraError;
use crate::llm::ToolSpec;
use crate::skills::SkillOutcome;

/// Nom du fichier d'instructions au sein du dossier d'un skill.
const SKILL_FILE: &str = "SKILL.md";

/// Outil de **divulgation progressive** : charge les instructions détaillées d'une fiche à la
/// demande. Le prompt système ne porte que nom + description (économie de tokens) ; l'agent
/// appelle `Load_Skill` quand il a besoin du « comment faire » complet.
pub const LOAD_SKILL: &str = "Load_Skill";

/// Vrai si `name` est la primitive de chargement de fiche (aiguillage côté runtime).
pub fn handles(name: &str) -> bool {
    name == LOAD_SKILL
}

/// Définition d'outil de [`LOAD_SKILL`], à exposer aux agents qui ont au moins une fiche assignée.
pub fn tool_definition() -> ToolSpec {
    ToolSpec {
        name: LOAD_SKILL.to_string(),
        description:
            "Charge les instructions détaillées d'une de tes compétences (fiche). Fournis son \
             `id` tel qu'indiqué dans la section « Compétences ». À appeler seulement quand tu as \
             besoin de la procédure complète."
                .to_string(),
        parameters: json!({
            "type": "object",
            "properties": { "id": { "type": "string", "description": "Identifiant de la fiche à charger" } },
            "required": ["id"]
        }),
    }
}

/// Exécute [`LOAD_SKILL`] : renvoie le corps Markdown de la fiche `id` de l'espace `root`.
pub fn execute(input: &Value, root: &Path) -> SkillOutcome {
    let Some(id) = input.get("id").and_then(Value::as_str) else {
        return SkillOutcome::err("paramètre `id` manquant.");
    };
    match load_all(root).into_iter().find(|m| m.id == id || m.name == id) {
        Some(m) if !m.instructions.trim().is_empty() => SkillOutcome::ok(m.instructions),
        Some(_) => SkillOutcome::ok(format!("(la fiche « {id} » ne contient pas encore d'instructions)")),
        None => SkillOutcome::err(format!("compétence « {id} » introuvable dans cet espace.")),
    }
}

/// Un skill décrit par un `SKILL.md` : métadonnées (nom/description) + instructions.
#[derive(Debug, Clone)]
pub struct MarkdownSkill {
    /// Identifiant = nom du dossier (ce que l'on assigne à un agent).
    pub id: String,
    /// Nom affichable (en-tête `name`, défaut = `id`).
    pub name: String,
    /// Résumé court (en-tête `description`) — sert à choisir/marquer le skill.
    pub description: String,
    /// Corps Markdown : le « comment faire » injecté dans le prompt de l'agent.
    pub instructions: String,
    /// Chemin du `SKILL.md` sur disque.
    pub path: PathBuf,
}

/// Dossier hébergeant les skills Markdown d'un espace : `.orchestra/skills/`.
pub fn skills_dir(root: &Path) -> PathBuf {
    root.join(".orchestra").join("skills")
}

/// Charge tous les skills Markdown de l'espace, triés par `id`. Dossier absent → liste vide
/// (l'accès disque reste dans le cœur ; l'UI ne lit jamais directement le système de fichiers).
pub fn load_all(root: &Path) -> Vec<MarkdownSkill> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(skills_dir(root)) else {
        return out;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Some(id) = dir.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let file = dir.join(SKILL_FILE);
        if let Ok(raw) = fs::read_to_string(&file) {
            out.push(parse(id, &raw, file));
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// Crée le squelette `.orchestra/skills/<id>/SKILL.md` et renvoie son chemin. Échoue si le
/// nom est invalide ou si le skill existe déjà (jamais d'écrasement silencieux).
pub fn create(root: &Path, name: &str, description: &str) -> Result<PathBuf, OrchestraError> {
    let id = sanitize_id(name);
    if id.is_empty() {
        return Err(OrchestraError::InvalidSkillName(name.to_string()));
    }
    let file = skills_dir(root).join(&id).join(SKILL_FILE);
    if file.exists() {
        return Err(OrchestraError::SkillAlreadyExists(id));
    }
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&file, template(&id, description.trim()))?;
    Ok(file)
}

/// Écrit le contenu d'un `SKILL.md` (édition depuis l'interface). Centralise l'écriture
/// disque dans le cœur, comme [`crate::model::ContextSpace::save_persona`].
pub fn save(path: &Path, content: &str) -> Result<(), OrchestraError> {
    fs::write(path, content)?;
    Ok(())
}

/// Normalise un nom en identifiant de dossier sûr : conserve lettres/chiffres et `_`/`-`,
/// remplace les espaces par `_`, et écarte tout le reste (anti-traversée de chemin).
fn sanitize_id(name: &str) -> String {
    name.trim()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('_'),
            c if c.is_alphanumeric() || c == '_' || c == '-' => Some(c),
            _ => None,
        })
        .collect()
}

/// Squelette d'un `SKILL.md` : en-tête `name`/`description` + corps guidé.
fn template(id: &str, description: &str) -> String {
    let description = if description.is_empty() {
        "Décris en une phrase ce que fait ce skill."
    } else {
        description
    };
    format!(
        "---\n\
         name: {id}\n\
         description: {description}\n\
         ---\n\n\
         # {id}\n\n\
         Décris ici, en Markdown, **comment** réaliser ce skill : étapes à suivre, critères\n\
         de qualité et format de sortie attendu. Ce texte est injecté dans le prompt de\n\
         chaque agent à qui le skill est assigné.\n\n\
         Tu peux t'appuyer sur les primitives déjà disponibles, par exemple :\n\
         - `Web_Fetch` pour lire le contenu d'une URL,\n\
         - `Read_File` / `Write_File_Validated` pour les fichiers du workspace,\n\
         - `Execute_Terminal_Command` pour lancer une commande.\n"
    )
}

/// Parse un `SKILL.md` : en-tête `---` optionnel (clés `name`/`description`) puis corps.
fn parse(id: &str, raw: &str, path: PathBuf) -> MarkdownSkill {
    let mut name = id.to_string();
    let mut description = String::new();
    let mut instructions = raw.trim().to_string();

    // En-tête entre deux lignes `---` (front matter minimal).
    if let Some(rest) = raw.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---") {
            let front = &rest[..end];
            instructions = rest[end + 4..].trim_start_matches(['\n', '\r']).trim().to_string();
            for line in front.lines() {
                if let Some((key, value)) = line.split_once(':') {
                    let value = value.trim().trim_matches('"').trim();
                    match key.trim() {
                        "name" if !value.is_empty() => name = value.to_string(),
                        "description" => description = value.to_string(),
                        _ => {}
                    }
                }
            }
        }
    }

    MarkdownSkill { id: id.to_string(), name, description, instructions, path }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("orch-mdskill-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn create_then_load_round_trips() {
        let root = tmp();
        fs::create_dir_all(&root).unwrap();

        let path = create(&root, "Creation Quiz", "Génère un quiz").unwrap();
        assert!(path.ends_with("Creation_Quiz/SKILL.md")); // espaces → « _ »

        let skills = load_all(&root);
        assert_eq!(skills.len(), 1);
        let s = &skills[0];
        assert_eq!(s.id, "Creation_Quiz");
        assert_eq!(s.description, "Génère un quiz");
        assert!(s.instructions.contains("# Creation_Quiz"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn create_refuses_duplicate_and_invalid() {
        let root = tmp();
        fs::create_dir_all(&root).unwrap();

        create(&root, "Web_Search", "").unwrap();
        assert!(matches!(
            create(&root, "Web_Search", ""),
            Err(OrchestraError::SkillAlreadyExists(_))
        ));
        assert!(matches!(
            create(&root, "  /// ", ""),
            Err(OrchestraError::InvalidSkillName(_))
        ));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn parse_reads_frontmatter_and_body() {
        let s = parse(
            "Demo",
            "---\nname: Joli Nom\ndescription: \"un résumé\"\n---\n\nCorps **ici**.",
            PathBuf::from("x"),
        );
        assert_eq!(s.name, "Joli Nom");
        assert_eq!(s.description, "un résumé");
        assert_eq!(s.instructions, "Corps **ici**.");
    }

    #[test]
    fn parse_without_frontmatter_uses_id_and_full_body() {
        let s = parse("Brut", "Juste du texte.", PathBuf::from("x"));
        assert_eq!(s.name, "Brut");
        assert!(s.description.is_empty());
        assert_eq!(s.instructions, "Juste du texte.");
    }

    #[test]
    fn load_all_missing_dir_is_empty() {
        assert!(load_all(Path::new("/nope/orch/zzz")).is_empty());
    }

    #[test]
    fn load_skill_returns_body_or_error() {
        let root = tmp();
        fs::create_dir_all(&root).unwrap();
        create(&root, "Web_Search", "Cherche sur le web").unwrap();

        let ok = execute(&json!({ "id": "Web_Search" }), &root);
        assert!(!ok.is_error && ok.text.contains("# Web_Search"));

        let missing = execute(&json!({ "id": "Inconnu" }), &root);
        assert!(missing.is_error);

        let no_id = execute(&json!({}), &root);
        assert!(no_id.is_error);

        let _ = fs::remove_dir_all(&root);
    }
}
