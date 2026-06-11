use serde::{Deserialize, Serialize};

/// Les quatre familles de projets que l'outil sait modéliser nativement.
///
/// Le type pilote la matrice de Skills/Agents par défaut (voir [`super::skill_id`])
/// et les templates générés par `orchestra init` (Phase 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectType {
    Dev,
    Nutrition,
    Langue,
    Immobilier,
}

impl ProjectType {
    /// Libellé court pour l'affichage.
    pub fn label(self) -> &'static str {
        match self {
            ProjectType::Dev => "Dev",
            ProjectType::Nutrition => "Nutrition",
            ProjectType::Langue => "Langue",
            ProjectType::Immobilier => "Immobilier",
        }
    }
}
