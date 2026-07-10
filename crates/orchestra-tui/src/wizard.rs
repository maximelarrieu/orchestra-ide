//! Assistant interactif de `orchestra init`.
//!
//! Pur I/O terminal (stdin/stdout) : collecte les choix de l'utilisateur, les empaquette
//! dans un [`InitOptions`] et confie l'écriture au cœur ([`orchestra_core::scaffold`]).
//! Aucune logique de scaffolding ici — c'est la frontière métier/affichage de la spec.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use orchestra_core::model::config::{GitIntegration, GithubIntegration, Integrations};
use orchestra_core::{scaffold_space, InitOptions};

/// Point d'entrée de la sous-commande `init`. `target` = répertoire où créer l'espace.
pub fn run(target: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("┌─ ORCHESTRA INIT ─ création d'un Espace de Contexte");
    println!("│  Cible : {}", target.display());
    println!("└─ (Entrée pour accepter la valeur entre crochets)\n");

    let default_name = default_project_name(target);
    let project_name = prompt_line("Nom du projet", Some(&default_name))?;

    let raw = prompt_line("Chemin du code à piloter (workspace, vide si aucun)", Some("."))?;
    let workspace_path = if raw.trim().is_empty() {
        None
    } else {
        Some(absolutize(&raw)) // chemin absolu → robuste quel que soit le cwd au lancement
    };

    let integrations = prompt_integrations()?;

    let opts = InitOptions {
        project_name,
        workspace_path,
        integrations,
        objectives: String::new(),
    };

    let space = scaffold_space(target, opts)?;

    println!("\n✓ Espace « {} » créé.", space.config.project_name);
    if let Some(ws) = &space.config.workspace_path {
        println!("  Workspace   : {}", ws.display());
    }
    let integ = &space.config.integrations;
    if integ.git.is_some() || integ.github.is_some() {
        let mut parts = Vec::new();
        if integ.git.is_some() {
            parts.push("Git".to_string());
        }
        if let Some(gh) = &integ.github {
            parts.push(format!("GitHub ({})", gh.repo));
        }
        println!("  Intégrations : {}", parts.join(", "));
    }
    println!("\n  Fichiers : .orchestra/{{config.json, persona.md, adr/}}");
    println!("  → Complète .orchestra/persona.md (surtout « ## Objectifs »), puis ouvre l'espace :");
    println!("      cargo run -p orchestra-tui -- {}", target.display());

    Ok(())
}

/// Résout un chemin (relatif ou `.`) en chemin absolu, sans exiger qu'il existe.
fn absolutize(raw: &str) -> PathBuf {
    let p = PathBuf::from(raw);
    if let Ok(canon) = std::fs::canonicalize(&p) {
        return canon;
    }
    if p.is_absolute() {
        p
    } else {
        std::env::current_dir().map(|c| c.join(&p)).unwrap_or(p)
    }
}

/// Propose de configurer les intégrations Git/GitHub. Tokens jamais saisis
/// ici : seul le *nom* de la variable d'environnement est enregistré.
fn prompt_integrations() -> io::Result<Integrations> {
    let mut integ = Integrations::default();

    if prompt_yes_no("Activer l'intégration Git ?", false)? {
        let main_branch = prompt_line("  Branche principale", Some("main"))?;
        let auto_branching = prompt_yes_no("  Créer une branche par intention (auto-branching) ?", false)?;
        integ.git = Some(GitIntegration { auto_branching, main_branch });
    }

    if prompt_yes_no("Activer l'intégration GitHub ?", false)? {
        let repo = prompt_line("  Dépôt GitHub (owner/repo)", None)?;
        if repo.is_empty() {
            println!("  ✗ Dépôt vide — intégration GitHub ignorée.");
        } else {
            let token_env_var = prompt_line("  Variable d'env du token", Some("GITHUB_TOKEN"))?;
            integ.github = Some(GithubIntegration { repo, token_env_var });
        }
    }

    Ok(integ)
}

/// Nom par défaut déduit du dossier cible (« . » → dossier courant résolu).
fn default_project_name(target: &Path) -> String {
    let resolved = if target == Path::new(".") {
        std::env::current_dir().unwrap_or_else(|_| target.to_path_buf())
    } else {
        target.to_path_buf()
    };
    resolved
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("mon-espace")
        .to_string()
}

/// Affiche une invite et lit une ligne. Renvoie `default` si la saisie est vide.
fn prompt_line(label: &str, default: Option<&str>) -> io::Result<String> {
    match default {
        Some(d) => print!("{label} [{d}] : "),
        None => print!("{label} : "),
    }
    io::stdout().flush()?;

    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    let trimmed = buf.trim();

    if trimmed.is_empty() {
        Ok(default.unwrap_or("").to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

/// Question oui/non avec valeur par défaut. Accepte o/oui/y/yes et n/non/no.
fn prompt_yes_no(label: &str, default: bool) -> io::Result<bool> {
    let hint = if default { "O/n" } else { "o/N" };
    loop {
        print!("{label} [{hint}] : ");
        io::stdout().flush()?;

        let mut buf = String::new();
        io::stdin().read_line(&mut buf)?;
        match buf.trim().to_lowercase().as_str() {
            "" => return Ok(default),
            "o" | "oui" | "y" | "yes" => return Ok(true),
            "n" | "non" | "no" => return Ok(false),
            _ => println!("  ✗ Réponds par o (oui) ou n (non)."),
        }
    }
}
