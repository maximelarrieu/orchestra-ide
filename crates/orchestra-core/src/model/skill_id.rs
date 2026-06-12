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

/// Composition par défaut de l'« orchestre » d'agents selon le type de projet.
///
/// Comme [`default_skills`], cette matrice pré-remplit `config.json` lors de
/// `orchestra init` (Phase 2). Les agents ne sont que des noms pour l'instant ;
/// le runtime qui les fait vivre arrive en Phase 3.
pub fn default_agents(kind: ProjectType) -> Vec<String> {
    let agents: &[&str] = match kind {
        ProjectType::Dev => &["Agent_Architecte", "Agent_Codeur", "Agent_Testeur"],
        ProjectType::Nutrition => &["Agent_Planificateur", "Agent_Nutritionniste"],
        ProjectType::Langue => &["Agent_Tuteur", "Agent_Correcteur"],
        ProjectType::Immobilier => &["Agent_Scraper", "Agent_Filtrage"],
    };
    agents.iter().map(|s| s.to_string()).collect()
}
