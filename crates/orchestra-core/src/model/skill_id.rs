use super::config::AgentDef;
use super::project_type::ProjectType;

/// Matrice des Skills activés par défaut selon le type de projet (spec §3).
///
/// Utilisée par `orchestra init` (Phase 2) pour pré-remplir `config.json`. Les Skills
/// sont pour l'instant des identifiants (`String`) ; le trait `Skill` exécutable arrive
/// en Phase 3.
pub fn default_skills(kind: ProjectType) -> Vec<String> {
    let skills: &[&str] = match kind {
        ProjectType::Dev => &["Read_File", "Write_File_Validated", "Execute_Terminal_Command"],
        ProjectType::Langue => &["Generate_Quiz", "Translate_Text", "Text_To_Speech"],
    };
    skills.iter().map(|s| s.to_string()).collect()
}

/// Composition par défaut de l'« orchestre » d'agents selon le type de projet : nom,
/// rôle (qui oriente le prompt) et skills propres (par défaut ceux du type, modifiables
/// ensuite agent par agent depuis l'interface).
pub fn default_agents(kind: ProjectType) -> Vec<AgentDef> {
    let roster: &[(&str, &str)] = match kind {
        ProjectType::Dev => &[
            ("Agent_Architecte", "Analyse les besoins et conçoit le plan d'implémentation."),
            ("Agent_Codeur", "Écrit et modifie le code selon le plan."),
            ("Agent_Testeur", "Écrit et exécute les tests, vérifie la qualité."),
        ],
        ProjectType::Langue => &[
            ("Agent_Tuteur", "Donne des leçons et des exercices adaptés au niveau."),
            ("Agent_Correcteur", "Corrige les réponses et explique les erreurs."),
        ],
    };
    let skills = default_skills(kind);
    roster
        .iter()
        .map(|(name, role)| AgentDef {
            name: name.to_string(),
            role: role.to_string(),
            skills: skills.clone(),
        })
        .collect()
}
