use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use super::jwks::{Algorithm, JwksClient, JwksError, decode_base64url};
use crate::audit::{PrincipalKind, sha256_hex};
use crate::config::{AuthSettings, Mode, OauthSettings, Settings};
use crate::error::{Error, Result};
use crate::server::Principal;
use crate::tool_specs::{
    SCOPE_DDL, SCOPE_HOST, SCOPE_MAINTENANCE, SCOPE_READ, SCOPE_ROLES, SCOPE_WRITE,
};

pub const ALL_SCOPES: [&str; 6] = [
    SCOPE_READ,
    SCOPE_WRITE,
    SCOPE_DDL,
    SCOPE_ROLES,
    SCOPE_MAINTENANCE,
    SCOPE_HOST,
];
pub const LEEWAY_SECONDS: u64 = 30;
pub const MAX_TOKEN_BYTES: usize = 8 * 1024;

#[must_use]
pub fn scopes_for_mode(mode: Mode) -> Vec<String> {
    let scopes: &[&str] = match mode {
        Mode::ReadOnly => &[SCOPE_READ],
        Mode::WriteOnly => &[
            SCOPE_WRITE,
            SCOPE_DDL,
            SCOPE_ROLES,
            SCOPE_MAINTENANCE,
            SCOPE_HOST,
        ],
        Mode::ReadWrite => &ALL_SCOPES,
    };
    scopes.iter().map(|scope| (*scope).to_owned()).collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    pub status: u16,
    pub error: &'static str,
    pub description: String,
    pub scope: Option<String>,
}

impl Rejection {
    fn unauthorized(description: impl Into<String>) -> Self {
        Self {
            status: 401,
            error: "invalid_token",
            description: description.into(),
            scope: None,
        }
    }

    fn missing() -> Self {
        Self {
            status: 401,
            error: "invalid_request",
            description: "a bearer token is required".to_owned(),
            scope: None,
        }
    }

    #[must_use]
    pub fn insufficient(scope: &str) -> Self {
        Self {
            status: 403,
            error: "insufficient_scope",
            description: format!("the token does not carry the {scope} scope"),
            scope: Some(scope.to_owned()),
        }
    }
}

pub struct BearerToken {
    digest: [u8; 32],
    name: String,
    scopes: Vec<String>,
    fingerprint: String,
}

fn digest_of(secret: &[u8]) -> [u8; 32] {
    Sha256::digest(secret).into()
}

impl std::fmt::Debug for BearerToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BearerToken")
            .field("name", &self.name)
            .field("scopes", &self.scopes)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
pub struct BearerTokens {
    tokens: Vec<BearerToken>,
}

fn parse_scopes(text: &str) -> Option<Vec<String>> {
    if let Some(mode) = Mode::parse(text) {
        return Some(scopes_for_mode(mode));
    }
    let scopes: Vec<String> = text
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect();
    if scopes.is_empty()
        || scopes
            .iter()
            .any(|scope| !ALL_SCOPES.contains(&scope.as_str()))
    {
        return None;
    }
    Some(scopes)
}

impl BearerTokens {
    pub fn parse(text: &str, source: &str) -> Result<Self> {
        Self::parse_labeled(text, source, "token")
    }

    fn parse_labeled(text: &str, source: &str, label: &str) -> Result<Self> {
        let mut tokens: Vec<BearerToken> = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split_whitespace();
            let secret = parts.next().unwrap_or_default();
            let grant = parts.next().unwrap_or("read-only");
            let name = parts
                .next()
                .map_or_else(|| format!("{label}-{}", index + 1), str::to_owned);
            if tokens.iter().any(|known| known.name == name) {
                return Err(Error::ConfigInvalid {
                    setting: source.to_owned(),
                    value: name,
                    detail: "two bearer tokens carry the same name; names must be distinct because handles and audit lines are attributed by name".to_owned(),
                });
            }
            let digest = digest_of(secret.as_bytes());
            if tokens.iter().any(|known| known.digest == digest) {
                return Err(Error::ConfigInvalid {
                    setting: source.to_owned(),
                    value: format!("line {}", index + 1),
                    detail: "the same bearer token appears twice".to_owned(),
                });
            }
            if secret.len() < 16 || secret.len() > MAX_TOKEN_BYTES {
                return Err(Error::ConfigInvalid {
                    setting: source.to_owned(),
                    value: format!("line {}", index + 1),
                    detail: "a bearer token needs at least 16 characters".to_owned(),
                });
            }
            let Some(scopes) = parse_scopes(grant) else {
                return Err(Error::ConfigInvalid {
                    setting: source.to_owned(),
                    value: grant.to_owned(),
                    detail: "the second column is read-only, write-only, read-write, or a comma list of ownpg:* scopes".to_owned(),
                });
            };
            tokens.push(BearerToken {
                fingerprint: sha256_hex(secret.as_bytes()).chars().take(16).collect(),
                digest,
                name,
                scopes,
            });
        }
        if tokens.is_empty() {
            return Err(Error::ConfigInvalid {
                setting: source.to_owned(),
                value: String::new(),
                detail: "no bearer token was found; one token per line as `<token> <mode> [name]`"
                    .to_owned(),
            });
        }
        Ok(Self { tokens })
    }

    pub fn from_file(path: &Path) -> Result<Self> {
        crate::config::profile::refuse_open_permissions(path)?;
        let text = std::fs::read_to_string(path).map_err(|source| Error::ConfigUnreadable {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse_labeled(&text, &path.display().to_string(), "file")
    }

    fn merge(&mut self, other: Self, source: &str) -> Result<()> {
        for token in other.tokens {
            if self.tokens.iter().any(|known| known.name == token.name) {
                return Err(Error::ConfigInvalid {
                    setting: source.to_owned(),
                    value: token.name,
                    detail:
                        "a bearer token with this name is already configured from another source"
                            .to_owned(),
                });
            }
            if self.tokens.iter().any(|known| known.digest == token.digest) {
                return Err(Error::ConfigInvalid {
                    setting: source.to_owned(),
                    value: token.name,
                    detail: "this bearer token is already configured from another source"
                        .to_owned(),
                });
            }
            self.tokens.push(token);
        }
        Ok(())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    fn lookup(&self, presented: &[u8]) -> Option<&BearerToken> {
        let digest = digest_of(presented);
        let mut found = None;
        for token in &self.tokens {
            if bool::from(token.digest.ct_eq(&digest)) {
                found = Some(token);
            }
        }
        found
    }
}

#[derive(Debug, Deserialize)]
struct Header {
    alg: String,
    #[serde(default)]
    kid: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Claims {
    #[serde(default)]
    iss: Option<String>,
    #[serde(default)]
    aud: Option<serde_json::Value>,
    #[serde(default)]
    exp: Option<u64>,
    #[serde(default)]
    nbf: Option<u64>,
    #[serde(default)]
    sub: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    scp: Option<serde_json::Value>,
}

impl Claims {
    fn principal_name(&self) -> Option<String> {
        self.sub
            .as_deref()
            .filter(|sub| !sub.trim().is_empty())
            .map(|sub| format!("sub:{sub}"))
            .or_else(|| {
                self.client_id
                    .as_deref()
                    .filter(|id| !id.trim().is_empty())
                    .map(|id| format!("client:{id}"))
            })
    }

    fn audiences(&self) -> Vec<String> {
        match &self.aud {
            Some(serde_json::Value::String(text)) => vec![text.clone()],
            Some(serde_json::Value::Array(items)) => items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::to_owned)
                .collect(),
            _ => Vec::new(),
        }
    }

    fn scopes(&self) -> Vec<String> {
        let mut scopes: Vec<String> = self
            .scope
            .as_deref()
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        match &self.scp {
            Some(serde_json::Value::Array(items)) => {
                scopes.extend(
                    items
                        .iter()
                        .filter_map(|item| item.as_str())
                        .map(str::to_owned),
                );
            }
            Some(serde_json::Value::String(text)) => {
                scopes.extend(text.split_whitespace().map(str::to_owned));
            }
            _ => {}
        }
        scopes.sort();
        scopes.dedup();
        scopes
    }
}

pub struct Oauth {
    pub settings: OauthSettings,
    pub jwks: Arc<JwksClient>,
}

impl std::fmt::Debug for Oauth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Oauth")
            .field("issuer", &self.settings.issuer.value)
            .finish_non_exhaustive()
    }
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

pub fn principal_from_claims(
    claims: &Claims,
    issuer: &str,
    audience: &str,
    now: u64,
) -> std::result::Result<String, Rejection> {
    let Some(exp) = claims.exp else {
        return Err(Rejection::unauthorized("the token carries no expiry"));
    };
    if now > exp.saturating_add(LEEWAY_SECONDS) {
        return Err(Rejection::unauthorized("the token has expired"));
    }
    if let Some(nbf) = claims.nbf
        && now.saturating_add(LEEWAY_SECONDS) < nbf
    {
        return Err(Rejection::unauthorized("the token is not valid yet"));
    }
    if claims.iss.as_deref() != Some(issuer) {
        return Err(Rejection::unauthorized(
            "the token was issued by another issuer",
        ));
    }
    if !claims.audiences().iter().any(|held| held == audience) {
        return Err(Rejection::unauthorized(
            "the token was issued for another audience",
        ));
    }
    claims.principal_name().ok_or_else(|| {
        Rejection::unauthorized("the token names no subject: it carries neither sub nor client_id")
    })
}

impl Oauth {
    async fn validate(&self, token: &str) -> std::result::Result<Principal, Rejection> {
        let mut parts = token.split('.');
        let (Some(header_text), Some(payload_text), Some(signature_text), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(Rejection::unauthorized(
                "the token is not a compact JWS with three parts",
            ));
        };
        let header: Header = decode_base64url(header_text)
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .ok_or_else(|| Rejection::unauthorized("the token header is not valid"))?;
        let Some(algorithm) = Algorithm::parse(&header.alg) else {
            return Err(Rejection::unauthorized(format!(
                "the token algorithm {} is not accepted",
                header.alg.chars().take(16).collect::<String>()
            )));
        };
        let Some(kid) = header.kid.as_deref() else {
            return Err(Rejection::unauthorized("the token names no key id"));
        };
        let key = match self.jwks.key(kid).await {
            Ok(Some(key)) => key,
            Ok(None) => {
                return Err(Rejection::unauthorized(
                    "the token's key id is unknown to the issuer",
                ));
            }
            Err(JwksError::Unreachable(detail)) => {
                tracing::warn!(%detail, "the key endpoint is unreachable; the token is refused");
                return Err(Rejection::unauthorized(
                    "the key endpoint could not be reached",
                ));
            }
            Err(JwksError::Unusable(detail)) => {
                tracing::warn!(%detail, "the key endpoint document is unusable; the token is refused");
                return Err(Rejection::unauthorized("the key endpoint is unusable"));
            }
        };
        if !key.accepts(algorithm) {
            return Err(Rejection::unauthorized(
                "the token algorithm does not match its key",
            ));
        }
        let signature = decode_base64url(signature_text)
            .ok_or_else(|| Rejection::unauthorized("the token signature is not valid"))?;
        let signed = format!("{header_text}.{payload_text}");
        if !key.verify(algorithm, signed.as_bytes(), &signature) {
            return Err(Rejection::unauthorized(
                "the token signature does not verify",
            ));
        }
        let claims: Claims = decode_base64url(payload_text)
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .ok_or_else(|| Rejection::unauthorized("the token claims are not valid"))?;
        let name = principal_from_claims(
            &claims,
            &self.settings.issuer.value,
            &self.settings.audience.value,
            now_seconds(),
        )?;
        Ok(Principal::from_token(
            name,
            PrincipalKind::TokenSubject,
            claims.scopes(),
        ))
    }
}

#[derive(Debug)]
pub enum Authenticator {
    None,
    Bearer(BearerTokens),
    Oauth(Oauth),
}

impl Authenticator {
    pub fn build(settings: &Settings, environment_tokens: Option<&str>) -> Result<Self> {
        match &settings.http.auth {
            AuthSettings::None => Ok(Self::None),
            AuthSettings::Bearer {
                tokens_file,
                tokens_from_environment,
            } => {
                let mut tokens = BearerTokens::default();
                if let Some(file) = tokens_file {
                    tokens.merge(
                        BearerTokens::from_file(&file.value)?,
                        &file.value.display().to_string(),
                    )?;
                }
                if *tokens_from_environment && let Some(text) = environment_tokens {
                    let normalized = text.replace(';', "\n");
                    tokens.merge(
                        BearerTokens::parse_labeled(&normalized, "OWNPG_BEARER_TOKENS", "env")?,
                        "OWNPG_BEARER_TOKENS",
                    )?;
                }
                if tokens.is_empty() {
                    return Err(Error::ConfigInvalid {
                        setting: "auth".to_owned(),
                        value: "bearer".to_owned(),
                        detail: "no bearer token was configured".to_owned(),
                    });
                }
                Ok(Self::Bearer(tokens))
            }
            AuthSettings::Oauth(oauth) => {
                let jwks = JwksClient::new(oauth.jwks_url.value.clone()).map_err(|error| {
                    Error::ConfigInvalid {
                        setting: "http.oauth_jwks_url".to_owned(),
                        value: oauth.jwks_url.value.clone(),
                        detail: error.to_string(),
                    }
                })?;
                Ok(Self::Oauth(Oauth {
                    settings: oauth.clone(),
                    jwks: Arc::new(jwks),
                }))
            }
        }
    }

    #[must_use]
    pub const fn requires_token(&self) -> bool {
        !matches!(self, Self::None)
    }

    pub async fn authenticate(
        &self,
        authorization: Option<&str>,
        anonymous: &Principal,
    ) -> std::result::Result<(Principal, String), Rejection> {
        let presented = authorization.and_then(|value| {
            let (scheme, rest) = value.trim().split_once(' ')?;
            scheme
                .eq_ignore_ascii_case("bearer")
                .then(|| rest.trim().to_owned())
        });
        match self {
            Self::None => Ok((anonymous.clone(), String::new())),
            Self::Bearer(tokens) => {
                let Some(token) = presented else {
                    return Err(Rejection::missing());
                };
                if token.len() > MAX_TOKEN_BYTES {
                    return Err(Rejection::unauthorized("the token is too long"));
                }
                let Some(found) = tokens.lookup(token.as_bytes()) else {
                    return Err(Rejection::unauthorized("the token is not known"));
                };
                Ok((
                    Principal::from_token(
                        found.name.clone(),
                        PrincipalKind::Bearer,
                        found.scopes.clone(),
                    ),
                    found.fingerprint.clone(),
                ))
            }
            Self::Oauth(oauth) => {
                let Some(token) = presented else {
                    return Err(Rejection::missing());
                };
                if token.len() > MAX_TOKEN_BYTES {
                    return Err(Rejection::unauthorized("the token is too long"));
                }
                let principal = oauth.validate(&token).await?;
                let fingerprint: String = sha256_hex(token.as_bytes()).chars().take(16).collect();
                Ok((principal, fingerprint))
            }
        }
    }

    pub async fn ready(&self) -> std::result::Result<(), String> {
        match self {
            Self::Oauth(oauth) => oauth.jwks.ready().await.map_err(|error| error.to_string()),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_lines_carry_a_mode_or_explicit_scopes() {
        let tokens = BearerTokens::parse(
            "# comment\nabcdefghijklmnop0123 read-only reader\nzyxwvutsrqponmlk9876 ownpg:read,ownpg:write\n",
            "test",
        )
        .unwrap();
        assert_eq!(tokens.len(), 2);
        let reader = tokens.lookup(b"abcdefghijklmnop0123").unwrap();
        assert_eq!(reader.name, "reader");
        assert_eq!(reader.scopes, vec![SCOPE_READ.to_owned()]);
        let writer = tokens.lookup(b"zyxwvutsrqponmlk9876").unwrap();
        assert_eq!(writer.name, "token-3");
        assert!(
            BearerTokens::parse(
                "abcdefghijklmnop0123 read-only a\nabcdefghijklmnop0123 read-write b\n",
                "test"
            )
            .is_err()
        );
        assert!(
            BearerTokens::parse(
                "abcdefghijklmnop0123 read-only same\nzyxwvutsrqponmlk9876 read-write same\n",
                "test"
            )
            .is_err()
        );
        let mut merged =
            BearerTokens::parse_labeled("abcdefghijklmnop0123 read-only\n", "f", "file").unwrap();
        assert_eq!(merged.tokens[0].name, "file-1");
        let env =
            BearerTokens::parse_labeled("zyxwvutsrqponmlk9876 read-only\n", "e", "env").unwrap();
        assert_eq!(env.tokens[0].name, "env-1");
        merged.merge(env, "e").unwrap();
        assert_eq!(merged.len(), 2);
        let clash =
            BearerTokens::parse_labeled("abcdefghijklmnop0123 read-only other\n", "e", "env")
                .unwrap();
        assert!(merged.merge(clash, "e").is_err());
        assert_eq!(
            writer.scopes,
            vec![SCOPE_READ.to_owned(), SCOPE_WRITE.to_owned()]
        );
        assert!(tokens.lookup(b"abcdefghijklmnop0124").is_none());
        assert!(tokens.lookup(b"abcdefghijklmnop012").is_none());
        assert!(BearerTokens::parse("short read-only\n", "test").is_err());
        assert!(BearerTokens::parse("abcdefghijklmnop0123 admin\n", "test").is_err());
        assert!(BearerTokens::parse("\n\n", "test").is_err());
    }

    #[test]
    fn modes_map_onto_scope_sets() {
        assert_eq!(scopes_for_mode(Mode::ReadOnly), vec![SCOPE_READ.to_owned()]);
        assert!(!scopes_for_mode(Mode::WriteOnly).contains(&SCOPE_READ.to_owned()));
        assert_eq!(scopes_for_mode(Mode::ReadWrite).len(), 6);
    }

    fn claims(exp: Option<u64>, nbf: Option<u64>, iss: &str, aud: serde_json::Value) -> Claims {
        Claims {
            iss: Some(iss.to_owned()),
            aud: Some(aud),
            exp,
            nbf,
            sub: Some("alice".to_owned()),
            client_id: None,
            scope: Some("ownpg:read ownpg:write".to_owned()),
            scp: Some(serde_json::json!(["ownpg:ddl", "ownpg:read"])),
        }
    }

    #[test]
    fn scope_claims_come_from_scope_and_scp() {
        let claims = claims(Some(100), None, "https://issuer", serde_json::json!("aud"));
        assert_eq!(claims.scopes(), ["ownpg:ddl", "ownpg:read", "ownpg:write"]);
    }

    #[test]
    fn claims_are_checked_for_time_issuer_and_audience() {
        let issuer = "https://auth.example.com";
        let audience = "https://db.example.com/mcp";
        let good = claims(
            Some(1_000),
            None,
            issuer,
            serde_json::json!([audience, "x"]),
        );
        assert!(principal_from_claims(&good, issuer, audience, 990).is_ok());
        assert!(principal_from_claims(&good, issuer, audience, 1_000 + LEEWAY_SECONDS).is_ok());
        assert!(
            principal_from_claims(&good, issuer, audience, 1_000 + LEEWAY_SECONDS + 1).is_err()
        );
        let early = claims(Some(1_000), Some(900), issuer, serde_json::json!(audience));
        assert!(principal_from_claims(&early, issuer, audience, 900 - LEEWAY_SECONDS - 1).is_err());
        assert!(principal_from_claims(&early, issuer, audience, 900 - LEEWAY_SECONDS).is_ok());
        let other_issuer = claims(
            Some(1_000),
            None,
            "https://other",
            serde_json::json!(audience),
        );
        assert!(principal_from_claims(&other_issuer, issuer, audience, 10).is_err());
        let other_audience = claims(
            Some(1_000),
            None,
            issuer,
            serde_json::json!("https://x/mcp"),
        );
        assert!(principal_from_claims(&other_audience, issuer, audience, 10).is_err());
        let no_exp = claims(None, None, issuer, serde_json::json!(audience));
        assert!(principal_from_claims(&no_exp, issuer, audience, 10).is_err());
        assert_eq!(
            principal_from_claims(&good, issuer, audience, 990)
                .ok()
                .as_deref(),
            Some("sub:alice")
        );
        let mut client = claims(Some(1_000), None, issuer, serde_json::json!(audience));
        client.sub = None;
        client.client_id = Some("svc-7".to_owned());
        assert_eq!(
            principal_from_claims(&client, issuer, audience, 990)
                .ok()
                .as_deref(),
            Some("client:svc-7")
        );
        let mut anonymous = claims(Some(1_000), None, issuer, serde_json::json!(audience));
        anonymous.sub = Some("  ".to_owned());
        let refusal = principal_from_claims(&anonymous, issuer, audience, 990).unwrap_err();
        assert!(refusal.description.contains("neither sub nor client_id"));
    }
}
