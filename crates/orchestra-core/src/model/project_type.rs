use serde::{Deserialize, Serialize};

/// Les familles de projets que l'outil sait modéliser nativement.
///
/// L'IDE est centré sur le **développement** ; **Langue** est conservé (mis de côté pour
/// l'instant). Le type pilote la matrice de Skills/Agents par défaut (voir [`super::skill_id`])
/// et les templates générés à la création d'un espace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectType {
    Dev,
    Langue,
}

impl ProjectType {
    /// Libellé court pour l'affichage.
    pub fn label(self) -> &'static str {
        match self {
            ProjectType::Dev => "Dev",
            ProjectType::Langue => "Langue",
        }
    }
}
