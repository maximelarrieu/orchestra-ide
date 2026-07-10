//! Clients LLM (Phase 4a) — Claude *ou* Gemini, au choix, en HTTP brut.
//!
//! Rust n'a pas de SDK officiel pour ces fournisseurs : on appelle donc directement leurs
//! API REST via `reqwest`. Une représentation **neutre** ([`Msg`], [`Block`],
//! [`ToolSpec`]) découple la boucle agentique du format de chaque fournisseur ; chaque
//! provider sait *rendre* cette représentation dans son protocole et *parser* sa réponse.
//!
//! Le client est *optionnel* : sans clé API, [`LlmClient::from_env`] renvoie `None` et le
//! runtime retombe sur les agents simulés. Les clés ne sont jamais codées en dur.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{json, Map, Value};
use thiserror::Error;

/// Modèles par défaut (surchargés par `ORCHESTRA_MODEL`).
pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-opus-4-8";
pub const DEFAULT_GEMINI_MODEL: &str = "gemini-2.5-flash";

const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const GEMINI_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";
const MAX_OUTPUT_TOKENS: u32 = 8192;

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

/// Un backend LLM configuré : fournisseur + clé + modèle.
struct Backend {
    provider: Provider,
    api_key: String,
    model: String,
}

/// Client LLM avec **bascule automatique** de fournisseur.
///
/// Il essaie les backends dans l'ordre de préférence (par défaut Claude puis Gemini, selon les
/// clés présentes) et **bascule sur le suivant** si l'actuel est indisponible — réseau,
/// surcharge, quota, ou **crédit épuisé**. Un backend en échec *permanent* (clé invalide, plus
/// de crédit) est écarté pour les appels suivants. Le `reqwest::Client` est `Arc` en interne ;
/// l'état de bascule (`active`) vit derrière l'`Arc<LlmClient>` partagé par le runtime.
pub struct LlmClient {
    http: reqwest::Client,
    backends: Vec<Backend>,
    /// Index du premier backend à essayer (avance au-delà des backends morts).
    active: AtomicUsize,
}

impl LlmClient {
    /// Construit le client depuis l'environnement, ou `None` (→ mode simulé).
    ///
    /// - `ORCHESTRA_PROVIDER` (`anthropic`/`claude` ou `gemini`) force un fournisseur **unique** ;
    /// - sinon, on enregistre **tous** les fournisseurs dont la clé est présente — Claude en
    ///   préférence, Gemini en repli (bascule auto si Claude est indisponible ou sans crédit) ;
    /// - `ORCHESTRA_MODEL` surcharge le modèle du fournisseur principal.
    pub fn from_env() -> Option<Self> {
        let forced = std::env::var("ORCHESTRA_PROVIDER").ok().map(|p| p.trim().to_lowercase());
        let model_override = std::env::var("ORCHESTRA_MODEL").ok().filter(|m| !m.trim().is_empty());

        let mut backends: Vec<Backend> = Vec::new();
        match forced.as_deref() {
            Some("gemini") => push_backend(&mut backends, Provider::Gemini, "GEMINI_API_KEY", model_override.as_deref()),
            Some("anthropic") | Some("claude") => {
                push_backend(&mut backends, Provider::Anthropic, "ANTHROPIC_API_KEY", model_override.as_deref())
            }
            _ => {
                push_backend(&mut backends, Provider::Anthropic, "ANTHROPIC_API_KEY", model_override.as_deref());
                // Gemini en repli ; la surcharge de modèle ne vaut que pour le principal.
                let gem_override = if backends.is_empty() { model_override.as_deref() } else { None };
                push_backend(&mut backends, Provider::Gemini, "GEMINI_API_KEY", gem_override);
            }
        }
        if backends.is_empty() {
            return None;
        }

        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .build()
            .ok()?;
        Some(Self { http, backends, active: AtomicUsize::new(0) })
    }

    fn active_backend(&self) -> &Backend {
        let i = self.active.load(Ordering::Relaxed).min(self.backends.len() - 1);
        &self.backends[i]
    }

    pub fn provider(&self) -> Provider {
        self.active_backend().provider
    }
    pub fn model(&self) -> &str {
        &self.active_backend().model
    }

    /// Description lisible pour l'UI : fournisseur·modèle actif (+ repli éventuel).
    pub fn describe(&self) -> String {
        let active = self.active_backend();
        let mut s = format!("{} · {}", active.provider.label(), active.model);
        let fallbacks: Vec<&str> = self
            .backends
            .iter()
            .filter(|b| b.provider != active.provider)
            .map(|b| b.provider.label())
            .collect();
        if !fallbacks.is_empty() {
            s.push_str(&format!(" (repli : {})", fallbacks.join(", ")));
        }
        s
    }

    /// Un tour de conversation, avec **bascule automatique** de fournisseur en cas
    /// d'indisponibilité (réseau, surcharge, quota, crédit épuisé).
    pub async fn complete(
        &self,
        system: &str,
        tools: &[ToolSpec],
        conv: &[Msg],
    ) -> Result<Vec<Block>, LlmError> {
        let start = self.active.load(Ordering::Relaxed).min(self.backends.len() - 1);
        let mut last_err: Option<LlmError> = None;
        for i in start..self.backends.len() {
            match self.try_backend(&self.backends[i], system, tools, conv).await {
                Ok(blocks) => return Ok(blocks),
                Err(e) => {
                    // Échec permanent (clé invalide / plus de crédit) → on écarte ce backend.
                    if is_permanent(&e) {
                        self.active.store(i + 1, Ordering::Relaxed);
                    }
                    // Erreur non rattrapable par bascule (ex. requête malformée) → on remonte.
                    if !should_failover(&e) {
                        return Err(e);
                    }
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| LlmError::Shape("aucun fournisseur LLM disponible".into())))
    }

    /// Un appel à un backend précis.
    async fn try_backend(
        &self,
        backend: &Backend,
        system: &str,
        tools: &[ToolSpec],
        conv: &[Msg],
    ) -> Result<Vec<Block>, LlmError> {
        match backend.provider {
            Provider::Anthropic => {
                let body = anthropic_body(&backend.model, system, tools, conv);
                let resp = self
                    .http
                    .post(ANTHROPIC_URL)
                    .header("x-api-key", &backend.api_key)
                    .header("anthropic-version", ANTHROPIC_VERSION)
                    .json(&body)
                    .send()
                    .await?;
                parse_anthropic(&checked_json(resp).await?)
            }
            Provider::Gemini => {
                let url = format!("{GEMINI_BASE}/{}:generateContent", backend.model);
                let body = gemini_body(system, tools, conv);
                let resp = self
                    .http
                    .post(url)
                    .header("x-goog-api-key", &backend.api_key)
                    .json(&body)
                    .send()
                    .await?;
                parse_gemini(&checked_json(resp).await?)
            }
        }
    }
}

fn push_backend(backends: &mut Vec<Backend>, provider: Provider, key_var: &str, model_override: Option<&str>) {
    if let Some(k) = key(key_var) {
        let model = model_override.map(str::to_string).unwrap_or_else(|| default_model(provider));
        backends.push(Backend { provider, api_key: k, model });
    }
}

fn default_model(provider: Provider) -> String {
    match provider {
        Provider::Anthropic => DEFAULT_ANTHROPIC_MODEL.to_string(),
        Provider::Gemini => DEFAULT_GEMINI_MODEL.to_string(),
    }
}

/// Vrai si l'erreur justifie d'essayer le fournisseur suivant (réseau, surcharge, quota, crédit
/// épuisé, auth). Une requête malformée (400 hors facturation) n'est pas rattrapable par bascule.
fn should_failover(e: &LlmError) -> bool {
    match e {
        LlmError::Transport(_) => true,
        LlmError::Shape(_) => false,
        LlmError::Api { status, body } => {
            matches!(status, 401 | 402 | 403 | 429) || *status >= 500 || (*status == 400 && is_billing(body))
        }
    }
}

/// Vrai si l'échec est *permanent* pour ce fournisseur (clé invalide, plus de crédit) → on
/// l'écarte des appels suivants (inutile de le re-tenter à chaque tour).
fn is_permanent(e: &LlmError) -> bool {
    matches!(e, LlmError::Api { status, body }
        if matches!(status, 401..=403) || (*status == 400 && is_billing(body)))
}

/// Détecte un message d'erreur lié au crédit/quota/facturation (Anthropic renvoie un 400
/// « Your credit balance is too low » quand il n'y a plus de crédit).
fn is_billing(body: &str) -> bool {
    let b = body.to_lowercase();
    b.contains("credit") || b.contains("quota") || b.contains("billing") || b.contains("balance")
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
    // Prompt caching : le system prompt (projet + rôle + compétences + persona) est stable
    // d'un tour à l'autre et d'un agent à l'autre. On marque ce préfixe comme cacheable —
    // les requêtes suivantes paient une fraction des tokens d'entrée sur ce bloc.
    body.insert(
        "system".into(),
        json!([{ "type": "text", "text": system, "cache_control": { "type": "ephemeral" } }]),
    );
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
    // `thinkingBudget: 0` désactive le « raisonnement » interne de Gemini 2.5 (Flash) : sinon
    // ses tokens de réflexion consomment `maxOutputTokens` et la réponse peut revenir **sans
    // `parts`** (finishReason MAX_TOKENS) — ce qui rendait le LLM « injoignable ».
    body.insert(
        "generationConfig".into(),
        json!({ "maxOutputTokens": MAX_OUTPUT_TOKENS, "thinkingConfig": { "thinkingBudget": 0 } }),
    );
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
    let parts = match v.pointer("/candidates/0/content/parts").and_then(Value::as_array) {
        Some(parts) => parts,
        None => {
            // Pas de `parts` : on diagnostique via finishReason / blocage de sécurité plutôt que
            // d'échouer avec un message opaque.
            let finish = v
                .pointer("/candidates/0/finishReason")
                .and_then(Value::as_str)
                .unwrap_or("");
            let block = v
                .pointer("/promptFeedback/blockReason")
                .and_then(Value::as_str)
                .unwrap_or("");
            let msg = match (finish, block) {
                ("MAX_TOKENS", _) => {
                    "réponse tronquée par la limite de tokens (MAX_TOKENS) — aucun contenu renvoyé".to_string()
                }
                (_, b) if !b.is_empty() => format!("requête bloquée par Gemini (raison : {b})"),
                ("SAFETY" | "RECITATION", _) => {
                    format!("réponse bloquée par Gemini (finishReason : {finish})")
                }
                (f, _) if !f.is_empty() => format!("réponse sans contenu (finishReason : {f})"),
                _ => "aucun `candidates[0].content.parts`".to_string(),
            };
            return Err(LlmError::Shape(msg));
        }
    };
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

    #[test]
    fn failover_classification() {
        // Crédit épuisé (Anthropic renvoie un 400 avec « credit balance ») → bascule + permanent.
        let no_credit = LlmError::Api { status: 400, body: "Your credit balance is too low".into() };
        assert!(should_failover(&no_credit) && is_permanent(&no_credit));

        // Auth invalide → bascule + permanent.
        let auth = LlmError::Api { status: 401, body: "authentication_error".into() };
        assert!(should_failover(&auth) && is_permanent(&auth));

        // Rate limit → on bascule pour cet appel, mais pas permanent (on retentera le principal).
        let rate = LlmError::Api { status: 429, body: "rate_limit".into() };
        assert!(should_failover(&rate) && !is_permanent(&rate));

        // Réseau → bascule, non permanent.
        // (pas d'instance reqwest::Error simple à fabriquer ici ; couvert par le type)

        // Requête malformée (notre bug) → ni bascule ni permanent.
        let bad = LlmError::Api { status: 400, body: "tools.0.custom.name: invalid".into() };
        assert!(!should_failover(&bad) && !is_permanent(&bad));
    }

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
    fn anthropic_body_marks_system_prompt_cacheable() {
        let b = anthropic_body("claude-opus-4-8", "sys", &tools(), &sample_conv());
        // Le system est un tableau de blocs, avec un point de césure de cache éphémère.
        assert_eq!(b["system"][0]["text"], "sys");
        assert_eq!(b["system"][0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn gemini_body_shapes_tool_roundtrip() {
        let b = gemini_body("sys", &tools(), &sample_conv());
        assert_eq!(b["contents"][1]["parts"][0]["functionCall"]["name"], "Read_File");
        assert_eq!(b["contents"][2]["parts"][0]["functionResponse"]["name"], "Read_File");
        assert_eq!(b["tools"][0]["functionDeclarations"][0]["name"], "Read_File");
        assert!(b.get("systemInstruction").is_some());
        // Le « thinking » de Gemini 2.5 est désactivé pour ne pas épuiser le budget de sortie.
        assert_eq!(b["generationConfig"]["thinkingConfig"]["thinkingBudget"], 0);
    }

    #[test]
    fn parse_gemini_diagnoses_missing_parts() {
        // MAX_TOKENS sans contenu → message clair (au lieu du « aucun parts » opaque).
        let truncated = json!({ "candidates": [{ "finishReason": "MAX_TOKENS", "content": {} }] });
        let err = parse_gemini(&truncated).unwrap_err();
        assert!(matches!(&err, LlmError::Shape(m) if m.contains("MAX_TOKENS")));

        // Blocage de sécurité → mentionne la raison.
        let blocked = json!({ "promptFeedback": { "blockReason": "SAFETY" } });
        let err = parse_gemini(&blocked).unwrap_err();
        assert!(matches!(&err, LlmError::Shape(m) if m.contains("SAFETY")));

        // Réponse vide mais valide (parts = []) → pas d'erreur, aucun bloc.
        let empty = json!({ "candidates": [{ "content": { "parts": [] } }] });
        assert!(parse_gemini(&empty).unwrap().is_empty());
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
