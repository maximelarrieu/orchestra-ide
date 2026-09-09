//! Clients LLM — Claude, Gemini **ou un modèle local via Ollama**, au choix, en HTTP brut.
//!
//! Rust n'a pas de SDK officiel pour ces fournisseurs : on appelle donc directement leurs
//! API REST via `reqwest`. Une représentation **neutre** ([`Msg`], [`Block`],
//! [`ToolSpec`]) découple la boucle agentique du format de chaque fournisseur ; chaque
//! provider sait *rendre* cette représentation dans son protocole et *parser* sa réponse.
//!
//! **Ollama ne demande aucune clé API** (serveur local, `http://localhost:11434` par
//! défaut) : `ORCHESTRA_PROVIDER=ollama` (ou `local`) l'utilise en fournisseur unique — voir
//! [`push_ollama_backend`]. Sans forçage explicite, Ollama ne rejoint la chaîne de repli
//! automatique (après Claude/Gemini) que si `ORCHESTRA_OLLAMA_MODEL` est défini, pour ne
//! jamais changer le comportement des installations qui n'ont pas Ollama.
//!
//! Le client est *optionnel* : sans clé API ni Ollama configuré, [`LlmClient::from_env`]
//! renvoie `None` et le runtime retombe sur les agents simulés. Les clés ne sont jamais
//! codées en dur.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{json, Map, Value};
use thiserror::Error;

/// Modèles par défaut (surchargés par `ORCHESTRA_MODEL`, ou `ORCHESTRA_OLLAMA_MODEL` pour Ollama).
pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-opus-4-8";
pub const DEFAULT_GEMINI_MODEL: &str = "gemini-2.5-flash";
/// Modèle Ollama par défaut : le plus orienté code du catalogue Ollama courant.
pub const DEFAULT_OLLAMA_MODEL: &str = "qwen2.5-coder";
/// Hôte Ollama par défaut (serveur local), surchargé par `ORCHESTRA_OLLAMA_HOST`.
pub const DEFAULT_OLLAMA_HOST: &str = "http://localhost:11434";

const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const GEMINI_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";
const MAX_OUTPUT_TOKENS: u32 = 8192;
/// Budget de réflexion Gemini 2.5 (borné pour laisser de la place à la réponse dans MAX_OUTPUT_TOKENS).
const THINKING_BUDGET: u32 = 2048;

/// Fournisseur d'IA disponible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Anthropic,
    Gemini,
    /// Modèle local servi par [Ollama](https://ollama.com) — aucune clé API, `/api/chat`.
    Ollama,
}

impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Provider::Anthropic => "Claude",
            Provider::Gemini => "Gemini",
            Provider::Ollama => "Ollama",
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

/// Un backend LLM configuré : fournisseur + clé + modèle. `api_key` est vide et `host` non
/// pertinent pour les fournisseurs cloud (Anthropic/Gemini, qui n'utilisent que la clé) ;
/// à l'inverse `host` porte l'URL du serveur et `api_key` reste vide pour Ollama (pas de clé).
struct Backend {
    provider: Provider,
    api_key: String,
    model: String,
    host: String,
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
    /// - `ORCHESTRA_PROVIDER` (`anthropic`/`claude`, `gemini`, ou `ollama`/`local`) force un
    ///   fournisseur **unique** — Ollama ainsi forcé ne demande **aucune clé API** ;
    /// - sinon, on enregistre **tous** les fournisseurs cloud dont la clé est présente — Claude
    ///   en préférence, Gemini en repli (bascule auto si Claude est indisponible ou sans crédit) —
    ///   et, **seulement si `ORCHESTRA_OLLAMA_MODEL` est défini**, Ollama comme repli local
    ///   supplémentaire (jamais ajouté par défaut, pour ne pas changer le comportement des
    ///   installations qui n'ont pas Ollama) ;
    /// - `ORCHESTRA_MODEL` surcharge le modèle du fournisseur principal ; `ORCHESTRA_OLLAMA_MODEL`
    ///   choisit le modèle local (ex. `qwen2.5-coder`, `mistral`, `gpt-oss`) et
    ///   `ORCHESTRA_OLLAMA_HOST` l'hôte du serveur Ollama (défaut `http://localhost:11434`).
    pub fn from_env() -> Option<Self> {
        let forced = std::env::var("ORCHESTRA_PROVIDER").ok().map(|p| p.trim().to_lowercase());
        let model_override = std::env::var("ORCHESTRA_MODEL").ok().filter(|m| !m.trim().is_empty());

        let mut backends: Vec<Backend> = Vec::new();
        match forced.as_deref() {
            Some("gemini") => push_backend(&mut backends, Provider::Gemini, "GEMINI_API_KEY", model_override.as_deref()),
            Some("anthropic") | Some("claude") => {
                push_backend(&mut backends, Provider::Anthropic, "ANTHROPIC_API_KEY", model_override.as_deref())
            }
            Some("ollama") | Some("local") => push_ollama_backend(&mut backends, model_override.as_deref()),
            _ => {
                push_backend(&mut backends, Provider::Anthropic, "ANTHROPIC_API_KEY", model_override.as_deref());
                // Gemini en repli ; la surcharge de modèle ne vaut que pour le principal.
                let gem_override = if backends.is_empty() { model_override.as_deref() } else { None };
                push_backend(&mut backends, Provider::Gemini, "GEMINI_API_KEY", gem_override);
                // Ollama en repli local optionnel : seulement si explicitement choisi.
                if ollama_model_env().is_some() {
                    push_ollama_backend(&mut backends, None);
                }
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
                // Gemini renvoie parfois `MALFORMED_FUNCTION_CALL` de façon transitoire : on
                // réessaie quelques fois (la sortie varie d'un appel à l'autre) avant d'abandonner.
                let mut last: Option<LlmError> = None;
                for attempt in 0..3 {
                    let resp = self
                        .http
                        .post(url.as_str())
                        .header("x-goog-api-key", &backend.api_key)
                        .json(&body)
                        .send()
                        .await?;
                    let v = checked_json(resp).await?;
                    match parse_gemini(&v) {
                        Ok(blocks) => return Ok(blocks),
                        Err(e) => {
                            let malformed = v
                                .pointer("/candidates/0/finishReason")
                                .and_then(Value::as_str)
                                == Some("MALFORMED_FUNCTION_CALL");
                            if malformed && attempt < 2 {
                                last = Some(e);
                                continue;
                            }
                            return Err(e);
                        }
                    }
                }
                Err(last.unwrap_or_else(|| LlmError::Shape("Gemini : échec après plusieurs tentatives".into())))
            }
            Provider::Ollama => {
                let url = format!("{}/api/chat", backend.host);
                let body = ollama_body(&backend.model, system, tools, conv);
                // Délai propre à Ollama, bien plus généreux que le défaut cloud (120 s) : un
                // modèle local (souvent CPU, voire un simple 7B) peut mettre plusieurs minutes à
                // répondre sur un tour avec beaucoup d'outils/contexte — un timeout serré s'y
                // manifeste comme une « erreur réseau » trompeuse (connexion coupée en plein
                // calcul), pas comme un vrai problème d'indisponibilité du serveur.
                let resp = self.http.post(&url).timeout(ollama_timeout()).json(&body).send().await?;
                parse_ollama(&checked_json(resp).await?)
            }
        }
    }
}

fn push_backend(backends: &mut Vec<Backend>, provider: Provider, key_var: &str, model_override: Option<&str>) {
    if let Some(k) = key(key_var) {
        let model = model_override.map(str::to_string).unwrap_or_else(|| default_model(provider));
        backends.push(Backend { provider, api_key: k, model, host: String::new() });
    }
}

/// Ajoute un backend Ollama — **jamais gardé par une clé** (serveur local). `model_override`
/// a priorité (ex. `ORCHESTRA_MODEL` quand Ollama est le fournisseur forcé), sinon
/// `ORCHESTRA_OLLAMA_MODEL`, sinon [`DEFAULT_OLLAMA_MODEL`].
fn push_ollama_backend(backends: &mut Vec<Backend>, model_override: Option<&str>) {
    let model = model_override
        .map(str::to_string)
        .or_else(ollama_model_env)
        .unwrap_or_else(|| DEFAULT_OLLAMA_MODEL.to_string());
    backends.push(Backend { provider: Provider::Ollama, api_key: String::new(), model, host: ollama_host() });
}

fn ollama_model_env() -> Option<String> {
    std::env::var("ORCHESTRA_OLLAMA_MODEL").ok().filter(|m| !m.trim().is_empty())
}

fn ollama_host() -> String {
    std::env::var("ORCHESTRA_OLLAMA_HOST")
        .ok()
        .map(|h| h.trim().trim_end_matches('/').to_string())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| DEFAULT_OLLAMA_HOST.to_string())
}

/// Délai max d'un appel Ollama. Par défaut **600 s** (l'inférence locale, souvent CPU, est
/// bien plus lente qu'une API cloud), surchargeable par `ORCHESTRA_OLLAMA_TIMEOUT_SECS`.
fn ollama_timeout() -> Duration {
    std::env::var("ORCHESTRA_OLLAMA_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&s| s > 0)
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(600))
}

fn default_model(provider: Provider) -> String {
    match provider {
        Provider::Anthropic => DEFAULT_ANTHROPIC_MODEL.to_string(),
        Provider::Gemini => DEFAULT_GEMINI_MODEL.to_string(),
        Provider::Ollama => DEFAULT_OLLAMA_MODEL.to_string(),
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
    // Budget de « réflexion » de Gemini 2.5 **borné** (pas 0) : un peu de raisonnement réduit les
    // appels d'outils mal formés (MALFORMED_FUNCTION_CALL), mais le plafond empêche la réflexion
    // de consommer tout `maxOutputTokens` et de renvoyer une réponse **sans `parts`** (MAX_TOKENS).
    body.insert(
        "generationConfig".into(),
        json!({ "maxOutputTokens": MAX_OUTPUT_TOKENS, "thinkingConfig": { "thinkingBudget": THINKING_BUDGET } }),
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
                ("MALFORMED_FUNCTION_CALL", _) => {
                    "Gemini a produit un appel d'outil mal formé (MALFORMED_FUNCTION_CALL) — réessaie, \
                     ou reformule ta demande".to_string()
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

// --- Rendu / parsing Ollama (local, /api/chat) ---------------------------------------

/// Rend la conversation au format `/api/chat` d'Ollama (proche d'OpenAI) : un message
/// `system`, puis un message par tour. Un [`Msg::Assistant`] est fusionné en **un seul**
/// message (texte concaténé + `tool_calls`) ; un [`Msg::Tool`] devient **un message `tool`
/// par résultat** (Ollama n'accepte pas de résultats groupés dans un seul message).
fn ollama_body(model: &str, system: &str, tools: &[ToolSpec], conv: &[Msg]) -> Value {
    let mut messages: Vec<Value> = vec![json!({ "role": "system", "content": system })];
    for m in conv {
        match m {
            Msg::User(t) => messages.push(json!({ "role": "user", "content": t })),
            Msg::Assistant(blocks) => {
                let mut text = String::new();
                let mut calls: Vec<Value> = Vec::new();
                for b in blocks {
                    match b {
                        Block::Text(t) => {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(t);
                        }
                        Block::ToolUse { name, input, .. } => {
                            calls.push(json!({ "function": { "name": name, "arguments": input } }))
                        }
                    }
                }
                let mut msg = Map::new();
                msg.insert("role".into(), json!("assistant"));
                msg.insert("content".into(), json!(text));
                if !calls.is_empty() {
                    msg.insert("tool_calls".into(), json!(calls));
                }
                messages.push(Value::Object(msg));
            }
            Msg::Tool(results) => {
                for r in results {
                    messages.push(json!({ "role": "tool", "content": r.content, "tool_name": r.name }));
                }
            }
        }
    }

    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    // Réponse complète en un seul JSON (pas de streaming NDJSON) : plus simple à parser, et le
    // reste du client (boucle agentique, événements) ne dépend pas du streaming.
    body.insert("stream".into(), json!(false));
    body.insert("messages".into(), json!(messages));
    if !tools.is_empty() {
        let defs: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": { "name": t.name, "description": t.description, "parameters": t.parameters }
                })
            })
            .collect();
        body.insert("tools".into(), json!(defs));
    }
    Value::Object(body)
}

fn parse_ollama(v: &Value) -> Result<Vec<Block>, LlmError> {
    let message = v.get("message").ok_or_else(|| LlmError::Shape("champ `message` absent".into()))?;
    let mut blocks = Vec::new();

    let native_calls = message.get("tool_calls").and_then(Value::as_array).filter(|c| !c.is_empty());

    if let Some(t) = message.get("content").and_then(Value::as_str).filter(|t| !t.is_empty()) {
        // Certains modèles (petits modèles locaux notamment) ne posent pas l'appel d'outil dans
        // `tool_calls` mais le « miment » en texte : la totalité du message est alors un objet
        // JSON `{"name": ..., "arguments": ...}` (parfois dans un bloc ```). On ne tente cette
        // récupération QUE si `tool_calls` est absent/vide et que le contenu, une fois débarrassé
        // d'un éventuel bloc de code, est ENTIÈREMENT ce JSON — jamais sur un texte narratif qui
        // contiendrait des accolades incidentes (on préfère rater un appel plutôt que d'exécuter
        // un outil sur un faux positif).
        match native_calls.is_none().then(|| fake_tool_call_from_text(t)).flatten() {
            Some((name, input)) => blocks.push(Block::ToolUse { id: format!("{name}-0"), name, input }),
            None => blocks.push(Block::Text(t.to_string())),
        }
    }

    if let Some(calls) = native_calls {
        for (i, c) in calls.iter().enumerate() {
            let f = c.get("function").cloned().unwrap_or_default();
            let name = f.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
            // Certains modèles renvoient `arguments` en chaîne JSON plutôt qu'en objet (comme
            // l'API OpenAI classique) : on gère les deux formes.
            let input = match f.get("arguments") {
                Some(Value::String(s)) => serde_json::from_str(s).unwrap_or_else(|_| json!({})),
                Some(other) => other.clone(),
                None => json!({}),
            };
            // Ollama n'attribue pas d'ID d'appel : on en synthétise un, comme pour Gemini.
            blocks.push(Block::ToolUse { id: format!("{name}-{i}"), name, input });
        }
    }
    Ok(blocks)
}

/// Détecte un appel d'outil « mimé » en texte par un modèle sans support natif fiable des
/// `tool_calls` : `content` (après un éventuel bloc ```/```json) est parsé comme JSON, et
/// accepté seulement s'il expose un champ `name` — chaîne de caractères — c'est-à-dire s'il a
/// vraiment la forme d'un appel d'outil, pas n'importe quel JSON.
fn fake_tool_call_from_text(content: &str) -> Option<(String, Value)> {
    let stripped = strip_code_fence(content.trim());
    let v: Value = serde_json::from_str(stripped).ok()?;
    let name = v.get("name")?.as_str()?.to_string();
    let input = v.get("arguments").cloned().unwrap_or_else(|| json!({}));
    Some((name, input))
}

/// Retire un bloc de code Markdown englobant (` ``` ` ou ` ```json `) s'il couvre tout le
/// texte ; sinon renvoie le texte tel quel.
fn strip_code_fence(s: &str) -> &str {
    let Some(rest) = s.strip_prefix("```") else { return s };
    let Some(rest) = rest.strip_suffix("```") else { return s };
    // Un éventuel identifiant de langage (`json`, `js`…) précède le premier saut de ligne.
    match rest.split_once('\n') {
        Some((lang, body)) if lang.chars().all(|c| c.is_ascii_alphanumeric()) => body.trim(),
        _ => rest.trim(),
    }
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
        // Le « thinking » de Gemini 2.5 est borné (pas d'épuisement du budget de sortie).
        assert_eq!(b["generationConfig"]["thinkingConfig"]["thinkingBudget"], 2048);
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

    #[test]
    fn ollama_body_shapes_tool_roundtrip() {
        let b = ollama_body("qwen2.5-coder", "sys", &tools(), &sample_conv());
        assert_eq!(b["model"], "qwen2.5-coder");
        assert_eq!(b["stream"], false);
        assert_eq!(b["messages"][0]["role"], "system");
        assert_eq!(b["messages"][1]["role"], "user");
        // Le tour assistant fusionne texte + tool_calls en UN seul message.
        assert_eq!(b["messages"][2]["role"], "assistant");
        assert_eq!(b["messages"][2]["tool_calls"][0]["function"]["name"], "Read_File");
        // Le résultat d'outil devient un message "tool" séparé.
        assert_eq!(b["messages"][3]["role"], "tool");
        assert_eq!(b["messages"][3]["content"], "contenu");
        assert_eq!(b["tools"][0]["type"], "function");
        assert_eq!(b["tools"][0]["function"]["name"], "Read_File");
    }

    #[test]
    fn parse_ollama_reads_text_and_tool_calls() {
        let v = json!({
            "message": {
                "role": "assistant",
                "content": "ok",
                "tool_calls": [{ "function": { "name": "Read_File", "arguments": { "path": "a" } } }]
            },
            "done": true
        });
        let blocks = parse_ollama(&v).unwrap();
        assert!(matches!(&blocks[0], Block::Text(t) if t == "ok"));
        assert!(matches!(&blocks[1], Block::ToolUse { name, .. } if name == "Read_File"));
    }

    #[test]
    fn parse_ollama_accepts_stringified_arguments() {
        // Certains modèles renvoient `arguments` en chaîne JSON plutôt qu'en objet.
        let v = json!({ "message": { "content": "", "tool_calls": [
            { "function": { "name": "Read_File", "arguments": "{\"path\":\"a\"}" } }
        ]}});
        let blocks = parse_ollama(&v).unwrap();
        assert!(matches!(&blocks[0], Block::ToolUse { input, .. } if input["path"] == "a"));
    }

    #[test]
    fn parse_ollama_empty_content_yields_no_text_block() {
        let v = json!({ "message": { "role": "assistant", "content": "" } });
        assert!(parse_ollama(&v).unwrap().is_empty());
    }

    #[test]
    fn parse_ollama_recovers_tool_call_faked_as_plain_text() {
        // Modèle local sans `tool_calls` fiable : il « mime » l'appel dans `content`, en JSON pur.
        let v = json!({ "message": { "role": "assistant",
            "content": "{\"name\": \"Recall\", \"arguments\": {\"query\": \"évolution projet\"}}"
        }});
        let blocks = parse_ollama(&v).unwrap();
        assert_eq!(blocks.len(), 1);
        assert!(matches!(&blocks[0],
            Block::ToolUse { name, input, .. } if name == "Recall" && input["query"] == "évolution projet"));
    }

    #[test]
    fn parse_ollama_recovers_fenced_json_tool_call() {
        let v = json!({ "message": { "role": "assistant",
            "content": "```json\n{\"name\": \"Read_File\", \"arguments\": {\"path\": \"README.md\"}}\n```"
        }});
        let blocks = parse_ollama(&v).unwrap();
        assert!(matches!(&blocks[0], Block::ToolUse { name, .. } if name == "Read_File"));
    }

    #[test]
    fn parse_ollama_keeps_narrative_text_with_braces_as_text() {
        // Ne doit JAMAIS être pris pour un appel d'outil : ce n'est pas un JSON valide dans son
        // ensemble (texte autour des accolades).
        let v = json!({ "message": { "role": "assistant",
            "content": "Le fichier contient une fonction `main() { ... }` assez simple."
        }});
        let blocks = parse_ollama(&v).unwrap();
        assert!(matches!(&blocks[0], Block::Text(_)));
    }

    #[test]
    fn parse_ollama_prefers_native_tool_calls_over_text_content() {
        // `tool_calls` natif présent : le contenu texte reste du texte, jamais réinterprété.
        let v = json!({ "message": { "role": "assistant",
            "content": "{\"name\": \"Recall\", \"arguments\": {}}",
            "tool_calls": [{ "function": { "name": "Read_File", "arguments": { "path": "a" } } }]
        }});
        let blocks = parse_ollama(&v).unwrap();
        assert!(matches!(&blocks[0], Block::Text(_)));
        assert!(matches!(&blocks[1], Block::ToolUse { name, .. } if name == "Read_File"));
    }
}
