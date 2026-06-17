//! Catalogue partagé d'agents et de skills + opérations d'édition, **communs au TUI et à la
//! GUI**.
//!
//! Conformément au découplage strict du projet (cf. `CLAUDE.md` et `docs/ARCHITECTURE.md`),
//! toute la logique « quels agents/skills sont possibles, lesquels sont branchés, comment les
//! brancher » vit ici : les deux interfaces appellent les **mêmes** fonctions, ce qui garantit
//! un comportement identique des deux côtés.

use std::path::PathBuf;

use crate::error::OrchestraError;
use crate::markdown_skill;
use crate::model::{default_agents, AgentDef, ContextSpace, ProjectType};
use crate::skills;

/// Nature d'un skill dans le sélecteur d'un agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillKind {
    /// Primitive exécutable (code Rust).
    Primitive,
    /// Fiche d'instructions Markdown (`SKILL.md`), éditable.
    Fiche,
    /// Assigné à l'agent mais sans implémentation ni fiche : **à brancher**.
    Unwired,
}

/// Une entrée du sélecteur de skills : id, nature, description, et si l'agent l'a assigné.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillEntry {
    pub id: String,
    pub kind: SkillKind,
    pub description: String,
    pub selected: bool,
}

/// Catalogue de skills pour l'agent `agent_idx` : primitives (code) + fiches (`SKILL.md`) +
/// skills assignés mais non branchés (gardés *cochés* pour pouvoir les retirer ou les brancher).
pub fn skill_entries(space: &ContextSpace, agent_idx: usize) -> Vec<SkillEntry> {
    let assigned = space
        .config
        .agents
        .get(agent_idx)
        .map(|a| a.skills.clone())
        .unwrap_or_default();

    let mut out: Vec<SkillEntry> = Vec::new();
    // Primitives (code) — registre du cœur.
    for m in skills::catalog() {
        let selected = assigned.iter().any(|s| s == m.id);
        out.push(SkillEntry {
            id: m.id.to_string(),
            kind: SkillKind::Primitive,
            description: m.description,
            selected,
        });
    }
    // Fiches Markdown — `.orchestra/skills/`.
    for f in markdown_skill::load_all(&space.root) {
        if out.iter().any(|e| e.id == f.id) {
            continue;
        }
        let selected = assigned.iter().any(|s| s == &f.id || s == &f.name);
        out.push(SkillEntry { id: f.id, kind: SkillKind::Fiche, description: f.description, selected });
    }
    // Assignés mais non branchés → conservés (cochés) pour les retirer ou les brancher.
    for s in &assigned {
        if !out.iter().any(|e| &e.id == s) {
            out.push(SkillEntry {
                id: s.clone(),
                kind: SkillKind::Unwired,
                description: "(non branché)".into(),
                selected: true,
            });
        }
    }
    out
}

/// Agents « suggérés » pour un type de projet (modèle de départ) : les rôles classiques à
/// activer en un geste, sans les retaper. (= matrice de [`default_agents`].)
pub fn agent_templates(kind: ProjectType) -> Vec<AgentDef> {
    default_agents(kind)
}

/// Modèles d'agents suggérés **pas encore présents** dans l'espace (comparaison par nom), pour
/// proposer ce qu'il reste à activer.
pub fn inactive_agent_templates(space: &ContextSpace) -> Vec<AgentDef> {
    agent_templates(space.config.project_type)
        .into_iter()
        .filter(|t| !space.config.agents.iter().any(|a| a.name == t.name))
        .collect()
}

/// « Branche » un skill non branché : crée sa fiche `.orchestra/skills/<id>/SKILL.md` (à
/// rédiger). Renvoie le chemin du `SKILL.md` créé. Échoue si la fiche existe déjà ou si l'id
/// est invalide (jamais d'écrasement silencieux).
pub fn wire_skill(space: &ContextSpace, id: &str) -> Result<PathBuf, OrchestraError> {
    markdown_skill::create(&space.root, id, "")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ProjectType;
    use std::fs;

    // Construit un espace minimal sur disque (sans dépendre du chemin de scaffolding), pour des
    // tests déterministes.
    fn build_space(kind: ProjectType, agents: Vec<AgentDef>) -> ContextSpace {
        use crate::model::ProjectConfig;
        let root = std::env::temp_dir().join(format!(
            "orch-catalog-b-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".orchestra")).unwrap();
        let cfg = ProjectConfig {
            project_name: "Test".into(),
            project_type: kind,
            workspace_path: None,
            documentalist_enabled: false,
            skills: Vec::new(),
            agents,
            integrations: Default::default(),
        };
        fs::write(
            root.join(".orchestra").join("config.json"),
            serde_json::to_string_pretty(&cfg).unwrap(),
        )
        .unwrap();
        ContextSpace::load(&root).unwrap()
    }

    #[test]
    fn skill_entries_classifies_primitive_fiche_unwired() {
        let agent = AgentDef {
            name: "A".into(),
            role: String::new(),
            skills: vec!["Read_File".into(), "Generate_Quiz".into()],
        };
        let space = build_space(ProjectType::Langue, vec![agent]);

        let entries = skill_entries(&space, 0);
        let read = entries.iter().find(|e| e.id == "Read_File").unwrap();
        assert_eq!(read.kind, SkillKind::Primitive);
        assert!(read.selected);

        let quiz = entries.iter().find(|e| e.id == "Generate_Quiz").unwrap();
        assert_eq!(quiz.kind, SkillKind::Unwired); // ni primitive ni fiche → à brancher
        assert!(quiz.selected);

        let _ = fs::remove_dir_all(&space.root);
    }

    #[test]
    fn wire_skill_turns_unwired_into_fiche() {
        let agent = AgentDef {
            name: "A".into(),
            role: String::new(),
            skills: vec!["Generate_Quiz".into()],
        };
        let space = build_space(ProjectType::Langue, vec![agent]);

        wire_skill(&space, "Generate_Quiz").unwrap();
        let entries = skill_entries(&space, 0);
        let quiz = entries.iter().find(|e| e.id == "Generate_Quiz").unwrap();
        assert_eq!(quiz.kind, SkillKind::Fiche); // désormais branché (fiche créée)

        let _ = fs::remove_dir_all(&space.root);
    }

    #[test]
    fn inactive_templates_exclude_present_agents() {
        // Espace Langue avec seulement le Tuteur présent → le Correcteur reste suggéré.
        let agent = AgentDef::new("Agent_Tuteur");
        let space = build_space(ProjectType::Langue, vec![agent]);

        let names: Vec<String> =
            inactive_agent_templates(&space).into_iter().map(|a| a.name).collect();
        assert!(names.iter().any(|n| n == "Agent_Correcteur"));
        assert!(!names.iter().any(|n| n == "Agent_Tuteur"));

        let _ = fs::remove_dir_all(&space.root);
    }
}
