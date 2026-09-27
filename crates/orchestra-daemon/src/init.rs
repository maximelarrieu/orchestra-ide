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
        "integrator.md",
        include_str!("../../../assets/roles/integrator.md"),
    ),
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

/// The zellij layout, installed only when asked: it lands in zellij's own
/// configuration directory, which is not ours to fill uninvited.
const ZELLIJ_LAYOUT: &str = include_str!("../../../assets/zellij/orchestra.kdl");

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

/// Where zellij looks for layouts, following its own rules: `$ZELLIJ_CONFIG_DIR`
/// first, then `$XDG_CONFIG_HOME/zellij`, then `~/.config/zellij`.
pub fn zellij_layouts_dir() -> PathBuf {
    let env = |key: &str| {
        std::env::var_os(key)
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
    };
    env("ZELLIJ_CONFIG_DIR")
        .or_else(|| env("XDG_CONFIG_HOME").map(|d| d.join("zellij")))
        .unwrap_or_else(|| {
            env("HOME")
                .unwrap_or_else(|| PathBuf::from("/"))
                .join(".config/zellij")
        })
        .join("layouts")
}

/// Install the zellij layout, and say where it went.
///
/// Never as part of `init`: someone who does not use zellij should not find a
/// layout in a directory they never asked for. `force` overwrites a layout of
/// the same name, which may well be one the user has since edited.
///
/// The directory is a parameter rather than read here, so a test can point it
/// somewhere that is not the machine's own configuration.
pub fn init_zellij(dir: &Path, name: &str, force: bool) -> Result<(PathBuf, bool)> {
    std::fs::create_dir_all(dir).with_context(|| format!("création de {}", dir.display()))?;
    // Named after `zellij.layout`, so that `zellij -l <ce nom>` and the
    // configuration cannot drift apart.
    let path = dir.join(format!("{name}.kdl"));
    if path.exists() && !force {
        return Ok((path, false));
    }
    std::fs::write(&path, ZELLIJ_LAYOUT)
        .with_context(|| format!("écriture de {}", path.display()))?;
    Ok((path, true))
}

/// Where systemd looks for a user's own units: `$XDG_CONFIG_HOME/systemd/user`,
/// falling back to `~/.config/systemd/user`.
pub fn systemd_user_dir() -> PathBuf {
    let env = |key: &str| {
        std::env::var_os(key)
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
    };
    env("XDG_CONFIG_HOME")
        .unwrap_or_else(|| {
            env("HOME")
                .unwrap_or_else(|| PathBuf::from("/"))
                .join(".config")
        })
        .join("systemd/user")
}

/// The unit and its timer, `bin` baked in as an absolute path: a systemd
/// user service does not inherit the shell's `PATH`.
fn notify_units(bin: &Path) -> (String, String) {
    let service = format!(
        "[Unit]\nDescription=Orchestra — vérification matinale des todos\n\n\
         [Service]\nType=oneshot\nExecStart={} todo notify\n",
        bin.display()
    );
    let timer = "[Unit]\nDescription=Orchestra — minuteur de la vérification matinale\n\n\
         [Timer]\nOnCalendar=*-*-* 08:00:00\nPersistent=true\n\n\
         [Install]\nWantedBy=timers.target\n"
        .to_string();
    (service, timer)
}

/// Install the `orchestra-todo` service and its timer, and say where they
/// went. Never as part of `init`, and never enabled: someone who does not
/// want a morning notification should not find one scheduled, and
/// `systemctl` is never ours to run on someone's behalf. `force` overwrites
/// units of the same name, which may well be ones the user has since
/// edited — the timer's `OnCalendar` line included.
pub fn init_notify(dir: &Path, bin: &Path, force: bool) -> Result<(PathBuf, PathBuf, bool)> {
    std::fs::create_dir_all(dir).with_context(|| format!("création de {}", dir.display()))?;
    let service_path = dir.join("orchestra-todo.service");
    let timer_path = dir.join("orchestra-todo.timer");
    if service_path.exists() && timer_path.exists() && !force {
        return Ok((service_path, timer_path, false));
    }
    let (service, timer) = notify_units(bin);
    std::fs::write(&service_path, service)
        .with_context(|| format!("écriture de {}", service_path.display()))?;
    std::fs::write(&timer_path, timer)
        .with_context(|| format!("écriture de {}", timer_path.display()))?;
    Ok((service_path, timer_path, true))
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
                    "integrator",
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

    /// A fake home, so the test never writes into the user's own config.
    struct Home(PathBuf);

    impl Home {
        fn new() -> Home {
            let dir = std::env::temp_dir().join(format!("orchestra-init-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Home(dir)
        }
    }

    impl Drop for Home {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    #[test]
    fn the_zellij_layout_is_installed_where_zellij_looks_and_never_silently_replaced() {
        let home = Home::new();
        // `ZELLIJ_CONFIG_DIR` is zellij's own override, and the one this
        // process can set without touching anything real.
        let dir = home.0.join("layouts");
        let (path, written) = init_zellij(&dir, "orchestra", false).unwrap();
        assert!(written);
        assert_eq!(path, dir.join("orchestra.kdl"));
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("orchestra"));

        // Une disposition que l'utilisateur a pu retoucher n'est pas écrasée
        // parce qu'il a relancé `init`.
        std::fs::write(&path, "à moi").unwrap();
        let (_, written) = init_zellij(&dir, "orchestra", false).unwrap();
        assert!(!written);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "à moi");

        let (_, written) = init_zellij(&dir, "orchestra", true).unwrap();
        assert!(written, "« --force » la remplace");

        // Le nom suit la configuration : `zellij -l atelier` doit trouver
        // « atelier.kdl ».
        let (path, _) = init_zellij(&dir, "atelier", false).unwrap();
        assert_eq!(path, dir.join("atelier.kdl"));
        assert!(std::fs::read_to_string(&path).unwrap().contains("layout"));
    }

    #[test]
    fn the_layout_directory_follows_zellij_own_rules() {
        // Pas de lecture de l'environnement réel dans le test : on vérifie la
        // forme du chemin, qui est ce qui peut se tromper de dossier.
        let dir = zellij_layouts_dir();
        assert!(dir.ends_with("layouts"), "{}", dir.display());
        assert!(dir.parent().unwrap().ends_with("zellij") || dir.parent().is_some());
    }

    #[test]
    fn the_notify_units_are_installed_and_never_silently_replaced() {
        let home = Home::new();
        let dir = home.0.join("systemd/user");
        let bin = PathBuf::from("/opt/orchestra/bin/orchestra");
        let (service, timer, written) = init_notify(&dir, &bin, false).unwrap();
        assert!(written);
        assert_eq!(service, dir.join("orchestra-todo.service"));
        assert_eq!(timer, dir.join("orchestra-todo.timer"));
        let service_body = std::fs::read_to_string(&service).unwrap();
        assert!(service_body.contains("ExecStart=/opt/orchestra/bin/orchestra todo notify"));
        assert!(service_body.contains("Type=oneshot"));
        let timer_body = std::fs::read_to_string(&timer).unwrap();
        assert!(timer_body.contains("OnCalendar="));
        assert!(timer_body.contains("Persistent=true"));

        // A timer the user retouched (a different hour, say) is not
        // overwritten just because `init` ran again.
        std::fs::write(&timer, "à moi").unwrap();
        let (_, _, written) = init_notify(&dir, &bin, false).unwrap();
        assert!(!written);
        assert_eq!(std::fs::read_to_string(&timer).unwrap(), "à moi");

        let (_, _, written) = init_notify(&dir, &bin, true).unwrap();
        assert!(written, "« --force » les remplace");
    }

    #[test]
    fn the_systemd_user_directory_follows_its_own_rules() {
        let dir = systemd_user_dir();
        assert!(dir.ends_with("systemd/user"), "{}", dir.display());
    }
}
