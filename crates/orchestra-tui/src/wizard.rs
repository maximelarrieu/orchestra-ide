//! Assistant interactif de `orchestra init`.
//!
//! Pur I/O terminal (stdin/stdout) : collecte les choix de l'utilisateur, les empaquette
//! dans un [`InitOptions`] et confie l'écriture au cœur ([`orchestra_core::scaffold`]).
//! Aucune logique de scaffolding ici — c'est la frontière métier/affichage de la spec.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use orchestra_core::model::ProjectType;
use orchestra_core::{scaffold_space, InitOptions};

/// Point d'entrée de la sous-commande `init`. `target` = répertoire où créer l'espace.
pub fn run(target: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("┌─ ORCHESTRA INIT ─ création d'un Espace de Contexte");
    println!("│  Cible : {}", target.display());
    println!("└─ (Entrée pour accepter la valeur entre crochets)\n");

    let default_name = default_project_name(target);
    let project_name = prompt_line("Nom du projet", Some(&default_name))?;
    let project_type = prompt_project_type()?;

    // Le chemin du code n'a de sens que pour les projets « Dev ».
    let workspace_path = if project_type == ProjectType::Dev {
        let raw = prompt_line("Chemin du code à piloter (workspace)", Some("."))?;
        Some(PathBuf::from(raw))
    } else {
        None
    };

    let documentalist_enabled = prompt_yes_no("Activer l'Agent Documentaliste ?", false)?;

    let opts = InitOptions {
        project_name,
        project_type,
        workspace_path,
        documentalist_enabled,
    };

    let space = scaffold_space(target, opts)?;

    println!("\n✓ Espace « {} » créé.", space.config.project_name);
    println!("  Type        : {}", space.config.project_type.label());
    println!("  Agents      : {}", space.config.agents.join(", "));
    println!("  Skills      : {}", space.config.skills.join(", "));
    println!(
        "  Documentaliste : {}",
        if space.config.documentalist_enabled { "oui" } else { "non" }
    );
    println!("\n  Fichiers : .orchestra/{{config.json, persona.md, adr/}}");
    println!("  → Complète .orchestra/persona.md, puis ouvre l'espace :");
    println!("      cargo run -p orchestra-tui -- {}", target.display());

    Ok(())
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

/// Menu numéroté des quatre types de projet. Boucle jusqu'à un choix valide.
fn prompt_project_type() -> io::Result<ProjectType> {
    const CHOICES: [(&str, ProjectType); 4] = [
        ("Dev", ProjectType::Dev),
        ("Nutrition", ProjectType::Nutrition),
        ("Langue", ProjectType::Langue),
        ("Immobilier", ProjectType::Immobilier),
    ];

    println!("Type de projet :");
    for (i, (label, _)) in CHOICES.iter().enumerate() {
        println!("  [{}] {label}", i + 1);
    }

    loop {
        let raw = prompt_line("Choix", Some("1"))?;
        match raw.parse::<usize>() {
            Ok(n) if (1..=CHOICES.len()).contains(&n) => return Ok(CHOICES[n - 1].1),
            _ => println!("  ✗ Entre un nombre entre 1 et {}.", CHOICES.len()),
        }
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
