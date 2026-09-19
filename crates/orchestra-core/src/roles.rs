//! The role catalog.
//!
//! A role is a Markdown file with YAML frontmatter, deliberately close to what
//! Claude Code uses for its own sub-agents so the format is already familiar:
//!
//! ```text
//! ---
//! name: backend
//! description: Implémente la logique serveur et les migrations.
//! model: sonnet
//! effort: high
//! ---
//! Tu es l'ingénieur backend de l'équipe…
//! ```
//!
//! Global roles live in `~/.config/orchestra/roles`, and a project can override
//! any of them by name in `<projet>/.orchestra/roles`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{CoreError, Result};
use crate::model::{Effort, RoleDefinition, RoleScope};

/// Fields accepted in the frontmatter.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Frontmatter {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    effort: Option<String>,
    #[serde(default)]
    allowed_tools: Vec<String>,
    #[serde(default)]
    disallowed_tools: Vec<String>,
    #[serde(default)]
    max_budget_usd: Option<f64>,
    #[serde(default)]
    subagents: Option<serde_json::Value>,
    #[serde(default)]
    tags: Vec<String>,
}

/// Split a Markdown file into its frontmatter and body.
fn split_frontmatter(src: &str) -> Result<(&str, &str)> {
    let rest = src
        .strip_prefix("---\n")
        .or_else(|| src.strip_prefix("---\r\n"))
        .ok_or_else(|| {
            CoreError::Parse("le fichier doit commencer par une entête « --- »".into())
        })?;
    let end = rest
        .find("\n---\n")
        .or_else(|| rest.find("\r\n---\r\n"))
        .ok_or_else(|| CoreError::Parse("entête « --- » non refermée".into()))?;
    let header = &rest[..end];
    let body = rest[end..]
        .trim_start_matches(['\n', '\r'])
        .trim_start_matches("---")
        .trim_start_matches(['\n', '\r']);
    Ok((header, body))
}

pub fn parse_role(path: &Path, src: &str, scope: RoleScope) -> Result<RoleDefinition> {
    let (header, body) = split_frontmatter(src)?;
    let fm: Frontmatter = serde_yaml_ng::from_str(header)?;
    let name = fm.name.trim().to_string();
    if name.is_empty() {
        return Err(CoreError::Parse("le rôle n'a pas de nom".into()));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(CoreError::Parse(format!(
            "nom de rôle invalide « {name} » : lettres, chiffres, tiret et souligné seulement"
        )));
    }
    let body = body.trim();
    if body.is_empty() {
        return Err(CoreError::Parse(format!(
            "le rôle « {name} » n'a pas de consigne sous son entête"
        )));
    }
    let effort = fm.effort.as_deref().map(Effort::parse).transpose()?;

    Ok(RoleDefinition {
        name,
        description: fm.description.trim().to_string(),
        model: fm.model.filter(|m| !m.trim().is_empty()),
        effort,
        allowed_tools: fm.allowed_tools,
        disallowed_tools: fm.disallowed_tools,
        max_budget_usd: fm.max_budget_usd,
        subagents: fm.subagents,
        tags: fm.tags,
        system_prompt: body.to_string(),
        source: path.to_path_buf(),
        scope,
    })
}

/// Roles found in one directory, sorted by name. A file that fails to parse is
/// reported rather than silently dropped: a broken role is a bug the user wants
/// to know about, not a role that quietly disappears.
pub fn load_dir(dir: &Path, scope: RoleScope) -> (Vec<RoleDefinition>, Vec<(PathBuf, CoreError)>) {
    let mut roles = Vec::new();
    let mut errors = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return (roles, errors),
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        // `_footer.md` and friends are fragments, not roles.
        .filter(|p| {
            !p.file_name()
                .map(|n| n.to_string_lossy().starts_with('_'))
                .unwrap_or(true)
        })
        .collect();
    paths.sort();

    for path in paths {
        match std::fs::read_to_string(&path) {
            Ok(src) => match parse_role(&path, &src, scope) {
                Ok(role) => roles.push(role),
                Err(e) => errors.push((path, e)),
            },
            Err(e) => errors.push((
                path.clone(),
                CoreError::Parse(format!("lecture impossible : {e}")),
            )),
        }
    }
    roles.sort_by(|a, b| a.name.cmp(&b.name));
    (roles, errors)
}

/// The catalog a ticket sees: global roles, with the project's own overriding
/// them by name.
pub struct Catalog {
    pub roles: Vec<RoleDefinition>,
    /// Files that could not be read, so the UI can say which and why.
    pub errors: Vec<(PathBuf, CoreError)>,
}

impl Catalog {
    pub fn load(global_dir: &Path, project_dir: Option<&Path>) -> Self {
        let (mut roles, mut errors) = load_dir(global_dir, RoleScope::Global);
        if let Some(dir) = project_dir {
            let (project_roles, project_errors) = load_dir(dir, RoleScope::Project);
            errors.extend(project_errors);
            for role in project_roles {
                match roles.iter().position(|r| r.name == role.name) {
                    Some(i) => roles[i] = role,
                    None => roles.push(role),
                }
            }
        }
        roles.sort_by(|a, b| a.name.cmp(&b.name));
        Catalog { roles, errors }
    }

    pub fn get(&self, name: &str) -> Option<&RoleDefinition> {
        self.roles.iter().find(|r| r.name == name)
    }

    pub fn names(&self) -> BTreeSet<String> {
        self.roles.iter().map(|r| r.name.clone()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.roles.is_empty()
    }

    /// One line per role, as handed to the orchestrator.
    pub fn summary_lines(&self) -> String {
        self.roles
            .iter()
            .map(|r| {
                let model = r.model.as_deref().unwrap_or("par défaut");
                format!("- {} ({model}) : {}", r.name, r.description)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BACKEND: &str = r#"---
name: backend
description: Implémente la logique serveur, les modèles et les migrations.
model: sonnet
effort: high
max_budget_usd: 5
disallowed_tools: ["WebSearch"]
tags: [impl]
---
Tu es l'ingénieur backend de l'équipe.
Tu écris du code testé.
"#;

    fn parse(src: &str) -> Result<RoleDefinition> {
        parse_role(Path::new("/tmp/r.md"), src, RoleScope::Global)
    }

    #[test]
    fn a_complete_role_parses() {
        let r = parse(BACKEND).unwrap();
        assert_eq!(r.name, "backend");
        assert_eq!(r.model.as_deref(), Some("sonnet"));
        assert_eq!(r.effort, Some(Effort::High));
        assert_eq!(r.max_budget_usd, Some(5.0));
        assert_eq!(r.disallowed_tools, vec!["WebSearch".to_string()]);
        assert_eq!(r.tags, vec!["impl".to_string()]);
        assert!(r.system_prompt.starts_with("Tu es l'ingénieur backend"));
        assert!(r.system_prompt.ends_with("testé."), "le corps est nettoyé");
        assert_eq!(r.scope, RoleScope::Global);
    }

    #[test]
    fn only_a_name_and_a_body_are_required() {
        let r = parse("---\nname: docs\n---\nÉcris la documentation.\n").unwrap();
        assert_eq!(r.name, "docs");
        assert!(r.model.is_none());
        assert!(r.effort.is_none());
        assert_eq!(r.description, "");
    }

    #[test]
    fn broken_files_are_rejected_with_a_reason() {
        // No frontmatter at all.
        assert!(parse("Juste du texte").is_err());
        // Unterminated header.
        assert!(parse("---\nname: x\nPas de fin").is_err());
        // Empty body: a role with no instruction is useless.
        let err = parse("---\nname: x\n---\n\n").unwrap_err();
        assert!(err.to_string().contains("consigne"), "{err}");
        // Unknown effort.
        assert!(parse("---\nname: x\neffort: turbo\n---\ncorps\n").is_err());
        // A typo in a field name is caught rather than ignored.
        assert!(parse("---\nname: x\nmodele: sonnet\n---\ncorps\n").is_err());
        // A name that would not survive a file path or a command line.
        assert!(parse("---\nname: \"back end\"\n---\ncorps\n").is_err());
        assert!(parse("---\nname: \"../evil\"\n---\ncorps\n").is_err());
    }

    #[test]
    fn a_project_role_overrides_the_global_one_of_the_same_name() {
        let dir = tempfile::tempdir().unwrap();
        let global = dir.path().join("global");
        let project = dir.path().join("project");
        std::fs::create_dir_all(&global).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(global.join("backend.md"), BACKEND).unwrap();
        std::fs::write(global.join("docs.md"), "---\nname: docs\n---\nÉcris.\n").unwrap();
        std::fs::write(
            project.join("backend.md"),
            "---\nname: backend\nmodel: opus\n---\nVersion du projet.\n",
        )
        .unwrap();

        let catalog = Catalog::load(&global, Some(&project));
        assert_eq!(catalog.roles.len(), 2, "pas de doublon");
        let backend = catalog.get("backend").unwrap();
        assert_eq!(backend.model.as_deref(), Some("opus"), "le projet gagne");
        assert_eq!(backend.scope, RoleScope::Project);
        assert!(catalog.get("docs").is_some(), "le global reste disponible");
        assert!(catalog.errors.is_empty());
    }

    #[test]
    fn a_broken_role_is_reported_not_swallowed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ok.md"), BACKEND).unwrap();
        std::fs::write(dir.path().join("casse.md"), "pas d'entête").unwrap();
        let catalog = Catalog::load(dir.path(), None);
        assert_eq!(catalog.roles.len(), 1);
        assert_eq!(catalog.errors.len(), 1);
        assert!(catalog.errors[0].0.ends_with("casse.md"));
    }

    #[test]
    fn fragments_are_not_roles() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("_footer.md"), "Règles communes.").unwrap();
        std::fs::write(dir.path().join("backend.md"), BACKEND).unwrap();
        let catalog = Catalog::load(dir.path(), None);
        assert_eq!(
            catalog.names().into_iter().collect::<Vec<_>>(),
            vec!["backend"]
        );
        assert!(
            catalog.errors.is_empty(),
            "un fragment n'est pas une erreur"
        );
    }

    #[test]
    fn a_missing_directory_is_an_empty_catalog() {
        let catalog = Catalog::load(Path::new("/nexiste/pas"), None);
        assert!(catalog.is_empty());
        assert!(catalog.errors.is_empty());
    }

    #[test]
    fn the_summary_lists_every_role_on_one_line() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("backend.md"), BACKEND).unwrap();
        let catalog = Catalog::load(dir.path(), None);
        let summary = catalog.summary_lines();
        assert!(summary.starts_with("- backend (sonnet) : Implémente"));
        assert_eq!(summary.lines().count(), 1);
    }
}
