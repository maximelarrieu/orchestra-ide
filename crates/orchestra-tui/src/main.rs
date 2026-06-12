//! Frontend terminal d'Orchestra IDE.
//!
//! Deux modes :
//! - `orchestra init [chemin]` → assistant de scaffolding (Phase 2, voir [`wizard`]) ;
//! - `orchestra [chemin]`      → tableau de bord TUI (radar vivant depuis la Phase 3).
//!
//! La boucle du dashboard est asynchrone : elle multiplexe (`tokio::select!`) l'entrée
//! clavier, le flux d'événements des agents et un tick de rafraîchissement.

mod app;
mod dashboard;
mod editor;
mod markdown;
mod wizard;

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::StreamExt;
use orchestra_core::events::AgentEvent;
use orchestra_core::model::ContextSpace;
use orchestra_core::runtime;
use ratatui::crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::app::{App, View};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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
            run_dashboard(&root).await
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

async fn run_dashboard(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // On tolère l'absence d'espace : le dashboard s'affiche quand même (état « vide »).
    let mut app = App::new(ContextSpace::load(root).ok());

    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app).await;
    ratatui::restore();
    result
}

/// Boucle d'affichage : dessine, puis attend le premier des trois flux (clavier / agents
/// / tick). `rx` est le canal du runtime, présent uniquement entre le lancement de
/// l'orchestre et la fin de tous les agents.
async fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut input = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let mut rx: Option<UnboundedReceiver<AgentEvent>> = None;
    // `Some` pendant une conversation : canal pour envoyer les messages au coordinateur.
    let mut chat_tx: Option<UnboundedSender<String>> = None;

    loop {
        terminal.draw(|frame| dashboard::render(frame, app))?;

        tokio::select! {
            maybe_input = input.next() => {
                match maybe_input {
                    Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                        if app.editor.is_some() {
                            // Mode édition du persona : les touches alimentent l'éditeur.
                            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                            match key.code {
                                KeyCode::Esc => {
                                    app.editor = None;
                                    app.notice = Some("Édition annulée.".to_string());
                                }
                                KeyCode::Char('s') if ctrl => {
                                    // Sauvegarde via le cœur (l'UI ne touche pas le disque).
                                    if let Some(text) = app.editor.as_ref().map(|e| e.to_text()) {
                                        match app.space.as_mut() {
                                            Some(space) => match space.save_persona(&text) {
                                                Ok(()) => {
                                                    app.editor = None;
                                                    app.notice = Some("Persona enregistré.".to_string());
                                                }
                                                Err(e) => {
                                                    app.notice = Some(format!("Échec enregistrement : {e}"))
                                                }
                                            },
                                            None => app.editor = None,
                                        }
                                    }
                                }
                                KeyCode::Enter => {
                                    if let Some(ed) = app.editor.as_mut() { ed.newline() }
                                }
                                KeyCode::Backspace => {
                                    if let Some(ed) = app.editor.as_mut() { ed.backspace() }
                                }
                                KeyCode::Left => { if let Some(ed) = app.editor.as_mut() { ed.left() } }
                                KeyCode::Right => { if let Some(ed) = app.editor.as_mut() { ed.right() } }
                                KeyCode::Up => { if let Some(ed) = app.editor.as_mut() { ed.up() } }
                                KeyCode::Down => { if let Some(ed) = app.editor.as_mut() { ed.down() } }
                                KeyCode::Home => { if let Some(ed) = app.editor.as_mut() { ed.home() } }
                                KeyCode::End => { if let Some(ed) = app.editor.as_mut() { ed.end() } }
                                KeyCode::Char(c) if !ctrl => {
                                    if let Some(ed) = app.editor.as_mut() { ed.insert_char(c) }
                                }
                                _ => {}
                            }
                        } else if app.viewer.is_some() {
                            // Visualiseur Markdown : défilement + fermeture (+ édition persona).
                            match key.code {
                                KeyCode::Esc => app.close_viewer(),
                                KeyCode::Up => app.viewer_scroll(-1),
                                KeyCode::Down => app.viewer_scroll(1),
                                KeyCode::PageUp => app.viewer_scroll(-10),
                                KeyCode::PageDown => app.viewer_scroll(10),
                                KeyCode::Char('e') if app.viewer_is_persona() => {
                                    app.close_viewer();
                                    app.open_persona_editor();
                                }
                                _ => {}
                            }
                        } else if app.view == View::Docs {
                            // Navigateur de documents : sélection + ouverture.
                            match key.code {
                                KeyCode::Up => app.docs_move(-1),
                                KeyCode::Down => app.docs_move(1),
                                KeyCode::Enter => app.open_selected_doc(),
                                KeyCode::Esc | KeyCode::Char('2') => app.toggle_docs(),
                                _ => {}
                            }
                        } else if app.chat.is_some() {
                            // Conversation : saisie d'un message + envoi au coordinateur.
                            match key.code {
                                KeyCode::Esc => {
                                    app.end_chat();
                                    chat_tx = None; // ferme le canal → termine la conversation
                                }
                                KeyCode::Enter => {
                                    if let Some(msg) = app.chat_submit() {
                                        if let Some(tx) = &chat_tx {
                                            let _ = tx.send(msg);
                                        }
                                    }
                                }
                                KeyCode::PageUp => app.radar_scroll_by(10),
                                KeyCode::PageDown => app.radar_scroll_by(-10),
                                KeyCode::Up => app.radar_scroll_by(3),
                                KeyCode::Down => app.radar_scroll_by(-3),
                                KeyCode::Backspace => app.chat_backspace(),
                                KeyCode::Char(c) => app.chat_push(c),
                                _ => {}
                            }
                        } else if app.input.is_some() {
                            // Mode saisie d'un chemin d'espace : les touches alimentent le tampon.
                            match key.code {
                                KeyCode::Esc => app.cancel_input(),
                                KeyCode::Backspace => app.input_backspace(),
                                KeyCode::Enter => {
                                    if let Some(path) = app.take_input() {
                                        match ContextSpace::load(Path::new(&path)) {
                                            Ok(space) => {
                                                let name = space.config.project_name.clone();
                                                *app = App::new(Some(space));
                                                app.notice = Some(format!("Espace chargé : {name}"));
                                                rx = None; // stoppe l'orchestre précédent
                                            }
                                            Err(e) => app.notice = Some(format!("Échec du chargement : {e}")),
                                        }
                                    }
                                }
                                KeyCode::Char(c) => app.input_push(c),
                                _ => {}
                            }
                        } else {
                            match key.code {
                                KeyCode::Char('q') | KeyCode::Esc => break,
                                KeyCode::Char('1') if app.can_launch() => {
                                    if app.persona_incomplete() && app.llm_model.is_some() {
                                        // Évite un appel LLM voué à l'échec faute de contexte.
                                        app.notice = Some(
                                            "⚠ Persona incomplet (« à compléter ») — édite .orchestra/persona.md puis relance [1]."
                                                .to_string(),
                                        );
                                    } else {
                                        app.notice = None;
                                        app.begin_run();
                                        rx = Some(runtime::spawn(app.space.as_ref().unwrap()));
                                    }
                                }
                                KeyCode::Char('5') if app.space.is_some() => {
                                    if app.persona_incomplete() && app.llm_model.is_some() {
                                        app.notice = Some(
                                            "⚠ Persona incomplet (« à compléter ») — édite-le ([4]) puis relance [5]."
                                                .to_string(),
                                        );
                                    } else {
                                        app.notice = None;
                                        app.start_chat();
                                        let handle = runtime::start_conversation(app.space.as_ref().unwrap());
                                        rx = Some(handle.events);
                                        chat_tx = Some(handle.user);
                                    }
                                }
                                KeyCode::Char('2') => app.toggle_docs(),
                                KeyCode::Char('3') => app.start_space_input(),
                                KeyCode::Char('4') => app.open_persona_editor(),
                                KeyCode::PageUp => app.radar_scroll_by(10),
                                KeyCode::PageDown => app.radar_scroll_by(-10),
                                KeyCode::Up => app.radar_scroll_by(3),
                                KeyCode::Down => app.radar_scroll_by(-3),
                                _ => {}
                            }
                        }
                    }
                    Some(Ok(_)) => {}                 // resize & co. : redraw au prochain tour
                    Some(Err(e)) => return Err(e.into()),
                    None => break,                    // stdin fermé
                }
            }
            ev = recv_optional(rx.as_mut()) => {
                match ev {
                    Some(ev) => app.on_event(ev),
                    None => {
                        // Canal fermé : tous les agents ont terminé.
                        app.mark_finished();
                        rx = None;
                    }
                }
            }
            _ = tick.tick() => {}                     // rafraîchissement périodique
        }
    }
    Ok(())
}

/// Attend le prochain événement du runtime si un canal est ouvert ; sinon ne se résout
/// jamais (branche `select!` neutralisée tant que l'orchestre n'est pas lancé).
async fn recv_optional(rx: Option<&mut UnboundedReceiver<AgentEvent>>) -> Option<AgentEvent> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}
