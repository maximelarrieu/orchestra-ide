//! `orchestra init`: put the catalog and the example config where they belong.
//!
//! Nothing here overwrites a file the user may have edited unless asked. The
//! role files are the interesting part: they are meant to be read and changed.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use orchestra_core::config::{Config, Paths};

/// The catalog shipped with the binary. `_orchestrator.md` is the planning
/// prompt and `_footer.md` the rules appended to every role, so both keep their
/// underscore and are not offered as roles.
const ROLES: &[(&str, &str)] = &[
    (
        "architect.md",
        include_str!("../../../assets/roles/architect.md"),
    ),
    (
        "backend.md",
        include_str!("../../../assets/roles/backend.md"),
    ),
    (
        "frontend.md",
        include_str!("../../../assets/roles/frontend.md"),
    ),
    ("tests.md", include_str!("../../../assets/roles/tests.md")),
    (
        "reviewer.md",
        include_str!("../../../assets/roles/reviewer.md"),
    ),
    ("docs.md", include_str!("../../../assets/roles/docs.md")),
    (
        "_footer.md",
        include_str!("../../../assets/roles/_footer.md"),
    ),
    (
        "_orchestrator.md",
        include_str!("../../../assets/roles/_orchestrator.md"),
    ),
];

const EXAMPLE_CONFIG: &str = include_str!("../../../assets/config.example.toml");

#[derive(Debug, Default, PartialEq, Eq)]
pub struct InitReport {
    pub written: Vec<PathBuf>,
    pub kept: Vec<PathBuf>,
}

impl InitReport {
    pub fn is_empty(&self) -> bool {
        self.written.is_empty() && self.kept.is_empty()
    }
}

/// Install the catalog and, if absent, a configuration file.
///
/// `force` rewrites role files that already exist; the configuration is never
/// overwritten, since it holds the user's own price table.
pub fn init(force: bool) -> Result<InitReport> {
    let config_dir = Paths::config_dir();
    let roles_dir = Paths::roles_dir();
    std::fs::create_dir_all(&roles_dir)
        .with_context(|| format!("création de {}", roles_dir.display()))?;

    let mut report = InitReport::default();
    for (name, body) in ROLES {
        let path = roles_dir.join(name);
        if path.exists() && !force {
            report.kept.push(path);
            continue;
        }
        std::fs::write(&path, body).with_context(|| format!("écriture de {}", path.display()))?;
        report.written.push(path);
    }

    let config_path = config_dir.join("config.toml");
    if config_path.exists() {
        report.kept.push(config_path);
    } else {
        std::fs::write(&config_path, EXAMPLE_CONFIG)
            .with_context(|| format!("écriture de {}", config_path.display()))?;
        report.written.push(config_path);
    }

    // A configuration that cannot be read would break the daemon silently.
    Config::load().context("la configuration installée est illisible")?;
    Ok(report)
}

/// Roles as they ship, for callers that want them without touching the disk.
pub fn bundled_roles() -> impl Iterator<Item = (&'static str, &'static str)> {
    ROLES.iter().copied()
}

/// True when a directory already holds a usable catalog.
pub fn has_catalog(roles_dir: &Path) -> bool {
    std::fs::read_dir(roles_dir)
        .map(|entries| {
            entries.flatten().any(|e| {
                let p = e.path();
                p.extension().is_some_and(|x| x == "md")
                    && !p
                        .file_name()
                        .map(|n| n.to_string_lossy().starts_with('_'))
                        .unwrap_or(true)
            })
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchestra_core::roles::Catalog;

    /// `init` writes to the user's real directories, so the test relocates them
    /// through the environment. Serialised because the environment is global.
    fn with_temp_config<T>(f: impl FnOnce(&Path) -> T) -> T {
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("ORCHESTRA_CONFIG_DIR");
        unsafe { std::env::set_var("ORCHESTRA_CONFIG_DIR", dir.path()) };
        let out = f(dir.path());
        match previous {
            Some(v) => unsafe { std::env::set_var("ORCHESTRA_CONFIG_DIR", v) },
            None => unsafe { std::env::remove_var("ORCHESTRA_CONFIG_DIR") },
        }
        out
    }

    #[test]
    fn init_installs_a_catalog_that_parses() {
        with_temp_config(|dir| {
            let report = init(false).unwrap();
            assert!(!report.written.is_empty());
            assert!(report.kept.is_empty());

            let roles_dir = dir.join("roles");
            assert!(has_catalog(&roles_dir));
            let catalog = Catalog::load(&roles_dir, None);
            assert!(
                catalog.errors.is_empty(),
                "les rôles livrés doivent être valides : {:?}",
                catalog.errors
            );
            let names: Vec<&str> = catalog.roles.iter().map(|r| r.name.as_str()).collect();
            assert_eq!(
                names,
                vec![
                    "architect",
                    "backend",
                    "docs",
                    "frontend",
                    "reviewer",
                    "tests"
                ]
            );
            // The fragments are installed but are not roles.
            assert!(roles_dir.join("_footer.md").exists());
            assert!(roles_dir.join("_orchestrator.md").exists());
            assert!(dir.join("config.toml").exists());
        });
    }

    #[test]
    fn init_does_not_trample_what_the_user_changed() {
        with_temp_config(|dir| {
            init(false).unwrap();
            let mine = dir.join("roles/backend.md");
            std::fs::write(&mine, "---\nname: backend\n---\nMa version.\n").unwrap();
            std::fs::write(dir.join("config.toml"), "[daemon]\nmax_attempts = 9\n").unwrap();

            let report = init(false).unwrap();
            assert!(report.written.is_empty(), "rien n'est réécrit");
            assert!(report.kept.contains(&mine));
            assert_eq!(
                std::fs::read_to_string(&mine).unwrap(),
                "---\nname: backend\n---\nMa version.\n"
            );
            assert!(std::fs::read_to_string(dir.join("config.toml"))
                .unwrap()
                .contains("max_attempts = 9"));
        });
    }

    #[test]
    fn force_restores_the_roles_but_never_the_config() {
        with_temp_config(|dir| {
            init(false).unwrap();
            let role = dir.join("roles/backend.md");
            std::fs::write(&role, "cassé").unwrap();
            std::fs::write(dir.join("config.toml"), "[daemon]\nmax_attempts = 9\n").unwrap();

            let report = init(true).unwrap();
            assert!(report.written.contains(&role));
            assert!(std::fs::read_to_string(&role).unwrap().contains("backend"));
            // The price table is the user's, even with --force.
            assert!(std::fs::read_to_string(dir.join("config.toml"))
                .unwrap()
                .contains("max_attempts = 9"));
        });
    }

    #[test]
    fn the_bundled_roles_are_all_parseable() {
        for (name, body) in bundled_roles() {
            if name.starts_with('_') {
                continue;
            }
            let role = orchestra_core::roles::parse_role(
                Path::new(name),
                body,
                orchestra_core::model::RoleScope::Global,
            )
            .unwrap_or_else(|e| panic!("{name} illisible : {e}"));
            assert!(!role.description.is_empty(), "{name} sans description");
            assert!(
                role.system_prompt.len() > 80,
                "{name} : consigne trop courte"
            );
        }
    }
}
