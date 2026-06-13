//! Orchestration réelle — modèle de **plan** exécutable par l'orchestre.
//!
//! Un [`Plan`] décompose un objectif en [`Task`]s assignées à des agents, reliées par des
//! **dépendances** (`depends_on`). Le runtime exécute le plan en **ordre topologique**, chaque
//! tâche recevant en contexte les sorties de ses dépendances (passage de relais). Ce module ne
//! contient que la logique *pure* (modèle, validation, tri, plan de repli) — l'exécution
//! asynchrone et les appels LLM vivent dans [`crate::runtime`].

use std::collections::{HashMap, VecDeque};

use serde_json::Value;

/// État d'avancement d'une tâche du plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    Running,
    Done,
    Failed,
}

/// Une étape du plan : un agent réalise un objectif, éventuellement après d'autres étapes.
#[derive(Debug, Clone)]
pub struct Task {
    pub id: String,
    pub agent: String,
    pub objective: String,
    pub depends_on: Vec<String>,
    pub status: TaskStatus,
    pub output: String,
}

impl Task {
    pub fn new(
        id: impl Into<String>,
        agent: impl Into<String>,
        objective: impl Into<String>,
        depends_on: Vec<String>,
    ) -> Self {
        Self {
            id: id.into(),
            agent: agent.into(),
            objective: objective.into(),
            depends_on,
            status: TaskStatus::Pending,
            output: String::new(),
        }
    }
}

/// Un plan d'orchestration : un ensemble de tâches reliées par leurs dépendances.
#[derive(Debug, Clone)]
pub struct Plan {
    pub tasks: Vec<Task>,
}

impl Plan {
    pub fn new(tasks: Vec<Task>) -> Self {
        Self { tasks }
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// Valide le plan : ids non vides et uniques, agents connus, dépendances existantes, et
    /// absence de cycle. Renvoie un message explicite en cas de problème.
    pub fn validate(&self, agents: &[String]) -> Result<(), String> {
        let mut seen = HashMap::new();
        for t in &self.tasks {
            if t.id.trim().is_empty() {
                return Err("une tâche a un id vide".to_string());
            }
            if seen.insert(t.id.as_str(), ()).is_some() {
                return Err(format!("id de tâche en double : « {} »", t.id));
            }
            if !agents.iter().any(|a| a == &t.agent) {
                return Err(format!("tâche « {} » : agent inconnu « {} »", t.id, t.agent));
            }
        }
        for t in &self.tasks {
            for dep in &t.depends_on {
                if !seen.contains_key(dep.as_str()) {
                    return Err(format!("tâche « {} » : dépendance inconnue « {dep} »", t.id));
                }
            }
        }
        self.topo_order().map(|_| ())
    }

    /// Ordre topologique des ids de tâches (Kahn). Conserve l'ordre de déclaration entre
    /// tâches indépendantes (déterminisme). Erreur si un cycle empêche d'ordonner.
    pub fn topo_order(&self) -> Result<Vec<String>, String> {
        let mut indegree: HashMap<&str, usize> = self.tasks.iter().map(|t| (t.id.as_str(), 0)).collect();
        // dependents[dep] = tâches qui dépendent de `dep`
        let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
        for t in &self.tasks {
            for dep in &t.depends_on {
                *indegree.entry(t.id.as_str()).or_insert(0) += 1;
                dependents.entry(dep.as_str()).or_default().push(t.id.as_str());
            }
        }
        // File initiale : tâches sans dépendance, dans l'ordre de déclaration.
        let mut queue: VecDeque<&str> = self
            .tasks
            .iter()
            .filter(|t| indegree.get(t.id.as_str()).copied().unwrap_or(0) == 0)
            .map(|t| t.id.as_str())
            .collect();

        let mut order = Vec::with_capacity(self.tasks.len());
        while let Some(id) = queue.pop_front() {
            order.push(id.to_string());
            if let Some(deps) = dependents.get(id) {
                for &next in deps {
                    let d = indegree.get_mut(next).expect("indegree connu");
                    *d -= 1;
                    if *d == 0 {
                        queue.push_back(next);
                    }
                }
            }
        }
        if order.len() != self.tasks.len() {
            return Err("cycle de dépendances dans le plan".to_string());
        }
        Ok(order)
    }
}

/// Plan de **repli déterministe** (hors-ligne ou si la planification LLM échoue) : un pipeline
/// linéaire sur le roster — chaque agent contribue à l'objectif après le précédent.
pub fn fallback_plan(agents: &[String], objective: &str) -> Plan {
    let mut tasks = Vec::with_capacity(agents.len());
    for (i, agent) in agents.iter().enumerate() {
        let id = format!("t{}", i + 1);
        let depends_on = if i == 0 { Vec::new() } else { vec![format!("t{i}")] };
        tasks.push(Task::new(id, agent.clone(), objective.to_string(), depends_on));
    }
    Plan::new(tasks)
}

/// Construit un [`Plan`] depuis le JSON renvoyé par le LLM (outil `submit_plan`) :
/// `{ "tasks": [ { "id", "agent", "objective", "depends_on"? } ] }`. `None` si la forme est
/// inexploitable (le caller retombe alors sur [`fallback_plan`]).
pub fn parse_plan(value: &Value) -> Option<Plan> {
    let arr = value.get("tasks")?.as_array()?;
    let mut tasks = Vec::with_capacity(arr.len());
    for t in arr {
        let id = t.get("id")?.as_str()?.trim().to_string();
        let agent = t.get("agent")?.as_str()?.trim().to_string();
        let objective = t.get("objective").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if id.is_empty() || agent.is_empty() {
            return None;
        }
        let depends_on = t
            .get("depends_on")
            .and_then(Value::as_array)
            .map(|d| d.iter().filter_map(|v| v.as_str().map(|s| s.trim().to_string())).collect())
            .unwrap_or_default();
        tasks.push(Task::new(id, agent, objective, depends_on));
    }
    Some(Plan::new(tasks))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn agents() -> Vec<String> {
        vec!["A".into(), "B".into(), "C".into()]
    }

    #[test]
    fn topo_order_respects_dependencies() {
        let plan = Plan::new(vec![
            Task::new("t1", "A", "o", vec![]),
            Task::new("t2", "B", "o", vec!["t1".into()]),
            Task::new("t3", "C", "o", vec!["t1".into(), "t2".into()]),
        ]);
        assert_eq!(plan.topo_order().unwrap(), vec!["t1", "t2", "t3"]);
        assert!(plan.validate(&agents()).is_ok());
    }

    #[test]
    fn validate_rejects_cycle() {
        let plan = Plan::new(vec![
            Task::new("t1", "A", "o", vec!["t2".into()]),
            Task::new("t2", "B", "o", vec!["t1".into()]),
        ]);
        assert!(plan.topo_order().is_err());
        assert!(plan.validate(&agents()).unwrap_err().contains("cycle"));
    }

    #[test]
    fn validate_rejects_unknown_agent_dep_and_dup() {
        let unknown = Plan::new(vec![Task::new("t1", "Z", "o", vec![])]);
        assert!(unknown.validate(&agents()).unwrap_err().contains("agent inconnu"));

        let bad_dep = Plan::new(vec![Task::new("t1", "A", "o", vec!["tX".into()])]);
        assert!(bad_dep.validate(&agents()).unwrap_err().contains("dépendance inconnue"));

        let dup = Plan::new(vec![Task::new("t1", "A", "o", vec![]), Task::new("t1", "B", "o", vec![])]);
        assert!(dup.validate(&agents()).unwrap_err().contains("double"));
    }

    #[test]
    fn fallback_plan_is_linear_pipeline() {
        let plan = fallback_plan(&agents(), "objectif");
        assert_eq!(plan.tasks.len(), 3);
        assert!(plan.tasks[0].depends_on.is_empty());
        assert_eq!(plan.tasks[1].depends_on, vec!["t1"]);
        assert_eq!(plan.tasks[2].depends_on, vec!["t2"]);
        assert!(plan.validate(&agents()).is_ok());
    }

    #[test]
    fn parse_plan_reads_tasks() {
        let v = json!({ "tasks": [
            { "id": "t1", "agent": "A", "objective": "faire X" },
            { "id": "t2", "agent": "B", "objective": "faire Y", "depends_on": ["t1"] }
        ]});
        let plan = parse_plan(&v).unwrap();
        assert_eq!(plan.tasks.len(), 2);
        assert_eq!(plan.tasks[1].depends_on, vec!["t1"]);
        assert!(plan.validate(&agents()).is_ok());

        assert!(parse_plan(&json!({ "nope": 1 })).is_none());
    }
}
