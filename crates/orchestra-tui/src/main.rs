//! Frontend terminal d'Orchestra IDE.
//!
//! Deux modes :
//! - `orchestra init [chemin]` → assistant de scaffolding (Phase 2, voir [`wizard`]) ;
//! - `orchestra [chemin]`      → tableau de bord TUI (Phase 1, voir [`dashboard`]).
//!
//! Le dashboard reste une coquille (radar vide) tant que le runtime d'agents (Phase 3)
//! n'est pas branché.

mod dashboard;
mod wizard;

use std::path::{Path, PathBuf};

use orchestra_core::model::ContextSpace;
use ratatui::crossterm::event::{self, Event, KeyCode};
use ratatui::DefaultTerminal;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(String::as_str) {
        Some("init") => {
            // `orchestra init [chemin]` — cible = 2e argument, sinon répertoire courant.
            let target = args.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            if let Err(e) = wizard::run(&target) {
                // Message lisible (Display) plutôt que le Debug brut de la chaîne d'erreurs.
                eprintln!("\n✗ {e}");
                std::process::exit(1);
            }
            Ok(())
        }
        Some("-h") | Some("--help") => {
            print_usage();
            Ok(())
        }
        _ => {
            // Mode dashboard : 1er argument = espace à ouvrir, sinon répertoire courant.
            let root = args.first().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            run_dashboard(&root)
        }
    }
}

fn print_usage() {
    println!("Orchestra IDE v{}\n", env!("CARGO_PKG_VERSION"));
    println!("Usage :");
    println!("  orchestra init [chemin]   Crée un Espace de Contexte (assistant interactif)");
    println!("  orchestra [chemin]        Ouvre le tableau de bord sur un Espace");
    println!("  orchestra --help          Affiche cette aide");
}

fn run_dashboard(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // On tolère l'absence d'espace : le dashboard s'affiche quand même (état « vide »).
    let space = ContextSpace::load(root).ok();

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, space);
    ratatui::restore();
    result
}

fn run(
    terminal: &mut DefaultTerminal,
    space: Option<ContextSpace>,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        terminal.draw(|frame| dashboard::render(frame, space.as_ref()))?;

        if let Event::Key(key) = event::read()? {
            if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
                break;
            }
        }
    }
    Ok(())
}
