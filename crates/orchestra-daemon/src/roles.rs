//! Role files on disk: the few ways the daemon writes them.
//!
//! Parsing lives in `orchestra-core::roles`; this is the part that touches the
//! filesystem. A role has no « proposed » state like a rule: only the user
//! writes one, from the TUI or by hand, so what is written applies at the next
//! agent launched.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use orchestra_core::config::Paths;
use orchestra_core::guard::GitPolicy;
use orchestra_core::model::{Project, RoleDefinition, RoleScope};
use orchestra_core::roles::{self, Catalog};

/// The catalog a project sees, or the global one without a project.
pub fn catalog(roles_dir: &Path, project: Option<&Project>) -> Catalog {
    let project_dir = project.map(|p| Paths::project_roles_dir(&p.path));
    Catalog::load(roles_dir, project_dir.as_deref())
}

/// Where a new role goes: the project's catalog when there is a project, the
/// global one otherwise — the same answer as for a convention.
pub fn dir_for(roles_dir: &Path, project: Option<&Project>) -> PathBuf {
    match project {
        Some(p) => Paths::project_roles_dir(&p.path),
        None => roles_dir.to_path_buf(),
    }
}

/// Write a new role from the skeleton and return its path.
///
/// Refused when the name is already in the catalog: a skeleton written into a
/// project under an existing name would quietly replace the real role there.
pub fn create(roles_dir: &Path, project: Option<&Project>, name: &str) -> Result<PathBuf> {
    let name = name.trim();
    roles::check_name(name)?;
    if let Some(existing) = catalog(roles_dir, project).get(name) {
        bail!(
            "le rôle « {name} » existe déjà ({}) : « e » pour l'éditer",
            existing.source.display()
        );
    }
    let dir = dir_for(roles_dir, project);
    let path = dir.join(format!("{name}.md"));
    if path.exists() {
        bail!("{} existe déjà", path.display());
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("création de {}", dir.display()))?;
    std::fs::write(&path, roles::skeleton(name))
        .with_context(|| format!("écriture de {}", path.display()))?;
    Ok(path)
}

/// The role a name resolves to, as the project sees it.
pub fn find<'a>(catalog: &'a Catalog, name: &str) -> Result<&'a RoleDefinition> {
    catalog
        .get(name)
        .with_context(|| format!("rôle « {name} » introuvable"))
}

/// Rewrite the role's `git:` line, and nothing else of its file.
pub fn set_git(path: &Path, git: GitPolicy) -> Result<()> {
    let src =
        std::fs::read_to_string(path).with_context(|| format!("lecture de {}", path.display()))?;
    let out = roles::with_git(&src, git)?;
    // Read back before writing: a file that no longer parses would drop the
    // role from the catalog, which is worse than not changing it.
    roles::parse_role(path, &out, RoleScope::Global)?;
    std::fs::write(path, out).with_context(|| format!("écriture de {}", path.display()))
}

/// Change one setting of a role in its file, checked by reading it back.
pub fn set_setting(path: &Path, setting: &roles::RoleSetting) -> Result<()> {
    let src =
        std::fs::read_to_string(path).with_context(|| format!("lecture de {}", path.display()))?;
    let out = roles::with_setting(&src, setting)?;
    roles::parse_role(path, &out, RoleScope::Global)?;
    std::fs::write(path, out).with_context(|| format!("écriture de {}", path.display()))
}

/// Move a project's role to the global catalog. Refused when a global role of
/// that name exists: replacing it would change every other project.
pub fn promote(roles_dir: &Path, project: &Project, name: &str) -> Result<PathBuf> {
    let cat = catalog(roles_dir, Some(project));
    let role = find(&cat, name)?;
    if role.scope != RoleScope::Project {
        bail!("« {name} » est déjà un rôle global");
    }
    if let Some(global) = Catalog::load(roles_dir, None).get(name) {
        bail!(
            "un rôle global « {name} » existe déjà ({}) : renomme l'un des deux",
            global.source.display()
        );
    }
    let target = roles_dir.join(format!("{name}.md"));
    if target.exists() {
        bail!("{} existe déjà", target.display());
    }
    std::fs::create_dir_all(roles_dir)
        .with_context(|| format!("création de {}", roles_dir.display()))?;
    std::fs::copy(&role.source, &target)
        .with_context(|| format!("copie vers {}", target.display()))?;
    std::fs::remove_file(&role.source)
        .with_context(|| format!("suppression de {}", role.source.display()))?;
    Ok(target)
}

/// Delete the file behind a role, refusing to leave the daemon without one it
/// needs: the relecture and the integration name their role in the config,
/// and a ticket cannot end without them.
pub fn delete(
    roles_dir: &Path,
    project: Option<&Project>,
    name: &str,
    required: &[&str],
) -> Result<PathBuf> {
    let cat = catalog(roles_dir, project);
    let role = find(&cat, name)?;
    // A project's copy can go: the global role takes its place again.
    let survives = role.scope == RoleScope::Project
        && Catalog::load(roles_dir, None).get(name).is_some();
    if required.contains(&name) && !survives {
        bail!(
            "« {name} » est un rôle dont Orchestra a besoin (relecture ou intégration, \
             voir config.toml) : modifie-le plutôt que de le supprimer"
        );
    }
    std::fs::remove_file(&role.source)
        .with_context(|| format!("suppression de {}", role.source.display()))?;
    Ok(role.source.clone())
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

    #[test]
    fn a_role_is_created_opened_to_git_promoted_and_deleted() {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("roles");
        let p = project(&root.path().join("repo"));

        let path = create(&global, Some(&p), "data").unwrap();
        assert!(path.starts_with(Paths::project_roles_dir(&p.path)));
        let role = catalog(&global, Some(&p)).get("data").cloned().unwrap();
        assert_eq!(role.git, Some(GitPolicy::Confined));
        assert!(create(&global, Some(&p), "data").is_err(), "pas deux fois");
        assert!(create(&global, None, "mauvais nom").is_err());

        set_git(&role.source, GitPolicy::Full).unwrap();
        let role = catalog(&global, Some(&p)).get("data").cloned().unwrap();
        assert_eq!(role.git, Some(GitPolicy::Full));

        let target = promote(&global, &p, "data").unwrap();
        assert!(target.exists() && !path.exists());
        assert_eq!(
            catalog(&global, None).get("data").unwrap().scope,
            RoleScope::Global
        );
        assert!(promote(&global, &p, "data").is_err(), "déjà global");

        delete(&global, None, "data", &["reviewer", "integrator"]).unwrap();
        assert!(catalog(&global, None).get("data").is_none());
    }

    #[test]
    fn a_role_the_daemon_needs_is_not_deleted_but_its_project_copy_is() {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("roles");
        let p = project(&root.path().join("repo"));
        let required = ["reviewer", "integrator"];
        std::fs::create_dir_all(&global).unwrap();
        std::fs::write(global.join("integrator.md"), roles::skeleton("integrator")).unwrap();

        let err = delete(&global, None, "integrator", &required).unwrap_err();
        assert!(err.to_string().contains("besoin"), "{err}");

        let dir = Paths::project_roles_dir(&p.path);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("integrator.md"), roles::skeleton("integrator")).unwrap();
        delete(&global, Some(&p), "integrator", &required).unwrap();
        assert_eq!(
            catalog(&global, Some(&p)).get("integrator").unwrap().scope,
            RoleScope::Global,
            "le rôle global reprend sa place"
        );
    }
}

#[cfg(test)]
mod setting_tests {
    use super::*;

    #[test]
    fn a_setting_lands_in_the_file_and_a_broken_result_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("backend.md");
        std::fs::write(&path, "---\nname: backend\ndescription: b\n---\nTu es backend.\n").unwrap();
        set_setting(&path, &roles::RoleSetting::Model(Some("haiku".into()))).unwrap();
        let src = std::fs::read_to_string(&path).unwrap();
        assert!(src.contains("model: haiku"), "{src}");
        assert!(src.ends_with("Tu es backend.\n"));
        set_setting(&path, &roles::RoleSetting::Model(None)).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("model:"));
    }
}
