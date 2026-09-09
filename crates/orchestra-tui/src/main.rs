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

use std::io::stdout;

use futures::StreamExt;
use orchestra_core::events::AgentEvent;
use orchestra_core::model::ContextSpace;
use orchestra_core::runtime;
use orchestra_core::session::{Sessions, Tabbed};
use orchestra_core::{docker, git};
use ratatui::crossterm::event::{
    Event, EventStream, KeyCode, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::DefaultTerminal;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;

use crate::app::{App, View};

/// Résultat d'une requête Git asynchrone (état ou diff), livré via un canal `oneshot`
/// unique — évite de multiplier les branches `select!` pour deux requêtes qui ne sont
/// jamais concurrentes dans un même onglet.
enum GitFetch {
    Status(git::GitStatus),
    Diff { path: String, text: String },
}

/// Un **onglet** de travail : l'état complet d'un dashboard (`App`) + les canaux de sa
/// conversation en cours. Chaque onglet garde donc **son propre contexte et son historique** ;
/// basculer d'onglet ne perd rien, et les agents d'un onglet en arrière-plan continuent de
/// travailler (leurs événements s'empilent dans `rx` jusqu'à ce qu'on y revienne).
struct TuiTab {
    app: App,
    /// Canal d'événements du runtime (présent pendant une orchestration / conversation).
    rx: Option<UnboundedReceiver<AgentEvent>>,
    /// Canal d'envoi des messages au coordinateur (présent pendant une conversation).
    chat_tx: Option<UnboundedSender<String>>,
    /// Canal d'approbation de plan (présent pendant orchestration / conversation).
    plan_tx: Option<UnboundedSender<bool>>,
    /// Requête Git en cours (état ou diff), présente entre le lancement et la réponse.
    git_rx: Option<oneshot::Receiver<GitFetch>>,
    /// Requête Docker en cours, présente entre le lancement et la réponse.
    docker_rx: Option<oneshot::Receiver<docker::DockerStatus>>,
}

impl TuiTab {
    /// Nouvel onglet à partir d'un `App`, canaux fermés (aucune conversation en cours).
    fn new(app: App) -> Self {
        Self { app, rx: None, chat_tx: None, plan_tx: None, git_rx: None, docker_rx: None }
    }
}

impl Tabbed for TuiTab {
    fn title(&self) -> String {
        match &self.app.space {
            Some(s) => s.config.project_name.clone(),
            None => "(vide)".to_string(),
        }
    }
    fn root(&self) -> &Path {
        match &self.app.space {
            Some(s) => &s.root,
            None => Path::new(""),
        }
    }
}

/// Mutation d'onglets **différée** : posée pendant le traitement clavier (où l'onglet actif est
/// emprunté), puis appliquée une fois cet emprunt relâché — sinon on emprunterait `Sessions`
/// deux fois. `Open` = ouvrir/activer un onglet pour un `App` fraîchement chargé.
enum SessionCmd {
    Open(Box<App>),
    Next,
    Prev,
    CloseActive,
}

/// Charge un espace depuis un chemin, le **mémorise** dans le registre (récents) et renvoie un
/// `App` neuf prêt à l'afficher. Centralise l'ouverture (saisie de chemin **et** sélecteur).
fn open_space(path: &str) -> Result<App, String> {
    match ContextSpace::load(Path::new(path)) {
        Ok(space) => {
            let _ = orchestra_core::registry::remember_space(Path::new(path));
            let name = space.config.project_name.clone();
            let mut app = App::new(Some(space));
            app.notice = Some(format!("Espace chargé : {name}"));
            Ok(app)
        }
        Err(e) => Err(format!("Échec du chargement : {e}")),
    }
}

/// Crée un nouvel espace (scaffolding via le cœur), le **mémorise** et renvoie un `App` neuf.
fn create_space(root: &Path, opts: orchestra_core::InitOptions) -> Result<App, String> {
    match orchestra_core::scaffold_space(root, opts) {
        Ok(space) => {
            let _ = orchestra_core::registry::remember_space(root);
            let name = space.config.project_name.clone();
            let mut app = App::new(Some(space));
            app.notice = Some(format!("Espace « {name} » créé."));
            Ok(app)
        }
        Err(e) => Err(format!("Échec de la création : {e}")),
    }
}

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
    let loaded = ContextSpace::load(root).ok();
    if loaded.is_some() {
        let _ = orchestra_core::registry::remember_space(root); // mémorise l'espace d'ouverture
    }
    let app = App::new(loaded);
    // Une seule session (onglet) au départ ; l'utilisateur en ouvre d'autres via `[3]`.
    let mut sessions: Sessions<TuiTab> = Sessions::new();
    sessions.open(TuiTab::new(app));

    let mut terminal = ratatui::init();
    // Best-effort : permet de distinguer Maj/Alt+Entrée (terminaux compatibles kitty).
    let _ = execute!(
        stdout(),
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    );
    let result = event_loop(&mut terminal, &mut sessions).await;
    let _ = execute!(stdout(), PopKeyboardEnhancementFlags);
    ratatui::restore();
    result
}

/// Boucle d'affichage : dessine, puis attend le premier des trois flux (clavier / agents
/// / tick). `rx` est le canal du runtime, présent uniquement entre le lancement de
/// l'orchestre et la fin de tous les agents.
async fn event_loop(
    terminal: &mut DefaultTerminal,
    sessions: &mut Sessions<TuiTab>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut input = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(250));

    loop {
        // Barre d'onglets (calculée avant d'emprunter l'onglet actif).
        let titles = sessions.titles();
        let active_tab = sessions.active_index();
        // Onglet actif : l'`App` rendu/piloté + ses canaux. Plus aucun onglet ⇒ on quitte.
        let Some(tab) = sessions.active_mut() else { break };
        let TuiTab { app, rx, chat_tx, plan_tx, git_rx, docker_rx } = tab;
        // Mutation d'onglets à appliquer après relâchement de l'emprunt ci-dessus.
        let mut cmd: Option<SessionCmd> = None;

        terminal.draw(|frame| dashboard::render(frame, app, &titles, active_tab))?;

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
                                    // Sauvegarde via le cœur (l'UI ne touche pas le disque) ;
                                    // la cible dépend de ce que l'éditeur édite (persona / skill).
                                    if let Some(text) = app.editor.as_ref().map(|e| e.to_text()) {
                                        let result = match &app.editor_target {
                                            app::EditTarget::Persona => app
                                                .space
                                                .as_mut()
                                                .map(|space| space.save_persona(&text))
                                                .transpose()
                                                .map(|_| "Persona enregistré."),
                                            app::EditTarget::Document(path) => {
                                                orchestra_core::model::save_document(path, &text)
                                                    .map(|()| "Document enregistré.")
                                            }
                                        };
                                        match result {
                                            Ok(msg) => {
                                                app.editor = None;
                                                app.notice = Some(msg.to_string());
                                            }
                                            Err(e) => {
                                                app.notice = Some(format!("Échec enregistrement : {e}"))
                                            }
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
                                KeyCode::Char('e') => app.edit_current_doc(),
                                _ => {}
                            }
                        } else if app.pending_plan {
                            // Un plan attend l'approbation : Entrée exécute, Échap annule.
                            match key.code {
                                KeyCode::Enter => {
                                    if let Some(tx) = &plan_tx {
                                        let _ = tx.send(true);
                                    }
                                    app.approve_plan();
                                }
                                KeyCode::Esc => {
                                    if let Some(tx) = &plan_tx {
                                        let _ = tx.send(false);
                                    }
                                    app.cancel_plan();
                                }
                                KeyCode::PageUp => app.radar_scroll_by(10),
                                KeyCode::PageDown => app.radar_scroll_by(-10),
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
                        } else if app.view == View::Changes {
                            // Vue Modifications : naviguer entre les fichiers changés.
                            match key.code {
                                KeyCode::Up => app.changes_move(-1),
                                KeyCode::Down => app.changes_move(1),
                                KeyCode::Esc | KeyCode::Char('7') => app.toggle_changes(),
                                _ => {}
                            }
                        } else if app.view == View::Files {
                            // Arborescence du workspace : naviguer + ouvrir un fichier.
                            match key.code {
                                KeyCode::Up => app.files_move(-1),
                                KeyCode::Down => app.files_move(1),
                                KeyCode::Enter => app.open_selected_file(),
                                KeyCode::Esc | KeyCode::Char('6') => app.toggle_files(),
                                _ => {}
                            }
                        } else if app.view == View::Git {
                            // État Git : naviguer les fichiers, Entrée = diff, [r] = rafraîchir.
                            match key.code {
                                KeyCode::Up => app.git_move(-1),
                                KeyCode::Down => app.git_move(1),
                                KeyCode::Enter => {
                                    if let Some(path) = app.git_selected_path() {
                                        if let Some(space) = &app.space {
                                            *git_rx = Some(spawn_git_diff(space.workspace(), path));
                                        }
                                    }
                                }
                                KeyCode::Char('r') => {
                                    if let Some(space) = &app.space {
                                        *git_rx = Some(spawn_git_status(space.workspace()));
                                    }
                                }
                                KeyCode::Esc | KeyCode::Char('8') => app.toggle_git(),
                                _ => {}
                            }
                        } else if app.view == View::Docker {
                            // État Docker : [r] rafraîchir, Échap retour.
                            match key.code {
                                KeyCode::Char('r') => {
                                    if let Some(space) = &app.space {
                                        *docker_rx = Some(spawn_docker_status(space.workspace()));
                                    }
                                }
                                KeyCode::Esc | KeyCode::Char('9') => app.toggle_docker(),
                                _ => {}
                            }
                        } else if app.view == View::Terminal {
                            match key.code {
                                KeyCode::Esc | KeyCode::Char('0') => app.toggle_terminal(),
                                _ => {}
                            }
                        } else if app.view == View::Memory {
                            match key.code {
                                KeyCode::Up => app.memory_scroll_by(-1),
                                KeyCode::Down => app.memory_scroll_by(1),
                                KeyCode::Esc | KeyCode::Char('m') => app.toggle_memory(),
                                _ => {}
                            }
                        } else if app.view == View::Context {
                            match key.code {
                                KeyCode::Esc | KeyCode::Char('c') => app.toggle_context(),
                                _ => {}
                            }
                        } else if app.view == View::Spaces && app.new_space.is_some() {
                            // Formulaire de création d'un nouvel espace.
                            use app::NewField;
                            let field = app.new_space.as_ref().map(|f| f.field);
                            match key.code {
                                KeyCode::Esc => app.cancel_new_space(),
                                KeyCode::Up => app.new_space_focus(-1),
                                KeyCode::Down | KeyCode::Tab => app.new_space_focus(1),
                                KeyCode::Enter => {
                                    if field == Some(NewField::Create) {
                                        match app.new_space_build() {
                                            Some((root, opts)) => match create_space(&root, opts) {
                                                // La nouvelle session s'ouvre dans un onglet dédié.
                                                Ok(new_app) => cmd = Some(SessionCmd::Open(Box::new(new_app))),
                                                Err(msg) => app.notice = Some(msg),
                                            },
                                            None => app.notice = Some("Le nom du projet est obligatoire.".into()),
                                        }
                                    } else {
                                        app.new_space_focus(1);
                                    }
                                }
                                KeyCode::Backspace => app.new_space_backspace(),
                                KeyCode::Char(c) => app.new_space_push(c),
                                _ => {}
                            }
                        } else if app.view == View::Spaces && app.browse.is_some() {
                            // Navigateur de dossiers : explorer et ouvrir un espace repéré.
                            match key.code {
                                KeyCode::Up => app.browse_move(-1),
                                KeyCode::Down => app.browse_move(1),
                                KeyCode::Enter => {
                                    if let Some(path) = app.browse_enter() {
                                        match open_space(&path) {
                                            Ok(new_app) => cmd = Some(SessionCmd::Open(Box::new(new_app))),
                                            Err(msg) => app.notice = Some(msg),
                                        }
                                    }
                                }
                                KeyCode::Left | KeyCode::Char('u') => app.browse_up(),
                                // [r] : reprendre le dossier sélectionné comme projet Dev existant.
                                KeyCode::Char('r') => {
                                    if let Some(path) = app.selected_browse_path() {
                                        match orchestra_core::scaffold::adopt_project(Path::new(&path)) {
                                            Ok(_) => match open_space(&path) {
                                                Ok(new_app) => cmd = Some(SessionCmd::Open(Box::new(new_app))),
                                                Err(msg) => app.notice = Some(msg),
                                            },
                                            Err(e) => app.notice = Some(format!("Reprise impossible : {e}")),
                                        }
                                    }
                                }
                                KeyCode::Esc => app.cancel_browse(),
                                _ => {}
                            }
                        } else if app.view == View::Spaces && app.input.is_none() {
                            // Sélecteur d'espaces connus : navigation + ouverture + suivi.
                            match key.code {
                                KeyCode::Up => app.spaces_move(-1),
                                KeyCode::Down => app.spaces_move(1),
                                KeyCode::Enter => {
                                    if let Some(path) = app.selected_space_path() {
                                        match open_space(&path) {
                                            // Ouvre la session dans un onglet (ou réactive le sien).
                                            Ok(new_app) => cmd = Some(SessionCmd::Open(Box::new(new_app))),
                                            Err(msg) => app.notice = Some(msg),
                                        }
                                    }
                                }
                                KeyCode::Char('n') => app.start_new_space(),
                                KeyCode::Char('b') => app.start_browse(),
                                KeyCode::Char('a') => app.start_space_input(),
                                KeyCode::Char('x') => app.forget_selected_space(),
                                KeyCode::Esc | KeyCode::Char('3') => app.toggle_spaces(),
                                _ => {}
                            }
                        } else if app.chat.is_some() {
                            // Conversation : saisie d'un message + envoi au coordinateur.
                            match key.code {
                                KeyCode::Esc => {
                                    app.end_chat();
                                    *chat_tx = None; // ferme le canal → termine la conversation
                                    *plan_tx = None;
                                }
                                // Maj/Alt+Entrée : nouvelle ligne ; Entrée seul : envoyer.
                                KeyCode::Enter
                                    if key.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
                                {
                                    app.chat_push('\n');
                                }
                                KeyCode::Enter => {
                                    if let Some(msg) = app.chat_submit() {
                                        if let Some(tx) = chat_tx.as_ref() {
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
                        } else if app.intention.is_some() {
                            // Saisie d'une intention : exécution one-shot par le coordinateur.
                            match key.code {
                                KeyCode::Esc => app.cancel_intention(),
                                KeyCode::Enter => {
                                    if let Some(goal) = app.take_intention() {
                                        app.notice = None;
                                        app.begin_run();
                                        // Orchestration réelle : plan → approbation → exécution.
                                        let handle =
                                            runtime::orchestrate(app.space.as_ref().unwrap(), &goal);
                                        *plan_tx = Some(handle.approve);
                                        *rx = Some(handle.events);
                                    }
                                }
                                KeyCode::Backspace => app.intention_backspace(),
                                KeyCode::Char(c) => app.intention_push(c),
                                _ => {}
                            }
                        } else if app.input.is_some() {
                            // Mode saisie d'un chemin d'espace : les touches alimentent le tampon.
                            match key.code {
                                KeyCode::Esc => app.cancel_input(),
                                KeyCode::Backspace => app.input_backspace(),
                                KeyCode::Enter => {
                                    if let Some(path) = app.take_input() {
                                        match open_space(&path) {
                                            Ok(new_app) => cmd = Some(SessionCmd::Open(Box::new(new_app))),
                                            Err(msg) => app.notice = Some(msg),
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
                                            "⚠ Persona incomplet (« à compléter ») — édite-le ([4]) puis relance [1]."
                                                .to_string(),
                                        );
                                    } else {
                                        app.start_intention(); // saisie de l'objectif à exécuter
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
                                        *rx = Some(handle.events);
                                        *chat_tx = Some(handle.user);
                                        *plan_tx = Some(handle.approve); // approbation des plans proposés en chat
                                    }
                                }
                                KeyCode::Char('2') => app.toggle_docs(),
                                KeyCode::Char('3') => app.toggle_spaces(),
                                KeyCode::Char('4') => app.open_persona_editor(),
                                KeyCode::Char('6') if app.space.is_some() => app.toggle_files(),
                                KeyCode::Char('7') if app.space.is_some() => app.toggle_changes(),
                                KeyCode::Char('8') if app.space.is_some() => {
                                    app.toggle_git();
                                    if app.view == View::Git {
                                        *git_rx = Some(spawn_git_status(app.space.as_ref().unwrap().workspace()));
                                    }
                                }
                                KeyCode::Char('9') if app.space.is_some() => {
                                    app.toggle_docker();
                                    if app.view == View::Docker {
                                        *docker_rx =
                                            Some(spawn_docker_status(app.space.as_ref().unwrap().workspace()));
                                    }
                                }
                                KeyCode::Char('0') if app.space.is_some() => app.toggle_terminal(),
                                KeyCode::Char('m') if app.space.is_some() => app.toggle_memory(),
                                KeyCode::Char('c') if app.space.is_some() => app.toggle_context(),
                                // Onglets (sessions) : Tab/Maj+Tab pour circuler, Ctrl+W pour fermer.
                                KeyCode::Tab => cmd = Some(SessionCmd::Next),
                                KeyCode::BackTab => cmd = Some(SessionCmd::Prev),
                                KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) && titles.len() > 1 => {
                                    cmd = Some(SessionCmd::CloseActive);
                                }
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
                        *rx = None;
                    }
                }
            }
            _ = tick.tick() => { app.tick(); }        // rafraîchissement + animation spinner

            res = recv_oneshot(git_rx.as_mut()) => {
                if let Some(fetch) = res {
                    match fetch {
                        GitFetch::Status(status) => app.set_git_status(status),
                        GitFetch::Diff { path, text } => app.set_git_diff(path, text),
                    }
                }
                *git_rx = None;
            }
            res = recv_oneshot(docker_rx.as_mut()) => {
                if let Some(status) = res {
                    app.set_docker_status(status);
                }
                *docker_rx = None;
            }
        }

        // L'emprunt de l'onglet actif est relâché : on peut muter la collection d'onglets.
        match cmd {
            Some(SessionCmd::Open(app)) => { sessions.open(TuiTab::new(*app)); }
            Some(SessionCmd::Next) => sessions.next(),
            Some(SessionCmd::Prev) => sessions.prev(),
            Some(SessionCmd::CloseActive) => sessions.close_active(),
            None => {}
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

/// Attend la réponse d'une requête ponctuelle (Git/Docker) si une est en cours ; sinon ne
/// se résout jamais. Symétrique de [`recv_optional`] pour un canal `oneshot`.
async fn recv_oneshot<T>(rx: Option<&mut oneshot::Receiver<T>>) -> Option<T> {
    match rx {
        Some(rx) => rx.await.ok(),
        None => std::future::pending().await,
    }
}

/// Lance la récupération de l'état Git en arrière-plan et retourne le récepteur à stocker
/// dans l'onglet.
fn spawn_git_status(root: std::path::PathBuf) -> oneshot::Receiver<GitFetch> {
    let (tx, rxo) = oneshot::channel();
    tokio::spawn(async move {
        let _ = tx.send(GitFetch::Status(git::status(&root).await));
    });
    rxo
}

/// Lance la récupération du diff d'un fichier en arrière-plan.
fn spawn_git_diff(root: std::path::PathBuf, path: String) -> oneshot::Receiver<GitFetch> {
    let (tx, rxo) = oneshot::channel();
    tokio::spawn(async move {
        let text = git::diff(&root, Some(&path)).await;
        let _ = tx.send(GitFetch::Diff { path, text });
    });
    rxo
}

/// Lance la détection de l'état Docker en arrière-plan.
fn spawn_docker_status(root: std::path::PathBuf) -> oneshot::Receiver<docker::DockerStatus> {
    let (tx, rxo) = oneshot::channel();
    tokio::spawn(async move {
        let _ = tx.send(docker::detect(&root).await);
    });
    rxo
}
