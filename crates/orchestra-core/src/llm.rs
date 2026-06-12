//! Clients LLM (Phase 4a) — Claude *ou* Gemini, au choix, en HTTP brut.
//!
//! Rust n'a pas de SDK officiel pour ces fournisseurs : on appelle donc directement leurs
//! API REST via `reqwest`. Une représentation **neutre** ([`Msg`], [`Block`],
//! [`ToolSpec`]) découple la boucle agentique du format de chaque fournisseur ; chaque
//! provider sait *rendre* cette représentation dans son protocole et *parser* sa réponse.
//!
//! Le client est *optionnel* : sans clé API, [`LlmClient::from_env`] renvoie `None` et le
//! runtime retombe sur les agents simulés. Les clés ne sont jamais codées en dur.

use std::time::Duration;

use serde_json::{json, Map, Value};
use thiserror::Error;

/// Modèles par défaut (surchargés par `ORCHESTRA_MODEL`).
pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-opus-4-8";
pub const DEFAULT_GEMINI_MODEL: &str = "gemini-2.0-flash";

const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const GEMINI_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";
const MAX_OUTPUT_TOKENS: u32 = 4096;

/// Fournisseur d'IA disponible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Anthropic,
    Gemini,
}

impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Provider::Anthropic => "Claude",
            Provider::Gemini => "Gemini",
        }
    }
}

/// Erreurs propres au client LLM. Le runtime les traite comme « LLM injoignable » et
/// bascule en mode simulé.
#[derive(Debug, Error)]
pub enum LlmError {
    #[error("erreur réseau vers l'API {0}")]
    Transport(#[from] reqwest::Error),
    #[error("réponse API {status} : {body}")]
    Api { status: u16, body: String },
    #[error("réponse API inattendue : {0}")]
    Shape(String),
}

/// Définition neutre d'un outil (Skill) exposé au modèle. `parameters` est un JSON Schema.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// Bloc de contenu produit par le modèle.
#[derive(Debug, Clone)]
pub enum Block {
    Text(String),
    ToolUse { id: String, name: String, input: Value },
}

/// Résultat d'exécution d'un outil, renvoyé au modèle au tour suivant.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub id: String,
    pub name: String,
    pub content: String,
    pub is_error: bool,
}

/// Un message de la conversation, indépendant du fournisseur.
#[derive(Debug, Clone)]
pub enum Msg {
    User(String),
    Assistant(Vec<Block>),
    Tool(Vec<ToolResult>),
}

/// Client réutilisable (le `reqwest::Client` est `Arc` en interne).
#[derive(Clone)]
pub struct LlmClient {
    provider: Provider,
    http: reqwest::Client,
    api_key: String,
    model: String,
}

impl LlmClient {
    /// Construit le client depuis l'environnement, ou `None` (→ mode simulé).
    ///
    /// Sélection : `ORCHESTRA_PROVIDER` (`anthropic`/`claude` ou `gemini`) force le choix ;
    /// sinon auto-détection selon la clé présente (`ANTHROPIC_API_KEY` puis
    /// `GEMINI_API_KEY`). Le modèle vient de `ORCHESTRA_MODEL`, sinon le défaut du provider.
    pub fn from_env() -> Option<Self> {
        let forced = std::env::var("ORCHESTRA_PROVIDER")
            .ok()
            .map(|p| p.trim().to_lowercase());

        let (provider, api_key) = match forced.as_deref() {
            Some("gemini") => (Provider::Gemini, key("GEMINI_API_KEY")?),
            Some("anthropic") | Some("claude") => (Provider::Anthropic, key("ANTHROPIC_API_KEY")?),
            _ => {
                if let Some(k) = key("ANTHROPIC_API_KEY") {
                    (Provider::Anthropic, k)
                } else {
                    (Provider::Gemini, key("GEMINI_API_KEY")?)
                }
            }
        };

        let model = std::env::var("ORCHESTRA_MODEL")
            .ok()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| match provider {
                Provider::Anthropic => DEFAULT_ANTHROPIC_MODEL.to_string(),
                Provider::Gemini => DEFAULT_GEMINI_MODEL.to_string(),
            });

        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .build()
            .ok()?;

        Some(Self { provider, http, api_key, model })
    }

    pub fn provider(&self) -> Provider {
        self.provider
    }
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Un tour de conversation : envoie l'historique + les outils, renvoie les blocs émis
    /// par le modèle (texte et/ou appels d'outils).
    pub async fn complete(
        &self,
        system: &str,
        tools: &[ToolSpec],
        conv: &[Msg],
    ) -> Result<Vec<Block>, LlmError> {
        match self.provider {
            Provider::Anthropic => {
                let body = anthropic_body(&self.model, system, tools, conv);
                let resp = self
                    .http
                    .post(ANTHROPIC_URL)
                    .header("x-api-key", &self.api_key)
                    .header("anthropic-version", ANTHROPIC_VERSION)
                    .json(&body)
                    .send()
                    .await?;
                parse_anthropic(&checked_json(resp).await?)
            }
            Provider::Gemini => {
                let url = format!("{GEMINI_BASE}/{}:generateContent", self.model);
                let body = gemini_body(system, tools, conv);
                let resp = self
                    .http
                    .post(url)
                    .header("x-goog-api-key", &self.api_key)
                    .json(&body)
                    .send()
                    .await?;
                parse_gemini(&checked_json(resp).await?)
            }
        }
    }
}

fn key(var: &str) -> Option<String> {
    std::env::var(var).ok().filter(|k| !k.trim().is_empty())
}

async fn checked_json(resp: reqwest::Response) -> Result<Value, LlmError> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(LlmError::Api { status: status.as_u16(), body });
    }
    Ok(resp.json().await?)
}

// --- Rendu / parsing Anthropic ------------------------------------------------------

fn anthropic_body(model: &str, system: &str, tools: &[ToolSpec], conv: &[Msg]) -> Value {
    let messages: Vec<Value> = conv
        .iter()
        .map(|m| match m {
            Msg::User(t) => json!({ "role": "user", "content": t }),
            Msg::Assistant(blocks) => json!({
                "role": "assistant",
                "content": blocks.iter().map(|b| match b {
                    Block::Text(t) => json!({ "type": "text", "text": t }),
                    Block::ToolUse { id, name, input } =>
                        json!({ "type": "tool_use", "id": id, "name": name, "input": input }),
                }).collect::<Vec<_>>()
            }),
            Msg::Tool(results) => json!({
                "role": "user",
                "content": results.iter().map(|r| json!({
                    "type": "tool_result",
                    "tool_use_id": r.id,
                    "content": r.content,
                    "is_error": r.is_error,
                })).collect::<Vec<_>>()
            }),
        })
        .collect();

    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    body.insert("max_tokens".into(), json!(MAX_OUTPUT_TOKENS));
    body.insert("system".into(), json!(system));
    body.insert("messages".into(), json!(messages));
    if !tools.is_empty() {
        let defs: Vec<Value> = tools
            .iter()
            .map(|t| json!({ "name": t.name, "description": t.description, "input_schema": t.parameters }))
            .collect();
        body.insert("tools".into(), json!(defs));
    }
    Value::Object(body)
}

fn parse_anthropic(v: &Value) -> Result<Vec<Block>, LlmError> {
    let content = v
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| LlmError::Shape("champ `content` absent".into()))?;
    let mut blocks = Vec::new();
    for b in content {
        match b.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(t) = b.get("text").and_then(Value::as_str) {
                    blocks.push(Block::Text(t.to_string()));
                }
            }
            Some("tool_use") => blocks.push(Block::ToolUse {
                id: b.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
                name: b.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                input: b.get("input").cloned().unwrap_or_else(|| json!({})),
            }),
            _ => {}
        }
    }
    Ok(blocks)
}

// --- Rendu / parsing Gemini ---------------------------------------------------------

fn gemini_body(system: &str, tools: &[ToolSpec], conv: &[Msg]) -> Value {
    let contents: Vec<Value> = conv
        .iter()
        .map(|m| match m {
            Msg::User(t) => json!({ "role": "user", "parts": [{ "text": t }] }),
            Msg::Assistant(blocks) => json!({
                "role": "model",
                "parts": blocks.iter().map(|b| match b {
                    Block::Text(t) => json!({ "text": t }),
                    Block::ToolUse { name, input, .. } =>
                        json!({ "functionCall": { "name": name, "args": input } }),
                }).collect::<Vec<_>>()
            }),
            Msg::Tool(results) => json!({
                "role": "user",
                "parts": results.iter().map(|r| json!({
                    "functionResponse": {
                        "name": r.name,
                        "response": { "result": r.content, "is_error": r.is_error }
                    }
                })).collect::<Vec<_>>()
            }),
        })
        .collect();

    let mut body = Map::new();
    body.insert("systemInstruction".into(), json!({ "parts": [{ "text": system }] }));
    body.insert("contents".into(), json!(contents));
    body.insert("generationConfig".into(), json!({ "maxOutputTokens": MAX_OUTPUT_TOKENS }));
    if !tools.is_empty() {
        let decls: Vec<Value> = tools
            .iter()
            .map(|t| json!({ "name": t.name, "description": t.description, "parameters": t.parameters }))
            .collect();
        body.insert("tools".into(), json!([{ "functionDeclarations": decls }]));
    }
    Value::Object(body)
}

fn parse_gemini(v: &Value) -> Result<Vec<Block>, LlmError> {
    let parts = v
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)
        .ok_or_else(|| LlmError::Shape("aucun `candidates[0].content.parts`".into()))?;
    let mut blocks = Vec::new();
    for (i, p) in parts.iter().enumerate() {
        if let Some(t) = p.get("text").and_then(Value::as_str) {
            blocks.push(Block::Text(t.to_string()));
        } else if let Some(fc) = p.get("functionCall") {
            let name = fc.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
            blocks.push(Block::ToolUse {
                // Gemini n'attribue pas d'ID : on en synthétise un (inutilisé côté Gemini,
                // qui apparie les résultats par nom).
                id: format!("{name}-{i}"),
                name,
                input: fc.get("args").cloned().unwrap_or_else(|| json!({})),
            });
        }
    }
    Ok(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_conv() -> Vec<Msg> {
        vec![
            Msg::User("salut".into()),
            Msg::Assistant(vec![Block::ToolUse {
                id: "t1".into(),
                name: "Read_File".into(),
                input: json!({ "path": "a.txt" }),
            }]),
            Msg::Tool(vec![ToolResult {
                id: "t1".into(),
                name: "Read_File".into(),
                content: "contenu".into(),
                is_error: false,
            }]),
        ]
    }

    fn tools() -> Vec<ToolSpec> {
        vec![ToolSpec {
            name: "Read_File".into(),
            description: "lit".into(),
            parameters: json!({ "type": "object", "properties": {} }),
        }]
    }

    #[test]
    fn anthropic_body_shapes_tool_roundtrip() {
        let b = anthropic_body("claude-opus-4-8", "sys", &tools(), &sample_conv());
        assert_eq!(b["messages"][0]["role"], "user");
        assert_eq!(b["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(b["messages"][2]["content"][0]["tool_use_id"], "t1");
        assert_eq!(b["tools"][0]["input_schema"]["type"], "object");
    }

    #[test]
    fn gemini_body_shapes_tool_roundtrip() {
        let b = gemini_body("sys", &tools(), &sample_conv());
        assert_eq!(b["contents"][1]["parts"][0]["functionCall"]["name"], "Read_File");
        assert_eq!(b["contents"][2]["parts"][0]["functionResponse"]["name"], "Read_File");
        assert_eq!(b["tools"][0]["functionDeclarations"][0]["name"], "Read_File");
        assert!(b.get("systemInstruction").is_some());
    }

    #[test]
    fn parse_each_provider_response() {
        let a = json!({ "content": [
            { "type": "text", "text": "ok" },
            { "type": "tool_use", "id": "x", "name": "Read_File", "input": { "path": "a" } }
        ]});
        let blocks = parse_anthropic(&a).unwrap();
        assert!(matches!(blocks[0], Block::Text(_)));
        assert!(matches!(blocks[1], Block::ToolUse { .. }));

        let g = json!({ "candidates": [{ "content": { "parts": [
            { "text": "ok" },
            { "functionCall": { "name": "Read_File", "args": { "path": "a" } } }
        ]}}]});
        let blocks = parse_gemini(&g).unwrap();
        assert!(matches!(blocks[0], Block::Text(_)));
        assert!(matches!(blocks[1], Block::ToolUse { .. }));
    }
}
