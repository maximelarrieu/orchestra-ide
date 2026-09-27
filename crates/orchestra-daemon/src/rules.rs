//! Rule files on disk: where they are, and the few ways the daemon writes them.
//!
//! The judging lives in `orchestra-core::conventions`; this is the part that
//! touches the filesystem. Agents never write here — a proposal they make is
//! written by the daemon, with `status: proposed`, into the project's own
//! `.orchestra` directory, never into their worktree.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use orchestra_core::config::Paths;
use orchestra_core::conventions::{self, RuleBook, RuleDraft, RuleKind, RuleStatus};
use orchestra_core::model::{Project, RoleScope};

/// The rules a project sees, falling back on the shipped conventions while
/// the global directory has never been installed.
pub fn book(conventions_dir: &Path, project: Option<&Project>) -> RuleBook {
    let project_dir = project.map(|p| Paths::project_dir(&p.path));
    RuleBook::load_or_bundled(conventions_dir, crate::init::CONVENTIONS, project_dir.as_deref())
}

/// One line per accepted decision of the repository at `repo`, empty when it
/// has none. What the orchestrator is told, so a team it proposes already
/// fits them.
pub fn adr_summary(repo: &Path) -> String {
    // Global conventions concern how agents work, not how a team is composed.
    RuleBook::load(Path::new(""), Some(&Paths::project_dir(repo))).adr_summary_lines()
}

/// Where a new rule of this kind goes: the project's directory when there is a
/// project, the global one for a convention without.
pub fn dir_for(conventions_dir: &Path, project: Option<&Project>, kind: RuleKind) -> Result<PathBuf> {
    match (project, kind) {
        (Some(p), kind) => Ok(Paths::project_dir(&p.path).join(kind.dir_name())),
        (None, RuleKind::Convention) => Ok(conventions_dir.to_path_buf()),
        (None, RuleKind::Adr) => bail!("un ADR appartient à un projet : choisis-en un"),
    }
}

/// A file name no other rule of this kind uses.
fn free_name(book: &RuleBook, dir: &Path, kind: RuleKind, title: &str) -> String {
    match kind {
        RuleKind::Adr => {
            let names: Vec<&str> = book
                .rules
                .iter()
                .filter(|r| r.kind == RuleKind::Adr)
                .map(|r| r.name.as_str())
                .collect();
            conventions::next_adr_name(names, title)
        }
        RuleKind::Convention => {
            let base = conventions::slug(title);
            let taken = |n: &str| {
                book.get(RuleKind::Convention, n).is_some() || dir.join(format!("{n}.md")).exists()
            };
            if !taken(&base) {
                return base;
            }
            (2..)
                .map(|i| format!("{base}-{i}"))
                .find(|n| !taken(n))
                .expect("un nom libre finit toujours par exister")
        }
    }
}

/// Write a new rule and return its name and path.
pub fn create(
    conventions_dir: &Path,
    project: Option<&Project>,
    draft: &RuleDraft<'_>,
) -> Result<(String, PathBuf)> {
    let dir = dir_for(conventions_dir, project, draft.kind)?;
    let book = book(conventions_dir, project);
    let name = free_name(&book, &dir, draft.kind, draft.title);
    let path = dir.join(format!("{name}.md"));
    let src = conventions::render(draft)?;
    std::fs::create_dir_all(&dir).with_context(|| format!("création de {}", dir.display()))?;
    std::fs::write(&path, src).with_context(|| format!("écriture de {}", path.display()))?;
    Ok((name, path))
}

/// The file behind a rule, refusing one that only exists in the binary: a
/// shipped convention not yet installed has nowhere to be written.
pub fn source_of(book: &RuleBook, kind: RuleKind, name: &str) -> Result<(PathBuf, String, RuleStatus)> {
    let rule = book
        .get(kind, name)
        .with_context(|| format!("{} « {name} » introuvable", kind.label_fr()))?;
    if !rule.source.exists() {
        bail!(
            "« {name} » est une convention livrée, pas encore installée : lance « orchestra init » \
             pour pouvoir la modifier"
        );
    }
    Ok((rule.source.clone(), rule.title.clone(), rule.status))
}

pub fn set_status(path: &Path, status: RuleStatus) -> Result<()> {
    let src = std::fs::read_to_string(path).with_context(|| format!("lecture de {}", path.display()))?;
    let out = conventions::with_status(&src, status)?;
    std::fs::write(path, out).with_context(|| format!("écriture de {}", path.display()))
}

/// Move a project's convention among the global ones. Refused when a global
/// convention of that name exists: silently replacing it would change every
/// other project.
pub fn promote(conventions_dir: &Path, project: &Project, name: &str) -> Result<(PathBuf, String)> {
    let book = book(conventions_dir, Some(project));
    let rule = book
        .get(RuleKind::Convention, name)
        .with_context(|| format!("convention « {name} » introuvable"))?;
    if rule.scope != RoleScope::Project {
        bail!("« {name} » est déjà une convention globale");
    }
    let target = conventions_dir.join(format!("{name}.md"));
    if target.exists() {
        bail!(
            "une convention globale « {name} » existe déjà ({}) : renomme l'une des deux",
            target.display()
        );
    }
    std::fs::create_dir_all(conventions_dir)
        .with_context(|| format!("création de {}", conventions_dir.display()))?;
    std::fs::copy(&rule.source, &target)
        .with_context(|| format!("copie vers {}", target.display()))?;
    std::fs::remove_file(&rule.source)
        .with_context(|| format!("suppression de {}", rule.source.display()))?;
    Ok((target, rule.title.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestra_core::model::ProjectKind;
    use uuid::Uuid;

    fn project(path: &Path) -> Project {
        Project {
            id: Uuid::new_v4(),
            name: "p".into(),
            path: path.to_path_buf(),
            default_branch: "main".into(),
            zellij_tab: None,
            kind: ProjectKind::Managed,
            created_at: orchestra_core::now(),
        }
    }

    fn draft<'a>(kind: RuleKind, title: &'a str) -> RuleDraft<'a> {
        RuleDraft {
            kind,
            title,
            status: RuleStatus::Proposed,
            applies_to: &[],
            proposed_by: Some("reviewer, ticket #1"),
            body: "Le texte.",
        }
    }

    #[test]
    fn created_rules_get_free_names_and_parse_back() {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("conventions");
        std::fs::create_dir_all(&global).unwrap();
        let p = project(&root.path().join("repo"));

        let (a, _) = create(&global, Some(&p), &draft(RuleKind::Convention, "Paginer")).unwrap();
        let (b, _) = create(&global, Some(&p), &draft(RuleKind::Convention, "Paginer")).unwrap();
        assert_eq!((a.as_str(), b.as_str()), ("paginer", "paginer-2"));
        let (first, _) = create(&global, Some(&p), &draft(RuleKind::Adr, "SQLite")).unwrap();
        let (second, _) = create(&global, Some(&p), &draft(RuleKind::Adr, "Pas de Redis")).unwrap();
        assert_eq!(first, "0001-sqlite");
        assert_eq!(second, "0002-pas-de-redis");
        assert!(create(&global, None, &draft(RuleKind::Adr, "X")).is_err());

        let book = book(&global, Some(&p));
        assert!(book.errors.is_empty(), "{:?}", book.errors);
        assert_eq!(book.pending(), 4);
    }

    #[test]
    fn accepting_and_promoting_a_project_convention() {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("conventions");
        std::fs::create_dir_all(&global).unwrap();
        let p = project(&root.path().join("repo"));
        let (name, path) = create(&global, Some(&p), &draft(RuleKind::Convention, "Paginer")).unwrap();

        let b = book(&global, Some(&p));
        let (src, _, status) = source_of(&b, RuleKind::Convention, &name).unwrap();
        assert_eq!(status, RuleStatus::Proposed);
        set_status(&src, RuleStatus::Accepted).unwrap();
        let b = book(&global, Some(&p));
        assert!(b.get(RuleKind::Convention, &name).unwrap().status.is_active());

        let (target, _) = promote(&global, &p, &name).unwrap();
        assert!(target.exists() && !path.exists());
        let b = book(&global, None);
        assert_eq!(b.get(RuleKind::Convention, &name).unwrap().scope, RoleScope::Global);
        assert!(promote(&global, &p, &name).is_err(), "plus rien à promouvoir");
    }

    #[test]
    fn a_shipped_convention_not_installed_cannot_be_edited() {
        let root = tempfile::tempdir().unwrap();
        let absent = root.path().join("conventions");
        let b = book(&absent, None);
        assert!(b.get(RuleKind::Convention, "commits").is_some());
        let err = source_of(&b, RuleKind::Convention, "commits").unwrap_err();
        assert!(err.to_string().contains("orchestra init"), "{err}");
    }
}
