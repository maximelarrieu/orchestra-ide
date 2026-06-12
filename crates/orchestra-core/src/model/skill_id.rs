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
        ProjectType::Nutrition => &["Web_Search", "Calorie_Calculator", "File_Append"],
        ProjectType::Langue => &["Generate_Quiz", "Translate_Text", "Text_To_Speech"],
        ProjectType::Immobilier => {
            &["Scrape_Web_Page", "Extract_JSON_From_HTML", "Geocoding_Calcul"]
        }
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
        ProjectType::Nutrition => &[
            ("Agent_Planificateur", "Construit des plans de repas adaptés aux objectifs."),
            ("Agent_Nutritionniste", "Analyse les apports et conseille sur l'équilibre."),
        ],
        ProjectType::Langue => &[
            ("Agent_Tuteur", "Donne des leçons et des exercices adaptés au niveau."),
            ("Agent_Correcteur", "Corrige les réponses et explique les erreurs."),
        ],
        ProjectType::Immobilier => &[
            ("Agent_Scraper", "Collecte les annonces depuis les sources configurées."),
            ("Agent_Filtrage", "Filtre et classe les annonces selon les critères stricts."),
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
