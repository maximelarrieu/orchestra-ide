//! What the keys do, said in one place.
//!
//! The bottom bar and the help overlay read the same list, so a command the
//! screen does not offer is never written anywhere. Two ranks only: what this
//! screen alone can do, up front, and what works everywhere, faded behind it.
//! The key sits in brackets with the action beside it, so the eye never has to
//! work out where one ends and the other begins.

use ratatui::text::Span;

use crate::app::{App, BoardPane, Screen};
use crate::theme;

/// Where a key comes from: this screen alone, or everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// What this screen alone knows how to do.
    Screen,
    /// Moving about and getting out, the same on every screen.
    Global,
}

/// A key and what it sets off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    pub key: String,
    pub label: String,
    pub scope: Scope,
}

impl Hint {
    pub fn screen(key: impl Into<String>, label: impl Into<String>) -> Self {
        Hint {
            key: key.into(),
            label: label.into(),
            scope: Scope::Screen,
        }
    }

    pub fn global(key: impl Into<String>, label: impl Into<String>) -> Self {
        Hint {
            key: key.into(),
            label: label.into(),
            scope: Scope::Global,
        }
    }

    /// How many columns `[key] action` takes.
    pub fn width(&self) -> usize {
        self.key.chars().count() + self.label.chars().count() + 3
    }

    /// `[p] planifier`, ready to drop on a line.
    pub fn spans(&self) -> Vec<Span<'static>> {
        let (key_style, label_style) = match self.scope {
            Scope::Screen => (theme::key(), theme::key_label()),
            Scope::Global => (theme::key_dim(), theme::key_dim()),
        };
        vec![
            Span::styled("[", theme::bracket()),
            Span::styled(self.key.clone(), key_style),
            Span::styled("] ", theme::bracket()),
            Span::styled(self.label.clone(), label_style),
        ]
    }
}

/// What goes between two keys: nothing, a gap, or the bar that marks the step
/// from this screen's keys to the ones that work everywhere.
pub fn separator(previous: Option<Scope>, next: Scope) -> &'static str {
    match previous {
        None => "",
        Some(scope) if scope == next => "  ",
        Some(_) => "  │  ",
    }
}

/// How many columns this bar would take.
pub fn line_width(hints: &[Hint]) -> usize {
    let mut width = 0;
    let mut previous = None;
    for hint in hints {
        width += separator(previous, hint.scope).chars().count() + hint.width();
        previous = Some(hint.scope);
    }
    width
}

/// The bar cut to the width there is.
///
/// Four shapes, fullest to leanest: everything; the common keys down to help
/// and the way out; help alone; then the screen's own keys, which the renderer
/// will trim. What goes first is exactly what the help repeats — a command
/// belonging to one screen is written nowhere else — and `?` goes last, since
/// it is the one that brings all the rest back.
pub fn strip_for_width(app: &App, width: usize) -> Vec<Hint> {
    let full = strip(app);
    if line_width(&full) <= width {
        return full;
    }
    let screen = screen_hints(app);
    // A text field has no common rank to give up: its bar is already as
    // short as it goes.
    if screen.is_empty() || full.iter().all(|h| h.scope == Scope::Screen) {
        return full;
    }
    let mut short = screen.clone();
    // What waits on the user survives the trim: it is the key that says
    // where to go next, and the badge at the top points to it.
    let waiting = app.attention_queue().len();
    if waiting > 0 {
        short.push(Hint::global("!", format!("à toi ({waiting})")));
    }
    short.push(Hint::global("?", "aide"));
    let mut with_exit = short.clone();
    with_exit.push(Hint::global(
        "q",
        if app.screen == Screen::Board {
            "quitter"
        } else {
            "retour"
        },
    ));
    if line_width(&with_exit) <= width {
        return with_exit;
    }
    if line_width(&short) <= width {
        return short;
    }
    // Even when nothing else fits, `?` stays: the renderer holds its place
    // and trims what comes before it.
    short
}

/// The bottom bar: the screen first, the rest after.
///
/// An open text field swallows the ordinary keys — promising « ? aide » while
/// someone writes a brief would be a lie, since the `?` lands in the field.
pub fn strip(app: &App) -> Vec<Hint> {
    if app.confirm.is_some() {
        return vec![
            Hint::screen("y", "confirmer"),
            Hint::global("Échap", "renoncer"),
        ];
    }
    if app.show_help {
        return vec![Hint::global("Échap", "fermer l'aide")];
    }
    if app.diff_view.is_some() {
        return vec![
            Hint::screen("j/k", "défiler"),
            Hint::screen("PgUp/PgDn", "page"),
            Hint::screen("g/G", "début / fin"),
            Hint::global("q", "fermer le diff"),
        ];
    }
    if app.palette.is_some() {
        return vec![
            Hint::screen("Entrée", "exécuter"),
            Hint::global("Échap", "annuler"),
        ];
    }
    if let Some(hints) = typing_hints(app) {
        return hints;
    }
    let mut hints = screen_hints(app);
    hints.extend(global_hints(app));
    hints
}

/// The keys of a text field, when one has focus.
fn typing_hints(app: &App) -> Option<Vec<Hint>> {
    if app.screen == Screen::NewTicket {
        return Some(vec![
            Hint::screen("Ctrl-S", "créer le ticket"),
            Hint::screen("Tab", "champ suivant"),
            Hint::global("Entrée", "nouvelle ligne"),
            Hint::global("Échap", "annuler"),
        ]);
    }
    if app.steer.is_some() {
        let what = if app.steer_hard {
            "rediriger tout de suite"
        } else {
            "envoyer après ce tour"
        };
        return Some(vec![
            Hint::screen("Ctrl-S", what),
            Hint::global("Échap", "annuler"),
        ]);
    }
    if app.screen == Screen::Proposal && app.editor.is_editing() {
        if matches!(app.editor.mode, crate::forms::EditorMode::Acceptance { .. }) {
            return Some(vec![
                Hint::screen("Ctrl-S", "garder les critères"),
                Hint::screen("Entrée", "critère suivant"),
                Hint::global("Échap", "annuler"),
            ]);
        }
        return Some(vec![
            Hint::screen("Ctrl-S", "garder l'objectif"),
            Hint::global("Échap", "annuler"),
        ]);
    }
    None
}

/// What this screen, and it alone, can do right now.
pub fn screen_hints(app: &App) -> Vec<Hint> {
    match app.screen {
        Screen::Board => board_hints(app),
        Screen::Ticket => ticket_hints(app),
        Screen::Agent => agent_hints(app),
        Screen::Cost => cost_hints(app),
        Screen::Proposal => proposal_hints(app),
        Screen::Todo => todo_hints(app),
        Screen::Rules => rules_hints(app),
        Screen::NewTicket => {
            let verb = if app.promoting_todo.is_some() {
                "promouvoir en ticket"
            } else {
                "créer le ticket"
            };
            vec![
                Hint::screen("Ctrl-S", verb),
                Hint::screen("Tab", "champ suivant"),
            ]
        }
    }
}

/// Moving about and getting out: the same from one screen to the next.
pub fn global_hints(app: &App) -> Vec<Hint> {
    // There is no back from the board: `q` leaves for good, and saying so
    // spares the surprise.
    let mut hints = Vec::new();
    // What waits on the user comes first among the common keys: it is the
    // one that changes what to do next.
    let waiting = app.attention_queue().len();
    if waiting > 0 {
        hints.push(Hint::global("!", format!("à toi ({waiting})")));
    }
    hints.extend([
        Hint::global("Tab", "écran"),
        Hint::global(":", "commande"),
        Hint::global("A", "activité"),
        Hint::global("?", "aide"),
    ]);
    if app.screen == Screen::Board {
        hints.push(Hint::global("q", "quitter"));
    } else {
        hints.push(Hint::global("q", "retour"));
        // Someone who does not know how to leave types "exit": the answer
        // belongs in front of them, not three screens back.
        hints.push(Hint::global("Q", "quitter"));
    }
    hints
}

fn board_hints(app: &App) -> Vec<Hint> {
    let mut hints = match app.board_pane {
        BoardPane::Projects => vec![
            Hint::screen("Entrée", "voir ses tickets"),
            Hint::screen("j/k", "changer de projet"),
            Hint::screen("d", "oublier"),
        ],
        BoardPane::Tickets => vec![
            Hint::screen("Entrée", "ouvrir le ticket"),
            Hint::screen("h/l", "changer de colonne"),
        ],
    };
    hints.push(Hint::screen("n", "nouveau ticket"));
    hints
}

fn ticket_hints(app: &App) -> Vec<Hint> {
    let Some(detail) = app.ticket.as_ref() else {
        return Vec::new();
    };
    let status = detail.ticket.status;
    let mut hints = Vec::new();
    // What the ticket's state makes possible today comes first: it is the
    // decision of the moment, and it is what must survive a trim.
    if app.can_integrate() {
        hints.push(Hint::screen("f", app.integration_label()));
    }
    if status == orchestra_core::model::TicketStatus::Review {
        hints.push(Hint::screen("t", "marquer terminé"));
    }
    if status.is_terminal() {
        hints.push(Hint::screen("o", "rouvrir"));
    }
    hints.push(Hint::screen("p", "planifier"));
    if detail.ticket.proposal.is_some() || detail.ticket.team.is_some() {
        hints.push(Hint::screen("a", "relire l'équipe"));
    }
    hints.push(Hint::screen("L", "lancer"));
    hints.push(Hint::screen("x", "arrêter"));
    if !detail.agents.is_empty() {
        hints.push(Hint::screen("Entrée", "suivre l'agent"));
    }
    hints.push(Hint::screen("n", "nouveau ticket"));
    if detail.ticket.branch.is_some() {
        hints.push(Hint::screen("D", "diff"));
    }
    hints.push(Hint::screen(
        "T",
        if app.ticket_timeline { "le brief" } else { "chronologie" },
    ));
    hints
}

fn agent_hints(app: &App) -> Vec<Hint> {
    let mut hints = Vec::new();
    if app
        .watched_agent()
        .is_some_and(|a| a.agent.status.is_active())
    {
        hints.push(Hint::screen("s", "consigne"));
        hints.push(Hint::screen("S", "rediriger"));
        hints.push(Hint::screen("x", "arrêter"));
    }
    // Going back to the live tail only means something once you have left
    // it; offering it always would drown the keys that change something.
    if app.log.is_following() {
        hints.push(Hint::screen("j/k", "remonter le journal"));
    } else {
        hints.push(Hint::screen("G", "revenir au direct"));
    }
    if app.watched_agent().is_some() {
        hints.push(Hint::screen("o", "son pane"));
        if app.ticket.as_ref().is_some_and(|d| d.ticket.branch.is_some()) {
            hints.push(Hint::screen("D", "diff"));
        }
        if !app
            .watched_agent()
            .is_some_and(|a| a.agent.status.is_active())
        {
            hints.push(Hint::screen("T", "reprendre la main"));
        }
    }
    hints
}

fn cost_hints(app: &App) -> Vec<Hint> {
    let c = &app.cost;
    // Each key states where it stands: the label is both what one is looking
    // at and what the key will change it to.
    vec![
        Hint::screen("m", format!("par {}", c.group.label_fr())),
        Hint::screen("p", c.period.label_fr()),
        Hint::screen(
            "u",
            if c.include_unmanaged {
                "sessions libres incluses"
            } else {
                "agents seulement"
            },
        ),
    ]
}

fn todo_hints(app: &App) -> Vec<Hint> {
    if app.todos.is_empty() {
        return Vec::new();
    }
    let mut hints = vec![Hint::screen("d", "supprimer")];
    if app.selected_project().is_some() {
        hints.push(Hint::screen("p", "promouvoir en ticket"));
    }
    hints
}

fn rules_hints(app: &App) -> Vec<Hint> {
    use orchestra_core::conventions::{RuleKind, RuleStatus};
    if let Some(role) = app.selected_role() {
        let mut hints = vec![Hint::screen("e", "éditer")];
        hints.push(match role.git.unwrap_or_default() {
            orchestra_core::guard::GitPolicy::Full => Hint::screen("p", "confiner git"),
            orchestra_core::guard::GitPolicy::Confined => Hint::screen("p", "ouvrir git"),
        });
        if role.scope == orchestra_core::model::RoleScope::Project
            && app.selected_project().is_some()
        {
            hints.push(Hint::screen("g", "rendre global"));
        }
        hints.push(Hint::screen("d", "supprimer"));
        return hints;
    }
    let Some(r) = app.selected_rule() else {
        return Vec::new();
    };
    let mut hints = Vec::new();
    if r.status != RuleStatus::Accepted {
        hints.push(Hint::screen("y", "accepter"));
    }
    if r.status != RuleStatus::Rejected {
        hints.push(Hint::screen("x", "rejeter"));
    }
    if r.kind == RuleKind::Adr && r.status == RuleStatus::Accepted {
        hints.push(Hint::screen("s", "remplacée"));
    }
    hints.push(Hint::screen("e", "éditer"));
    if r.kind == RuleKind::Convention
        && r.scope == orchestra_core::model::RoleScope::Project
        && app.selected_project().is_some()
    {
        hints.push(Hint::screen("g", "rendre globale"));
    }
    hints.push(Hint::screen("d", "supprimer"));
    hints
}

fn proposal_hints(app: &App) -> Vec<Hint> {
    if app.editor.is_empty() {
        return Vec::new();
    }
    vec![
        Hint::screen("y", "accepter l'équipe"),
        Hint::screen("e", "objectif"),
        Hint::screen("c", "critères"),
        Hint::screen("m", "modèle"),
        Hint::screen("E", "effort"),
        Hint::screen("a", "ajouter"),
        Hint::screen("d", "retirer"),
        Hint::screen("J/K", "déplacer"),
        Hint::screen("r", "replanifier"),
    ]
}

/// What moves the same way everywhere.
pub const NAVIGATION: &[(&str, &str)] = &[
    ("1…8", "aller à un écran"),
    ("Tab", "écran suivant"),
    ("⇧Tab", "écran précédent"),
    ("j k", "descendre / monter"),
    ("h l", "gauche / droite"),
    ("g G", "début / fin"),
    ("PgUp PgDn", "page haut / bas"),
    ("Entrée", "ouvrir"),
];

/// What answers the same way everywhere.
pub const PARTOUT: &[(&str, &str)] = &[
    (":", "palette de commandes"),
    ("R", "rafraîchir"),
    ("?", "cette aide"),
    ("q", "revenir en arrière"),
    ("Q", "quitter"),
    ("Ctrl-C", "quitter"),
    ("!", "aller à ce qui t'attend"),
    ("A", "replier le bandeau Activité"),
];

/// What the palette takes. Written as one types it, brackets left off: a
/// command is not a key.
pub const PALETTE: &[(&str, &str)] = &[
    (":project add <chemin>", "ajouter un projet"),
    (":project forget <nom>", "oublier un projet"),
    (":todo add <titre>", "ajouter un todo"),
    (":todo urgent <n>", "basculer l'urgence"),
    (":todo due <n> <AAAA-MM-JJ>", "fixer l'échéance"),
    (":todo open|doing|done|drop <n>", "changer le statut"),
    (":convention add <titre>", "écrire une convention"),
    (":adr add <titre>", "écrire une décision du projet"),
    (":role add <nom>", "créer un rôle (projet sélectionné, sinon global)"),
    (":regles", "les rôles, conventions et ADR"),
    (":usage", "les coûts"),
    (":refresh", "tout relire"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hint_reads_key_then_action() {
        let h = Hint::screen("p", "planifier");
        let text: String = h.spans().iter().map(|s| s.content.to_string()).collect();
        assert_eq!(text, "[p] planifier");
        assert_eq!(h.width(), text.chars().count());
    }

    #[test]
    fn the_screen_comes_before_what_works_everywhere() {
        let app = App::new();
        let hints = strip(&app);
        let first_global = hints.iter().position(|h| h.scope == Scope::Global);
        let last_screen = hints.iter().rposition(|h| h.scope == Scope::Screen);
        assert!(
            last_screen < first_global,
            "les touches de l'écran passent devant : {hints:?}"
        );
    }

    #[test]
    fn typing_never_advertises_keys_that_would_be_typed() {
        let mut app = App::new();
        app.screen = Screen::NewTicket;
        let hints = strip(&app);
        assert!(
            !hints.iter().any(|h| h.key == "?" || h.key == "q"),
            "« ? » et « q » s'écrivent dans le champ : {hints:?}"
        );
        assert!(hints.iter().any(|h| h.key == "Ctrl-S"));
    }

    #[test]
    fn an_empty_screen_offers_nothing_to_press() {
        // No ticket is open, so the Ticket screen has nothing to offer:
        // announcing « lancer » would promise what would not work.
        let mut app = App::new();
        app.screen = Screen::Ticket;
        assert!(screen_hints(&app).is_empty());
        // The bar still keeps a way back out.
        assert!(strip(&app).iter().any(|h| h.key == "?"));
    }

    #[test]
    fn the_cost_keys_say_what_they_show() {
        let mut app = App::new();
        app.screen = Screen::Cost;
        let hints = screen_hints(&app);
        assert!(hints.iter().any(|h| h.key == "u"
            && (h.label == "agents seulement" || h.label == "sessions libres incluses")));
    }

    #[test]
    fn a_narrow_bar_drops_what_the_help_repeats_before_what_it_does_not() {
        let mut app = App::new();
        app.screen = Screen::Cost;
        let screen = screen_hints(&app);
        let wide = strip_for_width(&app, 200);
        assert_eq!(wide, strip(&app), "au large, tout tient");
        for room in [30, 14] {
            let tight = strip_for_width(&app, line_width(&screen) + room);
            assert!(
                tight.iter().any(|h| h.key == "?"),
                "l'aide reste joignable à {room} colonnes près : {tight:?}"
            );
        }
        let tight = strip_for_width(&app, line_width(&screen) + 30);
        for hint in &screen {
            assert!(tight.contains(hint), "{hint:?} a sauté avant les communes");
        }
        // Too narrow for anything else: the screen's keys, and the `?` the
        // renderer keeps by trimming what comes before it.
        let last = strip_for_width(&app, 10);
        assert_eq!(last.last().map(|h| h.key.as_str()), Some("?"));
        assert_eq!(&last[..last.len() - 1], &screen[..]);
    }

    #[test]
    fn the_board_says_q_quits_because_there_is_nowhere_to_go_back_to() {
        let app = App::new();
        assert!(global_hints(&app)
            .iter()
            .any(|h| h.key == "q" && h.label == "quitter"));
        let mut app = App::new();
        app.screen = Screen::Cost;
        assert!(global_hints(&app)
            .iter()
            .any(|h| h.key == "q" && h.label == "retour"));
    }
}
