//! Form state: the new-ticket form and the team editor.
//!
//! Pure data and transitions, so both are tested without a terminal.

use orchestra_core::model::{Effort, RoleDefinition, TeamMember, TeamProposal};

/// Which field of the new-ticket form has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TicketField {
    Title,
    Brief,
}

impl TicketField {
    pub fn next(self) -> Self {
        match self {
            TicketField::Title => TicketField::Brief,
            TicketField::Brief => TicketField::Title,
        }
    }

    /// Enter inserts a newline here rather than validating.
    pub fn is_multiline(self) -> bool {
        matches!(self, TicketField::Brief)
    }
}

#[derive(Debug, Clone, Default)]
pub struct NewTicketForm {
    pub title: String,
    pub brief: String,
    pub error: Option<String>,
}

impl NewTicketForm {
    pub fn field_mut(&mut self, field: TicketField) -> &mut String {
        match field {
            TicketField::Title => &mut self.title,
            TicketField::Brief => &mut self.brief,
        }
    }

    pub fn field(&self, field: TicketField) -> &str {
        match field {
            TicketField::Title => &self.title,
            TicketField::Brief => &self.brief,
        }
    }

    pub fn type_char(&mut self, field: TicketField, c: char) {
        self.error = None;
        self.field_mut(field).push(c);
    }

    pub fn backspace(&mut self, field: TicketField) {
        self.error = None;
        self.field_mut(field).pop();
    }

    pub fn newline(&mut self, field: TicketField) {
        if field.is_multiline() {
            self.field_mut(field).push('\n');
        }
    }

    /// The trimmed title and brief, or why they are not usable.
    pub fn validated(&self) -> Result<(String, String), String> {
        let title = self.title.trim();
        let brief = self.brief.trim();
        if title.is_empty() {
            return Err("le titre est vide".into());
        }
        if brief.len() < 20 {
            return Err(
                "le brief est trop court : décris ce qui ne va pas et ce que tu attends".into(),
            );
        }
        Ok((title.to_string(), brief.to_string()))
    }

    pub fn clear(&mut self) {
        self.title.clear();
        self.brief.clear();
        self.error = None;
    }

    /// Pre-fill from a todo being promoted into a ticket.
    pub fn seed(&mut self, title: &str, brief: &str) {
        self.title = title.to_string();
        self.brief = brief.to_string();
        self.error = None;
    }
}

/// What the team editor is currently changing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum EditorMode {
    #[default]
    Browsing,
    /// Rewriting one member's objective.
    Objective { buffer: String },
}

/// The proposal, editable before it is accepted.
#[derive(Debug, Clone, Default)]
pub struct TeamEditor {
    pub summary: String,
    pub risks: Vec<String>,
    pub members: Vec<TeamMember>,
    pub selected: usize,
    pub mode: EditorMode,
    /// Roles available for this project, for adding and for validation.
    pub catalog: Vec<RoleDefinition>,
    pub error: Option<String>,
}

impl TeamEditor {
    pub fn load(&mut self, proposal: &TeamProposal, catalog: Vec<RoleDefinition>) {
        self.summary = proposal.summary.clone();
        self.risks = proposal.risks.clone();
        self.members = proposal.members.clone();
        self.selected = 0;
        self.mode = EditorMode::Browsing;
        self.catalog = catalog;
        self.error = None;
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    pub fn selected_member(&self) -> Option<&TeamMember> {
        self.members.get(self.selected)
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.members.is_empty() {
            self.selected = 0;
            return;
        }
        let next = (self.selected as isize + delta).clamp(0, self.members.len() as isize - 1);
        self.selected = next as usize;
    }

    /// Move the selected member up or down, keeping the selection on it.
    pub fn reorder(&mut self, delta: isize) {
        if self.members.len() < 2 {
            return;
        }
        let target = (self.selected as isize + delta).clamp(0, self.members.len() as isize - 1);
        let target = target as usize;
        if target != self.selected {
            self.members.swap(self.selected, target);
            self.selected = target;
        }
    }

    /// Cycle the model of the selected member through the offered aliases.
    ///
    /// `None` is the first value and means "leave whatever Claude Code is set
    /// to". An alias literally named `default` would be the same thing under
    /// another name, so it is dropped rather than offered twice.
    pub fn cycle_model(&mut self, aliases: &[String]) {
        let aliases: Vec<String> = aliases
            .iter()
            .filter(|a| a.as_str() != orchestra_core::config::MODEL_DEFAULT && !a.trim().is_empty())
            .cloned()
            .collect();
        let aliases = aliases.as_slice();
        let Some(member) = self.members.get_mut(self.selected) else {
            return;
        };
        // Position 0 is "leave the default"; the aliases follow it.
        let position = match &member.model {
            None => 0,
            Some(m) => aliases
                .iter()
                .position(|a| a == m)
                .map(|i| i + 1)
                .unwrap_or(0),
        };
        let next = (position + 1) % (aliases.len() + 1);
        member.model = if next == 0 {
            None
        } else {
            aliases.get(next - 1).cloned()
        };
    }

    pub fn cycle_effort(&mut self) {
        if let Some(member) = self.members.get_mut(self.selected) {
            member.effort = Some(match member.effort {
                None => Effort::Low,
                Some(e) => e.next(),
            });
        }
    }

    pub fn remove_selected(&mut self) {
        if self.members.is_empty() {
            return;
        }
        let removed = self.members.remove(self.selected);
        // A dependency on a role that just left would fail validation.
        for member in &mut self.members {
            member.depends_on.retain(|d| d != &removed.role);
        }
        self.selected = self.selected.min(self.members.len().saturating_sub(1));
        self.error = None;
    }

    /// Add the first catalog role not already on the team.
    pub fn add_next_role(&mut self) {
        let used: Vec<&str> = self.members.iter().map(|m| m.role.as_str()).collect();
        let Some(role) = self
            .catalog
            .iter()
            .find(|r| !used.contains(&r.name.as_str()))
        else {
            self.error = Some("tous les rôles du catalogue sont déjà dans l'équipe".into());
            return;
        };
        self.members.push(TeamMember {
            role: role.name.clone(),
            objective: role.description.clone(),
            depends_on: Vec::new(),
            model: None,
            effort: None,
            max_budget_usd: None,
            parallel_ok: false,
        });
        self.selected = self.members.len() - 1;
        self.error = None;
    }

    pub fn start_editing_objective(&mut self) {
        if let Some(member) = self.members.get(self.selected) {
            self.mode = EditorMode::Objective {
                buffer: member.objective.clone(),
            };
        }
    }

    pub fn type_char(&mut self, c: char) {
        if let EditorMode::Objective { buffer } = &mut self.mode {
            buffer.push(c);
        }
    }

    pub fn backspace(&mut self) {
        if let EditorMode::Objective { buffer } = &mut self.mode {
            buffer.pop();
        }
    }

    /// Keep what was typed, or drop it.
    pub fn finish_editing(&mut self, keep: bool) {
        if let EditorMode::Objective { buffer } = std::mem::take(&mut self.mode) {
            if keep {
                let text = buffer.trim().to_string();
                if !text.is_empty() {
                    if let Some(member) = self.members.get_mut(self.selected) {
                        member.objective = text;
                    }
                }
            }
        }
    }

    pub fn is_editing(&self) -> bool {
        matches!(self.mode, EditorMode::Objective { .. })
    }

    /// The team as it stands, ready to send.
    pub fn to_proposal(&self) -> TeamProposal {
        TeamProposal {
            summary: self.summary.clone(),
            members: self.members.clone(),
            risks: self.risks.clone(),
            estimated_size: orchestra_core::model::Size::M,
        }
    }

    /// Execution stages, when the team is valid. Shown before accepting so the
    /// order is not a surprise.
    pub fn stages(&self) -> Result<Vec<Vec<String>>, String> {
        let names = self.catalog.iter().map(|r| r.name.clone()).collect();
        orchestra_core::model::Team::from_proposal(&self.to_proposal(), &names)
            .map(|t| t.stages)
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn role(name: &str) -> RoleDefinition {
        RoleDefinition {
            name: name.into(),
            description: format!("rôle {name}"),
            model: None,
            effort: None,
            allowed_tools: vec![],
            disallowed_tools: vec![],
            max_budget_usd: None,
            subagents: None,
            tags: vec![],
            git: None,
            system_prompt: "consigne".into(),
            source: PathBuf::from("/tmp"),
            scope: orchestra_core::model::RoleScope::Global,
        }
    }

    fn member(name: &str, deps: &[&str]) -> TeamMember {
        TeamMember {
            role: name.into(),
            objective: format!("objectif {name}"),
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            model: None,
            effort: None,
            max_budget_usd: None,
            parallel_ok: false,
        }
    }

    fn editor() -> TeamEditor {
        let mut e = TeamEditor::default();
        e.load(
            &TeamProposal {
                summary: "résumé".into(),
                members: vec![member("architect", &[]), member("backend", &["architect"])],
                risks: vec!["un risque".into()],
                estimated_size: orchestra_core::model::Size::M,
            },
            vec![role("architect"), role("backend"), role("tests")],
        );
        e
    }

    #[test]
    fn the_form_refuses_what_the_orchestrator_could_not_use() {
        let mut f = NewTicketForm::default();
        assert!(f.validated().is_err(), "un titre vide");

        for c in "Ajouter un cache".chars() {
            f.type_char(TicketField::Title, c);
        }
        let err = f.validated().unwrap_err();
        assert!(err.contains("brief"), "{err}");

        for c in "Le rendu recalcule tout à chaque fois, on veut un cache.".chars() {
            f.type_char(TicketField::Brief, c);
        }
        let (title, brief) = f.validated().unwrap();
        assert_eq!(title, "Ajouter un cache");
        assert!(brief.starts_with("Le rendu"));
    }

    #[test]
    fn seeding_fills_the_form_for_a_promoted_todo() {
        let mut f = NewTicketForm {
            error: Some("boum".into()),
            ..Default::default()
        };
        f.seed("Ajouter un cache", "Le rendu recalcule tout à chaque fois.");
        assert_eq!(f.title, "Ajouter un cache");
        assert!(f.brief.starts_with("Le rendu"));
        assert!(f.error.is_none());
    }

    #[test]
    fn enter_writes_a_newline_in_the_brief_only() {
        let mut f = NewTicketForm::default();
        f.newline(TicketField::Title);
        assert!(f.title.is_empty(), "le titre reste sur une ligne");
        f.newline(TicketField::Brief);
        assert_eq!(f.brief, "\n");
    }

    #[test]
    fn typing_clears_a_previous_error() {
        let mut f = NewTicketForm {
            error: Some("boum".into()),
            ..Default::default()
        };
        f.type_char(TicketField::Title, 'a');
        assert!(f.error.is_none());
    }

    #[test]
    fn the_editor_moves_and_reorders_without_losing_the_selection() {
        let mut e = editor();
        assert_eq!(e.selected_member().unwrap().role, "architect");
        e.move_selection(1);
        assert_eq!(e.selected_member().unwrap().role, "backend");
        e.move_selection(5);
        assert_eq!(e.selected, 1, "la sélection reste dans les bornes");

        e.reorder(-1);
        assert_eq!(e.members[0].role, "backend");
        assert_eq!(
            e.selected_member().unwrap().role,
            "backend",
            "la sélection suit le rôle déplacé"
        );
        e.reorder(-1);
        assert_eq!(e.selected, 0, "on ne sort pas par le haut");
    }

    #[test]
    fn removing_a_role_removes_the_dependencies_on_it() {
        let mut e = editor();
        // Remove `architect`, which `backend` depends on.
        e.selected = 0;
        e.remove_selected();
        assert_eq!(e.members.len(), 1);
        assert!(
            e.members[0].depends_on.is_empty(),
            "sinon l'équipe ne passerait plus la validation"
        );
        assert!(e.stages().is_ok());
    }

    #[test]
    fn adding_offers_each_catalog_role_once() {
        let mut e = editor();
        e.add_next_role();
        assert_eq!(e.members.len(), 3);
        assert_eq!(e.members[2].role, "tests");
        assert_eq!(e.selected, 2, "le nouveau rôle est sélectionné");

        // Nothing left to add.
        e.add_next_role();
        assert_eq!(e.members.len(), 3);
        assert!(e.error.is_some());
    }

    #[test]
    fn cycling_the_model_returns_to_the_default() {
        let aliases = vec!["opus".to_string(), "sonnet".to_string()];
        let mut e = editor();
        assert!(e.selected_member().unwrap().model.is_none());
        e.cycle_model(&aliases);
        assert_eq!(e.selected_member().unwrap().model.as_deref(), Some("opus"));
        e.cycle_model(&aliases);
        assert_eq!(
            e.selected_member().unwrap().model.as_deref(),
            Some("sonnet")
        );
        e.cycle_model(&aliases);
        assert!(
            e.selected_member().unwrap().model.is_none(),
            "le tour revient au modèle par défaut"
        );

        // A model that is not offered any more restarts the cycle.
        e.members[e.selected].model = Some("un-modele-retire".into());
        e.cycle_model(&aliases);
        assert_eq!(e.selected_member().unwrap().model.as_deref(), Some("opus"));

        // No alias configured: the default is the only choice.
        e.cycle_model(&[]);
        assert!(e.selected_member().unwrap().model.is_none());
    }

    #[test]
    fn an_alias_named_default_is_not_offered_twice() {
        // `None` already means the default; cycling must not produce a model
        // literally called "default", which Claude Code would not understand.
        let aliases = vec![
            "default".to_string(),
            "opus".to_string(),
            "sonnet".to_string(),
        ];
        let mut e = editor();
        e.cycle_model(&aliases);
        assert_eq!(e.selected_member().unwrap().model.as_deref(), Some("opus"));
        e.cycle_model(&aliases);
        assert_eq!(
            e.selected_member().unwrap().model.as_deref(),
            Some("sonnet")
        );
        e.cycle_model(&aliases);
        assert!(e.selected_member().unwrap().model.is_none());
    }

    #[test]
    fn cycling_the_effort_starts_from_the_lowest() {
        let mut e = editor();
        e.cycle_effort();
        assert_eq!(e.selected_member().unwrap().effort, Some(Effort::Low));
        e.cycle_effort();
        assert_eq!(e.selected_member().unwrap().effort, Some(Effort::Medium));
    }

    #[test]
    fn editing_an_objective_can_be_kept_or_dropped() {
        let mut e = editor();
        let original = e.selected_member().unwrap().objective.clone();

        e.start_editing_objective();
        assert!(e.is_editing());
        e.backspace();
        for c in " revu".chars() {
            e.type_char(c);
        }
        e.finish_editing(false);
        assert_eq!(e.selected_member().unwrap().objective, original, "abandon");

        e.start_editing_objective();
        for c in " et complété".chars() {
            e.type_char(c);
        }
        e.finish_editing(true);
        assert_eq!(
            e.selected_member().unwrap().objective,
            format!("{original} et complété")
        );
        assert!(!e.is_editing());
    }

    #[test]
    fn an_emptied_objective_is_not_saved() {
        let mut e = editor();
        let original = e.selected_member().unwrap().objective.clone();
        e.start_editing_objective();
        for _ in 0..200 {
            e.backspace();
        }
        e.finish_editing(true);
        assert_eq!(e.selected_member().unwrap().objective, original);
    }

    #[test]
    fn stages_are_shown_before_accepting_and_errors_explained() {
        let e = editor();
        assert_eq!(
            e.stages().unwrap(),
            vec![vec!["architect".to_string()], vec!["backend".to_string()]]
        );

        // A role outside the catalog is refused with its name.
        let mut broken = editor();
        broken.members.push(member("magicien", &[]));
        let err = broken.stages().unwrap_err();
        assert!(err.contains("magicien"), "{err}");
    }

    #[test]
    fn an_empty_editor_does_not_panic() {
        let mut e = TeamEditor::default();
        e.move_selection(1);
        e.reorder(1);
        e.cycle_effort();
        e.cycle_model(&["opus".to_string()]);
        e.remove_selected();
        e.start_editing_objective();
        e.finish_editing(true);
        assert!(e.is_empty());
        assert!(e.stages().is_err(), "une équipe vide ne se lance pas");
    }
}
