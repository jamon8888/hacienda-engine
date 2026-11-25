//! Installation du viewer et du provider Codex locaux.
//!
//! Cette commande ajoute le provider Hacienda dans le `config.toml` utilisateur de
//! Codex **sans écraser les autres providers**, puis ouvre le viewer servi par
//! `hacienda serve --viewer-dir` sur la même origine que le gateway.
//!
//! Le secret local du gateway reste dans l'environnement (`HACIENDA_GATEWAY_TOKEN`) :
//! seul son *nom* est écrit dans le TOML via `env_http_headers`. `Authorization`
//! est réservé au token ChatGPT de l'utilisateur (`requires_openai_auth = true`)
//! et l'identifiant de compte vient de l'en-tête Codex, jamais du TOML ni du
//! navigateur.

use std::collections::BTreeMap;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    http::{header, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use clap::Parser;
use tokio::net::TcpListener;

const HACIENDA_PROVIDER: &str = "hacienda";
const DEFAULT_GATEWAY_URL: &str = "http://127.0.0.1:8787";

/// Dérive l'origine de l'API (hôte + port) depuis la base Responses `/v1` que
/// Codex utilise. Le viewer et les événements sont servis à cette origine, pas
/// sous le préfixe `/v1`.
pub fn api_origin_from_base_url(base_url: &str) -> &str {
    let trimmed = base_url.trim_end_matches('/');
    trimmed.strip_suffix("/v1").unwrap_or(trimmed)
}

/// Fichier de configuration utilisateur de Codex (défaut).
pub fn default_codex_config_path() -> PathBuf {
    dirs::home_dir()
        .map(|home| home.join(".codex").join("config.toml"))
        .unwrap_or_else(|| PathBuf::from(".codex/config.toml"))
}

/// Provider Hacienda fusionné dans la configuration utilisateur de Codex.
///
/// `requires_openai_auth = true` réserve `Authorization` au token ChatGPT de
/// l'utilisateur ; le secret local du gateway est transmis dans un en-tête
/// distinct dont la valeur est lue par Codex depuis l'environnement.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexProvider {
    pub name: String,
    pub base_url: String,
    pub wire_api: String,
    pub requires_openai_auth: bool,
    /// Nom de l'en-tête -> nom de la variable d'environnement.
    pub env_http_headers: BTreeMap<String, String>,
}

impl Default for CodexProvider {
    fn default() -> Self {
        Self {
            name: "Hacienda Responses Gateway".into(),
            base_url: format!("{DEFAULT_GATEWAY_URL}/v1"),
            wire_api: "responses".into(),
            requires_openai_auth: true,
            env_http_headers: BTreeMap::from([(
                "X-Hacienda-Gateway-Token".to_owned(),
                "HACIENDA_GATEWAY_TOKEN".to_owned(),
            )]),
        }
    }
}

/// Construit la valeur TOML du provider Hacienda à insérer dans la config.
fn provider_value(provider: &CodexProvider) -> toml::Value {
    let mut m = toml::map::Map::new();
    m.insert("name".into(), toml::Value::String(provider.name.clone()));
    m.insert(
        "base_url".into(),
        toml::Value::String(provider.base_url.clone()),
    );
    m.insert(
        "wire_api".into(),
        toml::Value::String(provider.wire_api.clone()),
    );
    m.insert(
        "requires_openai_auth".into(),
        toml::Value::Boolean(provider.requires_openai_auth),
    );
    m.insert(
        "env_http_headers".into(),
        toml::Value::Table(
            provider
                .env_http_headers
                .iter()
                .map(|(header, env)| (header.clone(), toml::Value::String(env.clone())))
                .collect(),
        ),
    );
    toml::Value::Table(m)
}

/// Fusionne le provider Hacienda dans un document TOML existant sans toucher aux
/// autres providers ni aux modifications utilisateur.
///
/// Toutes les tables et clés existantes sont conservées. Seule
/// `model_providers.hacienda` est ajoutée ou remplacée, et `profiles.hacienda` est
/// ajoutée quand elle est absente. La sérialisation re-formate le document (les
/// commentaires sont perdus) mais toutes les données utilisateur sont préservées.
pub fn merge_codex_provider(
    existing: &str,
    provider: &CodexProvider,
    add_profile: bool,
) -> Result<String> {
    let mut value: toml::Value = existing
        .parse()
        .with_context(|| "le config.toml de Codex existant n'est pas un TOML valide")?;
    let table = value
        .as_table_mut()
        .with_context(|| "le config doit être une table TOML")?;

    let providers = table
        .entry("model_providers")
        .or_insert_with(|| toml::Value::Table(Default::default()));
    let providers_table = providers
        .as_table_mut()
        .with_context(|| "model_providers doit être une table")?;
    providers_table.insert(HACIENDA_PROVIDER.to_string(), provider_value(provider));

    if add_profile {
        let profiles = table
            .entry("profiles")
            .or_insert_with(|| toml::Value::Table(Default::default()));
        let profiles_table = profiles
            .as_table_mut()
            .with_context(|| "profiles doit être une table")?;
        if !profiles_table.contains_key(HACIENDA_PROVIDER) {
            let mut profile = BTreeMap::new();
            profile.insert(
                "model_provider".to_string(),
                toml::Value::String("hacienda".into()),
            );
            profiles_table.insert(
                HACIENDA_PROVIDER.to_string(),
                toml::Value::Table(profile.into_iter().collect()),
            );
        }
    }

    Ok(toml::to_string_pretty(&value)?)
}

/// Écrit le document fusionné dans le fichier de config, en créant le fichier (et
/// ses répertoires parents) s'il est absent.
pub fn write_codex_config(path: &Path, document: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("création du répertoire {:?}", parent))?;
    }
    fs::write(path, document).with_context(|| format!("écriture de {:?}", path))?;
    Ok(())
}

// ─── Serveur viewer en boucle locale ───────────────────────────────────────

async fn serve_file(
    viewer_dir: Arc<PathBuf>,
    name: &'static str,
    content_type: &'static str,
) -> impl IntoResponse {
    match tokio::fs::read(viewer_dir.join(name)).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, content_type)], bytes).into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            format!("viewer introuvable: {name} ({e})"),
        )
            .into_response(),
    }
}

async fn index_handler(viewer_dir: Arc<PathBuf>) -> impl IntoResponse {
    serve_file(viewer_dir, "index.html", "text/html; charset=utf-8").await
}

async fn js_handler(viewer_dir: Arc<PathBuf>) -> impl IntoResponse {
    serve_file(
        viewer_dir,
        "viewer.js",
        "application/javascript; charset=utf-8",
    )
    .await
}

async fn css_handler(viewer_dir: Arc<PathBuf>) -> impl IntoResponse {
    serve_file(viewer_dir, "viewer.css", "text/css; charset=utf-8").await
}

/// Legacy standalone viewer server. The supported path now serves the assets
/// from the gateway router (`hacienda serve --viewer-dir`) so the SSE channel is
/// same-origin; kept as a helper for embedders that still need a bare static
/// file server.
#[allow(dead_code)]
pub async fn serve_viewer(viewer_dir: PathBuf, port: u16, should_open: bool) -> Result<SocketAddr> {
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .with_context(|| format!("liaison du viewer sur 127.0.0.1:{port}"))?;
    let addr = listener
        .local_addr()
        .with_context(|| "lecture de l'adresse liée du viewer")?;
    if should_open {
        open_browser(&format!("http://{}/", addr))?;
    }

    let dir = Arc::new(viewer_dir);
    let dir_root = Arc::clone(&dir);
    let dir_js = Arc::clone(&dir);
    let dir_css = Arc::clone(&dir);
    let app = Router::new()
        .route("/", get(move || index_handler(Arc::clone(&dir_root))))
        .route("/viewer.js", get(move || js_handler(Arc::clone(&dir_js))))
        .route(
            "/viewer.css",
            get(move || css_handler(Arc::clone(&dir_css))),
        );

    axum::serve(listener, app)
        .await
        .with_context(|| "serveur viewer arrêté")?;
    Ok(addr)
}

// ─── Ouverture du navigateur ───────────────────────────────────────────────

/// Ouvre `url` dans le navigateur par défaut de la plateforme.
pub fn open_browser(url: &str) -> Result<()> {
    if cfg!(target_os = "macos") {
        Command::new("open").arg(url).spawn()?;
    } else if cfg!(target_os = "linux") {
        Command::new("xdg-open").arg(url).spawn()?;
    } else if cfg!(windows) {
        Command::new("cmd").args(["/C", "start"]).arg(url).spawn()?;
    } else {
        anyhow::bail!("ouverture de navigateur non supportée sur cette plateforme");
    }
    Ok(())
}

// ─── Commande d'installation ───────────────────────────────────────────────

/// `hacienda codex install` — ajoute le provider Hacienda au config.toml de Codex
/// et ouvre le viewer servi en boucle locale par le gateway.
#[derive(Clone, Debug, Parser)]
pub struct CodexInstallArgs {
    /// Chemin du config.toml de Codex (défaut: ~/.codex/config.toml).
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// URL de base du gateway Hacienda (boucle locale uniquement).
    #[arg(long, default_value = DEFAULT_GATEWAY_URL, value_name = "URL")]
    pub gateway_url: String,

    /// Répertoire contenant les fichiers statiques du viewer (index.html, viewer.js, viewer.css).
    #[arg(long, value_name = "DIR", env = "HACIENDA_CODEX_VIEWER_DIR")]
    pub viewer_dir: Option<PathBuf>,

    /// Port d'écoute du viewer (0 = port dynamique choisi par le système).
    #[arg(long, default_value = "0", value_name = "PORT")]
    pub port: u16,

    /// Ne pas ouvrir le navigateur après avoir lancé le viewer.
    #[arg(long)]
    pub no_open: bool,
}

/// Exécute `hacienda codex install`.
pub async fn run_codex_install(args: CodexInstallArgs) -> Result<()> {
    // 1. Fusion du provider dans la config utilisateur : les autres providers et
    //    les modifications utilisateur sont préservés.
    let config_path = args.config.unwrap_or_else(default_codex_config_path);
    let existing = fs::read_to_string(&config_path).unwrap_or_default();
    let provider = CodexProvider {
        base_url: args.gateway_url,
        ..Default::default()
    };
    let merged = merge_codex_provider(&existing, &provider, true)?;
    write_codex_config(&config_path, &merged)?;
    println!(
        "Provider Hacienda ajouté à {:?} (autres providers préservés).",
        config_path
    );

    // 2. Le viewer est servi par `hacienda serve` sur la même origine que le
    //    gateway : aucun second port, aucun CORS et aucun secret côté navigateur.
    //    Cette commande ne démarre pas le gateway.
    let api_origin = api_origin_from_base_url(&provider.base_url);
    let viewer_url = format!("{api_origin}/viewer");
    match args.viewer_dir {
        Some(dir) => println!(
            "Gateway non démarré par cette commande. Lancez `hacienda serve --viewer-dir {}` puis ouvrez {viewer_url}.",
            dir.display()
        ),
        None => println!(
            "Gateway non démarré par cette commande. Lancez `hacienda serve --viewer-dir <dir>` puis ouvrez {viewer_url}."
        ),
    }
    if !args.no_open {
        open_browser(&viewer_url)?;
    }
    println!(
        "Provider Codex configuré sur {} (base Responses).",
        provider.base_url
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewer_origin_is_the_api_origin_not_the_responses_base() {
        assert_eq!(
            api_origin_from_base_url("http://127.0.0.1:8787/v1"),
            "http://127.0.0.1:8787"
        );
        assert_eq!(
            api_origin_from_base_url("http://127.0.0.1:8787/v1/"),
            "http://127.0.0.1:8787"
        );
        assert_eq!(
            api_origin_from_base_url("http://127.0.0.1:8787"),
            "http://127.0.0.1:8787"
        );
    }

    fn empty_config() -> String {
        String::new()
    }

    #[test]
    fn merge_adds_hacienda_provider_to_empty_config() {
        let provider = CodexProvider::default();
        let merged = merge_codex_provider(&empty_config(), &provider, true).unwrap();
        let parsed: toml::Value = merged.parse().unwrap();
        let providers = parsed.get("model_providers").unwrap().as_table().unwrap();
        let h = providers.get("hacienda").unwrap();
        assert_eq!(
            h.get("name").unwrap().as_str().unwrap(),
            "Hacienda Responses Gateway"
        );
        assert_eq!(h.get("wire_api").unwrap().as_str().unwrap(), "responses");
        assert_eq!(
            h.get("requires_openai_auth").unwrap().as_bool().unwrap(),
            true
        );
        assert!(h.get("env_key").is_none());
        let headers = h.get("env_http_headers").unwrap().as_table().unwrap();
        assert_eq!(
            headers
                .get("X-Hacienda-Gateway-Token")
                .unwrap()
                .as_str()
                .unwrap(),
            "HACIENDA_GATEWAY_TOKEN"
        );
        assert!(h
            .get("base_url")
            .unwrap()
            .as_str()
            .unwrap()
            .starts_with("http://127.0.0.1"));
        let profiles = parsed.get("profiles").unwrap().as_table().unwrap();
        assert_eq!(
            profiles
                .get("hacienda")
                .unwrap()
                .get("model_provider")
                .unwrap()
                .as_str()
                .unwrap(),
            "hacienda"
        );
    }

    #[test]
    fn merge_preserves_other_providers() {
        let existing = r#"
[model_providers.anthropic]
name = "Anthropic"
base_url = "https://api.anthropic.com"
wire_api = "messages"
env_key = "ANTHROPIC_API_KEY"

[model_providers.openai]
name = "OpenAI"
base_url = "https://api.openai.com/v1"
wire_api = "chat"
env_key = "OPENAI_API_KEY"
"#;
        let provider = CodexProvider::default();
        let merged = merge_codex_provider(existing, &provider, true).unwrap();
        let parsed: toml::Value = merged.parse().unwrap();
        let providers = parsed.get("model_providers").unwrap().as_table().unwrap();
        // Les providers existants sont intacts.
        assert_eq!(
            providers
                .get("anthropic")
                .unwrap()
                .get("name")
                .unwrap()
                .as_str()
                .unwrap(),
            "Anthropic"
        );
        assert_eq!(
            providers
                .get("openai")
                .unwrap()
                .get("name")
                .unwrap()
                .as_str()
                .unwrap(),
            "OpenAI"
        );
        // Hacienda a été ajouté.
        assert!(providers.contains_key("hacienda"));
    }

    #[test]
    fn merge_replaces_existing_hacienda_provider_idempotently() {
        let existing = r#"
[model_providers.hacienda]
name = "ancien"
base_url = "http://ancien.example/v1"
wire_api = "responses"
env_key = "OLD_TOKEN"

[profiles.hacienda]
model_provider = "hacienda"
"#;
        let provider = CodexProvider::default();
        let merged = merge_codex_provider(existing, &provider, true).unwrap();
        let parsed: toml::Value = merged.parse().unwrap();
        let providers = parsed.get("model_providers").unwrap().as_table().unwrap();
        let h = providers.get("hacienda").unwrap();
        assert_eq!(
            h.get("name").unwrap().as_str().unwrap(),
            "Hacienda Responses Gateway"
        );
        assert_eq!(
            h.get("requires_openai_auth").unwrap().as_bool().unwrap(),
            true
        );
        assert!(h.get("env_key").is_none());
        assert_eq!(
            h.get("env_http_headers")
                .unwrap()
                .as_table()
                .unwrap()
                .get("X-Hacienda-Gateway-Token")
                .unwrap()
                .as_str()
                .unwrap(),
            "HACIENDA_GATEWAY_TOKEN"
        );
        // Pas de doublon.
        let count = providers.keys().filter(|k| k == &"hacienda").count();
        assert_eq!(count, 1);
    }

    #[test]
    fn merge_does_not_overwrite_existing_profile() {
        let existing = r#"
[profiles.hacienda]
model_provider = "autre-provider"
"#;
        let provider = CodexProvider::default();
        let merged = merge_codex_provider(existing, &provider, true).unwrap();
        let parsed: toml::Value = merged.parse().unwrap();
        let profiles = parsed.get("profiles").unwrap().as_table().unwrap();
        // Le profil existant est préservé, pas écrasé.
        assert_eq!(
            profiles
                .get("hacienda")
                .unwrap()
                .get("model_provider")
                .unwrap()
                .as_str()
                .unwrap(),
            "autre-provider"
        );
    }

    #[test]
    fn merge_handles_invalid_toml() {
        let provider = CodexProvider::default();
        let result = merge_codex_provider("not valid {toml", &provider, true);
        assert!(result.is_err());
    }
}
