//! Client LLM Claude (Phase 4) — API Messages d'Anthropic en HTTP brut.
//!
//! Rust n'a pas de SDK Anthropic officiel : on appelle donc directement le point
//! d'entrée REST `POST /v1/messages` via `reqwest`, comme les exemples cURL de la doc.
//! Le client est *optionnel* : sans `ANTHROPIC_API_KEY`, [`LlmClient::from_env`] renvoie
//! `None` et le runtime retombe sur les agents simulés (Phase 3).
//!
//! La clé n'est jamais codée en dur : elle vient de la variable d'environnement.

use std::time::Duration;

use serde_json::{json, Map, Value};
use thiserror::Error;

/// Modèle par défaut — le plus capable de la famille Claude.
pub const DEFAULT_MODEL: &str = "claude-opus-4-8";
const API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Erreurs propres au client LLM. Le runtime les traite comme « LLM injoignable » et
/// bascule en mode simulé — elles ne remontent pas jusqu'au cœur applicatif.
#[derive(Debug, Error)]
pub enum LlmError {
    #[error("erreur réseau vers l'API Claude : {0}")]
    Transport(#[from] reqwest::Error),
    #[error("réponse API {status} : {body}")]
    Api { status: u16, body: String },
    #[error("réponse API inattendue : {0}")]
    Shape(String),
}

/// Réponse d'un appel `/v1/messages`, réduite à ce que la boucle agentique consomme.
pub struct LlmResponse {
    /// Blocs de contenu bruts (`text`, `tool_use`, …) — réinjectés tels quels comme tour
    /// `assistant` au tour suivant, conformément au protocole d'Anthropic.
    pub content: Vec<Value>,
    /// `end_turn`, `tool_use`, `max_tokens`, `refusal`…
    pub stop_reason: Option<String>,
}

/// Client réutilisable (le `reqwest::Client` est `Arc` en interne, clonage bon marché).
#[derive(Clone)]
pub struct LlmClient {
    http: reqwest::Client,
    api_key: String,
    model: String,
}

impl LlmClient {
    /// Construit le client depuis l'environnement. `None` si `ANTHROPIC_API_KEY` est
    /// absente ou vide — c'est le signal de repli sur la simulation.
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY").ok().filter(|k| !k.trim().is_empty())?;
        let model = std::env::var("ORCHESTRA_MODEL")
            .ok()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .build()
            .ok()?;
        Some(Self { http, api_key, model })
    }

    /// Nom du modèle utilisé (pour l'affichage).
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Un tour `/v1/messages`. `tools` peut être vide (champ alors omis). Pas de
    /// `thinking` : sur Opus 4.8 c'est valide et la boucle d'outils reste simple.
    pub async fn create_message(
        &self,
        system: &str,
        tools: &[Value],
        messages: &[Value],
    ) -> Result<LlmResponse, LlmError> {
        let mut body = Map::new();
        body.insert("model".into(), json!(self.model));
        body.insert("max_tokens".into(), json!(4096));
        body.insert("system".into(), json!(system));
        body.insert("messages".into(), json!(messages));
        if !tools.is_empty() {
            body.insert("tools".into(), json!(tools));
        }

        let resp = self
            .http
            .post(API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&Value::Object(body))
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(LlmError::Api { status: status.as_u16(), body });
        }

        let value: Value = resp.json().await?;
        let content = value
            .get("content")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| LlmError::Shape("champ `content` absent".into()))?;
        let stop_reason = value
            .get("stop_reason")
            .and_then(Value::as_str)
            .map(str::to_string);

        Ok(LlmResponse { content, stop_reason })
    }
}
