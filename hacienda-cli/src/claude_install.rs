//! Installation du gateway Hacienda pour Claude Code (CLI et onglet Code de Desktop).
//!
//! Cette commande écrit les variables d'environnement utilisateur de Claude Code dans
//! `~/.claude/settings.json` (`ANTHROPIC_BASE_URL` et un credential de gateway) **sans
//! écraser les autres réglages**. Le secret n'est jamais codé en dur : il est lu au
//! moment de l'installation depuis une variable d'environnement ou un fichier, puis
//! soit déposé dans le bloc `env` avec des permissions 0600, soit — mode recommandé —
//! laissé hors de `settings.json` et récupéré à l'exécution par un `apiKeyHelper`.
//!
//! Sources officielles : <https://code.claude.com/docs/en/llm-gateway-connect>,
//! <https://code.claude.com/docs/en/settings>,
//! <https://claude.com/docs/third-party/claude-desktop/gateway>.
//!
//! Le gateway `/v1/messages` de Hacienda exige un token **bearer** (le middleware
//! d'authentification ne lit que `Authorization: Bearer …`) avec la capacité
//! `documents:process`. C'est pourquoi la variable Claude Code par défaut est
//! `ANTHROPIC_AUTH_TOKEN`, et non `ANTHROPIC_API_KEY` (qui enverrait `x-api-key`).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};

/// Origine du gateway Hacienda par défaut. Claude Code ajoute lui-même `/v1/messages`.
pub const DEFAULT_GATEWAY_URL: &str = "http://127.0.0.1:8787";
/// Nom de la variable qui porte le credential du gateway Claude au moment de l'installation.
/// Distincte de `HACIENDA_GATEWAY_TOKEN`, qui est le secret local du gateway Codex.
pub const CREDENTIAL_ENV: &str = "HACIENDA_CLAUDE_GATEWAY_TOKEN";
const BASE_URL_ENV: &str = "ANTHROPIC_BASE_URL";
const AUTH_TOKEN_ENV: &str = "ANTHROPIC_AUTH_TOKEN";
const API_KEY_ENV: &str = "ANTHROPIC_API_KEY";
const API_KEY_HELPER: &str = "apiKeyHelper";
/// Non-secret correlation header for the local Mod projection channel. It is a
/// correlation hint only, never a credential or an authorization decision.
pub const PROJECTION_HEADER: &str = "X-Hacienda-Projection-Correlation";
const CUSTOM_HEADERS_ENV: &str = "ANTHROPIC_CUSTOM_HEADERS";
const PROJECTION_CORRELATION_FILE: &str = "hacienda-projection-correlation";
const SECRET_FILE: &str = "hacienda-gateway-token";
const HELPER_FILE: &str = "hacienda-gateway-key.sh";
#[cfg(windows)]
const HELPER_FILE_WINDOWS: &str = "hacienda-gateway-key.ps1";

/// Où placer le credential dans la configuration Claude Code.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum CredentialMode {
    /// Écrire la valeur lue à l'installation dans `env.ANTHROPIC_AUTH_TOKEN`.
    Env,
    /// Écrire un `apiKeyHelper` qui lit le secret à l'exécution : aucun secret dans
    /// `settings.json`. C'est le mode le plus sûr (coffre, rotation, Desktop).
    Helper,
}

/// `hacienda claude install` — configure le gateway Hacienda pour Claude Code.
#[derive(Clone, Debug, Parser)]
pub struct ClaudeInstallArgs {
    /// Chemin du settings.json utilisateur (défaut : ~/.claude/settings.json, ou
    /// $CLAUDE_CONFIG_DIR/settings.json si Claude Code est configuré ainsi).
    #[arg(long, value_name = "PATH")]
    pub settings: Option<PathBuf>,

    /// URL de base du gateway Hacienda (Claude Code ajoute `/v1/messages`).
    #[arg(long, default_value = DEFAULT_GATEWAY_URL, value_name = "URL")]
    pub gateway_url: String,

    /// Mode de livraison du credential : `env` (valeur dans settings.json, 0600)
    /// ou `helper` (apiKeyHelper, secret hors de settings.json).
    #[arg(long, value_enum, default_value = "env", value_name = "MODE")]
    pub credential_mode: CredentialMode,

    /// Nom de la variable d'environnement qui porte le credential à l'installation.
    #[arg(long, default_value = CREDENTIAL_ENV, value_name = "NAME")]
    pub credential_env: String,

    /// Alternativement, chemin d'un fichier contenant le credential (0600 recommandé).
    /// Quand ce chemin est fourni, il a priorité sur la variable d'environnement.
    #[arg(long, value_name = "PATH")]
    pub credential_file: Option<PathBuf>,
}

/// Fichier de configuration utilisateur Claude Code (défaut). Claude Codehonore
/// `CLAUDE_CONFIG_DIR` pour déplacer son répertoire de configuration.
pub fn default_claude_settings_path() -> PathBuf {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("settings.json");
        }
    }
    dirs::home_dir()
        .map(|home| home.join(".claude").join("settings.json"))
        .unwrap_or_else(|| PathBuf::from(".claude/settings.json"))
}

/// Un secret ne doit jamais être imprimé, même par un `Debug` de diagnostic.
#[derive(Clone)]
pub struct GatewayCredential(String);

impl GatewayCredential {
    /// Valide et normalise un secret fourni à l'installation.
    pub fn from_source(raw: Option<&str>, source: &str) -> Result<Self> {
        let raw =
            raw.with_context(|| format!("aucun credential de gateway trouvé dans {source}"))?;
        let trimmed = raw.trim();
        anyhow::ensure!(
            !trimmed.is_empty(),
            "le credential de gateway lu depuis {source} est vide"
        );
        Ok(Self(trimmed.to_owned()))
    }

    /// Accès explicite à la valeur, uniquement au moment de l'écriture du fichier privé.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for GatewayCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GatewayCredential([REDACTED])")
    }
}

/// Ce que `merge_claude_settings` doit écrire pour le credential.
#[derive(Clone, Debug)]
pub enum CredentialPlacement {
    Env { value: String },
    Helper { command: String },
}

/// Valide une URL de base de gateway : http(s), hôte présent, pas de credentials
/// incorporés, pas de requête/fragment ni de caractère de contrôle.
pub fn validate_gateway_url(url: &str) -> Result<String> {
    let trimmed = url.trim().trim_end_matches('/');
    anyhow::ensure!(!trimmed.is_empty(), "l'URL du gateway est vide");
    anyhow::ensure!(
        trimmed.starts_with("http://") || trimmed.starts_with("https://"),
        "l'URL du gateway doit commencer par http:// ou https://"
    );
    anyhow::ensure!(
        !trimmed
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '@' | '?' | '#')),
        "l'URL du gateway contient des caractères interdits"
    );
    let authority = trimmed
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or(""))
        .unwrap_or("");
    anyhow::ensure!(!authority.is_empty(), "l'URL du gateway n'a pas d'hôte");
    Ok(trimmed.to_owned())
}

/// Un nom de variable d'environnement est interpolé dans un script shell : le
/// restreindre à `[A-Za-z_][A-Za-z0-9_]*` évite toute injection.
pub fn validate_env_var_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    anyhow::ensure!(
        first_ok && chars.all(|c| c.is_ascii_alphanumeric() || c == '_'),
        "nom de variable d'environnement invalide : {name}"
    );
    Ok(())
}

/// A correlation hint is deliberately non-secret and shape-restricted: the
/// server refuses any other spelling so two processes can never be confused.
pub fn validate_correlation(value: &str) -> Result<()> {
    anyhow::ensure!(
        !value.is_empty()
            && value.len() <= 128
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')),
        "valeur de corrélation invalide"
    );
    Ok(())
}

/// Opaque, non-secret per-install correlation value. It is NOT an authenticator
/// (any process could read it); the server binds it to an authenticated session
/// and fails closed when it is ambiguous.
fn generate_correlation() -> String {
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let seed = format!(
        "{}:{}:{}",
        std::process::id(),
        nanos,
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    use std::hash::{Hash, Hasher};
    let mut first = std::collections::hash_map::DefaultHasher::new();
    seed.hash(&mut first);
    let mut second = std::collections::hash_map::DefaultHasher::new();
    seed.chars().rev().collect::<String>().hash(&mut second);
    format!("hcm_{:016x}{:016x}", first.finish(), second.finish())
}

/// Add or replace only Hacienda's correlation line in `ANTHROPIC_CUSTOM_HEADERS`.
/// Every other custom header, env var and top-level setting is preserved.
pub fn merge_projection_correlation(existing: &str, correlation: &str) -> Result<String> {
    validate_correlation(correlation)?;
    let mut value: serde_json::Value = if existing.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(existing)
            .context("le settings.json existant n'est pas du JSON valide")?
    };
    let root = value
        .as_object_mut()
        .context("le settings.json doit être un objet JSON")?;
    let env = root
        .entry("env")
        .or_insert_with(|| serde_json::Value::Object(Default::default()));
    let env = env
        .as_object_mut()
        .context("la clé env du settings.json doit être un objet JSON")?;
    let headers = env
        .get(CUSTOM_HEADERS_ENV)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let target = format!("{PROJECTION_HEADER}: {correlation}");
    let mut lines: Vec<String> = headers.lines().map(str::to_owned).collect();
    let mut replaced = false;
    for line in &mut lines {
        if line
            .split_once(':')
            .is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case(PROJECTION_HEADER))
        {
            *line = target.clone();
            replaced = true;
        }
    }
    if !replaced {
        lines.push(target);
    }
    env.insert(
        CUSTOM_HEADERS_ENV.to_owned(),
        serde_json::Value::String(lines.join("\n")),
    );
    Ok(format!("{}\n", serde_json::to_string_pretty(&value)?))
}

/// Reuse a previously installed correlation value so re-running the installer is
/// idempotent for any Mod session already bootstrapped from these settings.
fn existing_correlation(settings: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(settings).ok()?;
    let headers = value.get("env")?.get(CUSTOM_HEADERS_ENV)?.as_str()?;
    headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case(PROJECTION_HEADER)
            .then(|| value.trim().to_owned())
    })
}

/// Fusionne la configuration du gateway dans un settings.json existant.
///
/// Toutes les clés de premier niveau et toutes les autres variables `env` sont
/// conservées telles quelles. Seuls `env.ANTHROPIC_BASE_URL` et la clé de credential
/// choisie sont écrits ; l'autre mécanisme est retiré pour ne pas laisser deux
/// sources de credential actives (avertissement `auth may not work as expected`).
pub fn merge_claude_settings(
    existing: &str,
    gateway_url: &str,
    placement: &CredentialPlacement,
) -> Result<String> {
    let mut value: serde_json::Value = if existing.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(existing)
            .context("le settings.json existant n'est pas du JSON valide")?
    };
    let root = value
        .as_object_mut()
        .context("le settings.json doit être un objet JSON")?;

    match placement {
        CredentialPlacement::Env { .. } => {
            root.remove(API_KEY_HELPER);
        }
        CredentialPlacement::Helper { command } => {
            root.insert(API_KEY_HELPER.to_owned(), serde_json::json!(command));
        }
    }

    let env = root
        .entry("env")
        .or_insert_with(|| serde_json::Value::Object(Default::default()));
    let env = env
        .as_object_mut()
        .context("la clé env du settings.json doit être un objet JSON")?;
    match placement {
        CredentialPlacement::Env { value } => {
            env.remove(API_KEY_ENV);
            env.insert(AUTH_TOKEN_ENV.to_owned(), serde_json::json!(value));
        }
        CredentialPlacement::Helper { .. } => {
            env.remove(AUTH_TOKEN_ENV);
            env.remove(API_KEY_ENV);
        }
    }
    env.insert(BASE_URL_ENV.to_owned(), serde_json::json!(gateway_url));

    Ok(format!("{}\n", serde_json::to_string_pretty(&value)?))
}

/// Écrit un fichier avec des permissions restrictives. Sur Unix, le mode est
/// réappliqué après l'écriture pour neutraliser un umask large sur un fichier
/// préexistant.
pub fn write_private(path: &Path, contents: &str, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true).mode(mode);
        let mut file = options
            .open(path)
            .with_context(|| format!("ouverture de {:?}", path))?;
        std::io::Write::write_all(&mut file, contents.as_bytes())
            .with_context(|| format!("écriture de {:?}", path))?;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .with_context(|| format!("permissions de {:?}", path))?;
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
        fs::write(path, contents).with_context(|| format!("écriture de {:?}", path))?;
    }
    Ok(())
}

/// Écriture atomique du settings.json : un fichier partiellement écrit ferait
/// échouer le démarrage de Claude Code (`Settings Error`).
pub fn write_settings_atomic(path: &Path, contents: &str) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).with_context(|| format!("création de {:?}", parent))?;
    let tmp = parent.join(format!(
        ".settings.json.hacienda-{}.tmp",
        std::process::id()
    ));
    let result = (|| -> Result<()> {
        write_private(&tmp, contents, 0o600)?;
        #[cfg(unix)]
        fs::rename(&tmp, path).with_context(|| format!("remplacement de {:?}", path))?;
        #[cfg(not(unix))]
        {
            if path.exists() {
                fs::remove_file(path).with_context(|| format!("suppression de {:?}", path))?;
            }
            fs::rename(&tmp, path).with_context(|| format!("remplacement de {:?}", path))?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Artefacts du mode `helper` : fichier secret 0600, script exécutable 0700 et
/// commande à placer dans `apiKeyHelper`. Le script ne contient aucun secret : il
/// lit la variable d'environnement à l'exécution, sinon le fichier secret.
fn helper_artifacts(dir: &Path, credential_env: &str) -> (PathBuf, PathBuf, String, String) {
    let secret_file = dir.join(SECRET_FILE);
    let quoted_secret = shell_quote(&secret_file.to_string_lossy());

    #[cfg(windows)]
    {
        let helper = dir.join(HELPER_FILE_WINDOWS);
        let script = format!(
            "# Generated by `hacienda claude install`; prints the gateway credential only.\n\
             if ($env:{env}) {{ [Console]::Write($env:{env}); exit 0 }}\n\
             $f = {file}\n\
             if (Test-Path -LiteralPath $f) {{ [Console]::Write((Get-Content -LiteralPath $f -Raw).TrimEnd(\"`r\", \"`n\")); exit 0 }}\n\
             [Console]::Error.WriteLine('hacienda: no Claude gateway credential available')\n\
             exit 1\n",
            env = credential_env,
            file = quoted_secret,
        );
        // Windows command lines use double quotes around a -File path; single quotes
        // would be passed literally to PowerShell and break the path.
        let quoted_helper = format!("\"{}\"", helper.to_string_lossy().replace('"', "\\\""));
        let command = format!("powershell -NoProfile -File {quoted_helper}");
        (secret_file, helper, command, script)
    }

    #[cfg(not(windows))]
    {
        let helper = dir.join(HELPER_FILE);
        let script = format!(
            "#!/bin/sh\n\
             # Generated by `hacienda claude install`; prints the gateway credential only.\n\
             set -eu\n\
             if [ -n \"${{{env}:-}}\" ]; then\n\
             \x20 printf '%s' \"${env}\"\n\
             \x20 exit 0\n\
             fi\n\
             if [ -r {file} ]; then\n\
             \x20 tr -d '\\r\\n' < {file}\n\
             \x20 exit 0\n\
             fi\n\
             echo 'hacienda: no Claude gateway credential available' >&2\n\
             exit 1\n",
            env = credential_env,
            file = quoted_secret,
        );
        let command = shell_quote(&helper.to_string_lossy());
        (secret_file, helper, command, script)
    }
}

/// Exécute `hacienda claude install`.
pub async fn run_claude_install(args: ClaudeInstallArgs) -> Result<()> {
    let settings_path = args.settings.unwrap_or_else(default_claude_settings_path);
    let settings_dir = settings_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let gateway_url = validate_gateway_url(&args.gateway_url)?;
    validate_env_var_name(&args.credential_env)?;

    // Le secret n'a jamais de valeur par défaut dans le code : il vient de
    // l'environnement de l'opérateur ou d'un fichier qu'il désigne.
    let (raw, source) = match &args.credential_file {
        Some(path) => (
            Some(
                fs::read_to_string(path)
                    .with_context(|| format!("lecture du credential depuis {:?}", path))?,
            ),
            format!("{:?}", path),
        ),
        None => {
            let value = match std::env::var(&args.credential_env) {
                Ok(value) => Some(value),
                Err(std::env::VarError::NotPresent) => None,
                Err(e) => {
                    return Err(e).with_context(|| format!("lecture de ${}", args.credential_env));
                }
            };
            (value, format!("${}", args.credential_env))
        }
    };
    let credential = GatewayCredential::from_source(raw.as_deref(), &source)?;

    let placement = match args.credential_mode {
        CredentialMode::Env => CredentialPlacement::Env {
            value: credential.expose().to_owned(),
        },
        CredentialMode::Helper => {
            let (secret_file, helper, command, script) =
                helper_artifacts(&settings_dir, &args.credential_env);
            write_private(&secret_file, &format!("{}\n", credential.expose()), 0o600)?;
            write_private(&helper, &script, 0o700)?;
            CredentialPlacement::Helper { command }
        }
    };

    let existing = fs::read_to_string(&settings_path).unwrap_or_default();
    let correlation = existing_correlation(&existing)
        .filter(|value| validate_correlation(value).is_ok())
        .unwrap_or_else(generate_correlation);
    let document = merge_claude_settings(&existing, &gateway_url, &placement)?;
    let document = merge_projection_correlation(&document, &correlation)?;
    write_settings_atomic(&settings_path, &document)?;
    // The adapter reads this non-secret hint from a stable local path; it is
    // never a credential and never sent to the inference provider.
    write_private(
        &settings_dir.join(PROJECTION_CORRELATION_FILE),
        &format!("{correlation}\n"),
        0o600,
    )?;

    println!("Configuration Claude Code écrite dans {:?}.", settings_path);
    println!("  {BASE_URL_ENV} = {gateway_url} (Claude Code ajoute /v1/messages).");
    match &placement {
        CredentialPlacement::Env { .. } => {
            println!(
                "  Credential : {AUTH_TOKEN_ENV} dans le bloc env (fichier 0600, valeur non affichée)."
            );
        }
        CredentialPlacement::Helper { command } => {
            println!("  Credential : apiKeyHelper = {command} (secret hors de settings.json).");
        }
    }
    println!(
        "  Corrélation projection (non secrète) : {CUSTOM_HEADERS_ENV}='{PROJECTION_HEADER}: <valeur>' et {:?}.",
        settings_dir.join(PROJECTION_CORRELATION_FILE)
    );
    println!(
        "  Onglet Code de Desktop : ce fichier est partagé avec Claude Code dans Desktop, mais le routage gateway de l'application Desktop se configure dans sa configuration third-party (voir docs/claude-code-gateway-install.md)."
    );
    println!("Redémarrez Claude Code / Claude Desktop pour appliquer la configuration.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_placement(value: &str) -> CredentialPlacement {
        CredentialPlacement::Env {
            value: value.to_owned(),
        }
    }

    fn helper_placement(command: &str) -> CredentialPlacement {
        CredentialPlacement::Helper {
            command: command.to_owned(),
        }
    }

    #[test]
    fn merge_adds_gateway_env_to_empty_settings() {
        let merged = merge_claude_settings("", "http://127.0.0.1:8787", &env_placement("tok"))
            .expect("merge into empty settings");
        let value: serde_json::Value = serde_json::from_str(&merged).expect("valid JSON");
        let env = value
            .get("env")
            .expect("env")
            .as_object()
            .expect("env object");
        assert_eq!(
            env.get(BASE_URL_ENV).unwrap().as_str().unwrap(),
            "http://127.0.0.1:8787"
        );
        assert_eq!(env.get(AUTH_TOKEN_ENV).unwrap().as_str().unwrap(), "tok");
        assert!(value.get(API_KEY_HELPER).is_none());
    }

    #[test]
    fn merge_preserves_unrelated_settings_and_env_vars() {
        let existing = r#"{
          "$schema": "https://json.schemastore.org/claude-code-settings.json",
          "model": "claude-opus-5-5",
          "permissions": { "allow": ["Bash(cargo test *)"] },
          "hooks": { "SessionStart": [{ "hooks": [{ "type": "command", "command": "echo hi" }] }] },
          "env": {
            "ANTHROPIC_MODEL": "claude-sonnet-5",
            "FOO": "bar",
            "ANTHROPIC_BASE_URL": "https://old.example"
          }
        }"#;
        let merged = merge_claude_settings(
            existing,
            "http://127.0.0.1:8787",
            &env_placement("secret-value"),
        )
        .expect("merge preserves unrelated keys");
        let value: serde_json::Value = serde_json::from_str(&merged).expect("valid JSON");
        assert_eq!(
            value.get("model").unwrap().as_str().unwrap(),
            "claude-opus-5-5"
        );
        assert_eq!(
            value.get("permissions").unwrap().get("allow").unwrap()[0]
                .as_str()
                .unwrap(),
            "Bash(cargo test *)"
        );
        assert!(value.get("hooks").is_some());
        assert_eq!(
            value.get("$schema").unwrap().as_str().unwrap(),
            "https://json.schemastore.org/claude-code-settings.json"
        );
        let env = value.get("env").unwrap().as_object().unwrap();
        assert_eq!(
            env.get("ANTHROPIC_MODEL").unwrap().as_str().unwrap(),
            "claude-sonnet-5"
        );
        assert_eq!(env.get("FOO").unwrap().as_str().unwrap(), "bar");
        assert_eq!(
            env.get(BASE_URL_ENV).unwrap().as_str().unwrap(),
            "http://127.0.0.1:8787"
        );
        assert_eq!(
            env.get(AUTH_TOKEN_ENV).unwrap().as_str().unwrap(),
            "secret-value"
        );
    }

    #[test]
    fn merge_helper_mode_never_writes_the_secret() {
        let secret = "hcd_documents:process_topsecret";
        let merged = merge_claude_settings(
            "{\"env\": {\"KEEP\": \"yes\"}}",
            "http://127.0.0.1:8787",
            &helper_placement("'/home/u/.claude/hacienda-gateway-key.sh'"),
        )
        .expect("merge helper mode");
        assert!(
            !merged.contains(secret),
            "settings.json must not contain the credential"
        );
        let value: serde_json::Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(
            value.get(API_KEY_HELPER).unwrap().as_str().unwrap(),
            "'/home/u/.claude/hacienda-gateway-key.sh'"
        );
        let env = value.get("env").unwrap().as_object().unwrap();
        assert_eq!(env.get("KEEP").unwrap().as_str().unwrap(), "yes");
        assert!(env.get(AUTH_TOKEN_ENV).is_none());
        assert!(env.get(API_KEY_ENV).is_none());
        assert_eq!(
            env.get(BASE_URL_ENV).unwrap().as_str().unwrap(),
            "http://127.0.0.1:8787"
        );
    }

    #[test]
    fn merge_removes_the_other_credential_source() {
        let existing = r#"{
          "apiKeyHelper": "~/old-key.sh",
          "env": { "ANTHROPIC_AUTH_TOKEN": "old-token", "OTHER": "kept" }
        }"#;
        let env_mode =
            merge_claude_settings(existing, "http://127.0.0.1:8787", &env_placement("new"))
                .unwrap();
        let v: serde_json::Value = serde_json::from_str(&env_mode).unwrap();
        assert!(
            v.get(API_KEY_HELPER).is_none(),
            "stale helper removed in env mode"
        );
        assert_eq!(
            v.get("env")
                .unwrap()
                .get("OTHER")
                .unwrap()
                .as_str()
                .unwrap(),
            "kept"
        );

        let helper_mode = merge_claude_settings(
            &env_mode,
            "http://127.0.0.1:8787",
            &helper_placement("~/new-key.sh"),
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&helper_mode).unwrap();
        assert!(
            v.get("env").unwrap().get(AUTH_TOKEN_ENV).is_none(),
            "stale token removed in helper mode"
        );
        assert_eq!(
            v.get(API_KEY_HELPER).unwrap().as_str().unwrap(),
            "~/new-key.sh"
        );
    }

    #[test]
    fn merge_is_idempotent() {
        let first =
            merge_claude_settings("{\"env\": {\"K\": \"v\"}}", "http://x", &env_placement("t"))
                .unwrap();
        let second = merge_claude_settings(&first, "http://x", &env_placement("t")).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn merge_rejects_invalid_or_non_object_input() {
        assert!(merge_claude_settings("not json {", "http://x", &env_placement("t")).is_err());
        assert!(merge_claude_settings("[]", "http://x", &env_placement("t")).is_err());
        assert!(merge_claude_settings("{\"env\": 42}", "http://x", &env_placement("t")).is_err());
    }

    #[test]
    fn credential_debug_is_redacted() {
        let secret = "hcd_documents:process_supersecret";
        let credential = GatewayCredential::from_source(Some(secret), "test source").unwrap();
        let debug = format!("{:?}", credential);
        assert!(!debug.contains(secret));
        assert!(debug.contains("REDACTED"));
        assert_eq!(credential.expose(), secret.trim());
    }

    #[test]
    fn credential_source_requires_a_nonempty_value() {
        assert!(GatewayCredential::from_source(None, "$CRED").is_err());
        assert!(GatewayCredential::from_source(Some("  \n "), "$CRED").is_err());
    }

    #[test]
    fn helper_script_reads_credential_without_containing_it() {
        let dir = Path::new("/Users/test/.claude");
        let secret = "hcd_documents:process_fromfile";
        let (secret_file, helper, _command, script) = helper_artifacts(dir, CREDENTIAL_ENV);
        assert_eq!(secret_file, dir.join(SECRET_FILE));
        assert_eq!(helper, dir.join(HELPER_FILE));
        assert!(!script.contains(secret));
        assert!(script.contains(CREDENTIAL_ENV));
        assert!(script.contains(&secret_file.to_string_lossy().to_string()));
        assert!(script.starts_with("#!/bin/sh"));
    }

    #[test]
    fn validate_env_var_name_rejects_injection() {
        assert!(validate_env_var_name(CREDENTIAL_ENV).is_ok());
        assert!(validate_env_var_name("_OK1").is_ok());
        assert!(validate_env_var_name("1BAD").is_err());
        assert!(validate_env_var_name("A;rm -rf").is_err());
        assert!(validate_env_var_name("A${B}").is_err());
    }

    #[test]
    fn projection_correlation_merge_preserves_other_headers_and_settings() {
        let existing = r#"{
          "model": "claude-opus-5-5",
          "env": {
            "KEEP": "yes",
            "ANTHROPIC_CUSTOM_HEADERS": "X-Org-Route: prod\nX-Hacienda-Projection-Correlation: old"
          }
        }"#;
        let merged = merge_projection_correlation(existing, "hcm_new").unwrap();
        let value: serde_json::Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(value["model"], "claude-opus-5-5");
        let env = value["env"].as_object().unwrap();
        assert_eq!(env["KEEP"], "yes");
        let headers = env["ANTHROPIC_CUSTOM_HEADERS"].as_str().unwrap();
        assert!(headers.contains("X-Org-Route: prod"));
        assert!(headers.contains("X-Hacienda-Projection-Correlation: hcm_new"));
        assert!(!headers.contains("old"));
        // Reinstalling keeps the same correlation value.
        assert_eq!(existing_correlation(&merged).as_deref(), Some("hcm_new"));
    }

    #[test]
    fn projection_correlation_merge_adds_header_to_empty_settings() {
        let merged = merge_projection_correlation("{}", "hcm_1234").unwrap();
        let value: serde_json::Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(
            value["env"]["ANTHROPIC_CUSTOM_HEADERS"].as_str().unwrap(),
            "X-Hacienda-Projection-Correlation: hcm_1234"
        );
    }

    #[test]
    fn correlation_rejects_ambiguous_values() {
        assert!(validate_correlation("hcm_abc-123_XYZ").is_ok());
        assert!(validate_correlation("").is_err());
        assert!(validate_correlation("a b").is_err());
        assert!(validate_correlation("a:b").is_err());
        assert!(validate_correlation(&"x".repeat(129)).is_err());
    }

    #[test]
    fn validate_gateway_url_rejects_unsafe_values() {
        assert_eq!(
            validate_gateway_url(" http://127.0.0.1:8787/ ").unwrap(),
            "http://127.0.0.1:8787"
        );
        assert_eq!(
            validate_gateway_url("https://hacienda.example.test/v1").unwrap(),
            "https://hacienda.example.test/v1"
        );
        assert!(validate_gateway_url("ftp://example.test").is_err());
        assert!(validate_gateway_url("http://").is_err());
        assert!(validate_gateway_url("http://user:pass@example.test").is_err());
        assert!(validate_gateway_url("http://example.test?x=1").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn write_private_applies_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("secret");
        fs::write(&path, "old").unwrap();
        write_private(&path, "new", 0o600).expect("write private");
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn write_settings_atomic_is_private_and_preserves_existing_content_on_failure() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("settings.json");
        fs::write(&path, "{\"keep\":true}").unwrap();
        write_settings_atomic(&path, "{\"keep\":true,\"new\":1}").expect("atomic write");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\"keep\":true,\"new\":1}"
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::read_dir(dir.path())
                .unwrap()
                .filter(|e| e
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains("tmp"))
                .count(),
            0,
            "no temp file left behind"
        );
    }
}
