use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use ring::signature::{self, VerificationAlgorithm};
use serde::Deserialize;
use tokio::sync::Mutex;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
pub const BODY_CAP: usize = 1024 * 1024;
pub const DEFAULT_TTL: Duration = Duration::from_secs(300);
pub const MIN_TTL: Duration = Duration::from_secs(60);
pub const MAX_TTL: Duration = Duration::from_secs(24 * 60 * 60);
pub const MAX_STALE: Duration = Duration::from_secs(24 * 60 * 60);
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    Rs256,
    Rs384,
    Rs512,
    Ps256,
    Ps384,
    Ps512,
    Es256,
    Es384,
    EdDsa,
}

impl Algorithm {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "RS256" => Some(Self::Rs256),
            "RS384" => Some(Self::Rs384),
            "RS512" => Some(Self::Rs512),
            "PS256" => Some(Self::Ps256),
            "PS384" => Some(Self::Ps384),
            "PS512" => Some(Self::Ps512),
            "ES256" => Some(Self::Es256),
            "ES384" => Some(Self::Es384),
            "EdDSA" => Some(Self::EdDsa),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rs256 => "RS256",
            Self::Rs384 => "RS384",
            Self::Rs512 => "RS512",
            Self::Ps256 => "PS256",
            Self::Ps384 => "PS384",
            Self::Ps512 => "PS512",
            Self::Es256 => "ES256",
            Self::Es384 => "ES384",
            Self::EdDsa => "EdDSA",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curve {
    P256,
    P384,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyingKey {
    Rsa { n: Vec<u8>, e: Vec<u8> },
    Ec { curve: Curve, point: Vec<u8> },
    Ed25519(Vec<u8>),
}

impl VerifyingKey {
    #[must_use]
    pub fn accepts(&self, algorithm: Algorithm) -> bool {
        matches!(
            (self, algorithm),
            (
                Self::Rsa { .. },
                Algorithm::Rs256
                    | Algorithm::Rs384
                    | Algorithm::Rs512
                    | Algorithm::Ps256
                    | Algorithm::Ps384
                    | Algorithm::Ps512,
            ) | (
                Self::Ec {
                    curve: Curve::P256,
                    ..
                },
                Algorithm::Es256,
            ) | (
                Self::Ec {
                    curve: Curve::P384,
                    ..
                },
                Algorithm::Es384,
            ) | (Self::Ed25519(_), Algorithm::EdDsa)
        )
    }

    pub fn verify(&self, algorithm: Algorithm, message: &[u8], sig: &[u8]) -> bool {
        if !self.accepts(algorithm) {
            return false;
        }
        match self {
            Self::Rsa { n, e } => {
                let params: &signature::RsaParameters = match algorithm {
                    Algorithm::Rs256 => &signature::RSA_PKCS1_2048_8192_SHA256,
                    Algorithm::Rs384 => &signature::RSA_PKCS1_2048_8192_SHA384,
                    Algorithm::Rs512 => &signature::RSA_PKCS1_2048_8192_SHA512,
                    Algorithm::Ps256 => &signature::RSA_PSS_2048_8192_SHA256,
                    Algorithm::Ps384 => &signature::RSA_PSS_2048_8192_SHA384,
                    Algorithm::Ps512 => &signature::RSA_PSS_2048_8192_SHA512,
                    _ => return false,
                };
                signature::RsaPublicKeyComponents { n, e }
                    .verify(params, message, sig)
                    .is_ok()
            }
            Self::Ec { curve, point } => {
                let algorithm: &'static dyn VerificationAlgorithm = match curve {
                    Curve::P256 => &signature::ECDSA_P256_SHA256_FIXED,
                    Curve::P384 => &signature::ECDSA_P384_SHA384_FIXED,
                };
                signature::UnparsedPublicKey::new(algorithm, point)
                    .verify(message, sig)
                    .is_ok()
            }
            Self::Ed25519(public) => signature::UnparsedPublicKey::new(&signature::ED25519, public)
                .verify(message, sig)
                .is_ok(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct Jwk {
    kty: String,
    #[serde(default)]
    kid: Option<String>,
    #[serde(default, rename = "use")]
    public_key_use: Option<String>,
    #[serde(default)]
    crv: Option<String>,
    #[serde(default)]
    n: Option<String>,
    #[serde(default)]
    e: Option<String>,
    #[serde(default)]
    x: Option<String>,
    #[serde(default)]
    y: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JwkSet {
    keys: Vec<Jwk>,
}

pub fn decode_base64url(text: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(text.trim_end_matches('='))
        .ok()
}

fn key_from_jwk(jwk: &Jwk) -> Option<VerifyingKey> {
    if jwk
        .public_key_use
        .as_deref()
        .is_some_and(|purpose| purpose != "sig")
    {
        return None;
    }
    match jwk.kty.as_str() {
        "RSA" => Some(VerifyingKey::Rsa {
            n: decode_base64url(jwk.n.as_deref()?)?,
            e: decode_base64url(jwk.e.as_deref()?)?,
        }),
        "EC" => {
            let curve = match jwk.crv.as_deref()? {
                "P-256" => Curve::P256,
                "P-384" => Curve::P384,
                _ => return None,
            };
            let x = decode_base64url(jwk.x.as_deref()?)?;
            let y = decode_base64url(jwk.y.as_deref()?)?;
            let width = match curve {
                Curve::P256 => 32,
                Curve::P384 => 48,
            };
            if x.len() != width || y.len() != width {
                return None;
            }
            let mut point = Vec::with_capacity(1 + width * 2);
            point.push(0x04);
            point.extend_from_slice(&x);
            point.extend_from_slice(&y);
            Some(VerifyingKey::Ec { curve, point })
        }
        "OKP" => {
            if jwk.crv.as_deref()? != "Ed25519" {
                return None;
            }
            let x = decode_base64url(jwk.x.as_deref()?)?;
            (x.len() == 32).then_some(VerifyingKey::Ed25519(x))
        }
        _ => None,
    }
}

pub fn parse_key_set(body: &[u8]) -> Result<HashMap<String, Arc<VerifyingKey>>, JwksError> {
    let set: JwkSet =
        serde_json::from_slice(body).map_err(|error| JwksError::Unusable(error.to_string()))?;
    let mut keys = HashMap::new();
    for jwk in &set.keys {
        let Some(kid) = jwk.kid.clone() else {
            continue;
        };
        if let Some(key) = key_from_jwk(jwk) {
            keys.insert(kid, Arc::new(key));
        }
    }
    if keys.is_empty() {
        return Err(JwksError::Unusable(
            "no usable asymmetric key with a key id".to_owned(),
        ));
    }
    Ok(keys)
}

#[derive(Clone)]
struct Cache {
    keys: Arc<HashMap<String, Arc<VerifyingKey>>>,
    fetched_at: Option<Instant>,
    ttl: Duration,
    last_attempt: Option<Instant>,
}

impl Cache {
    fn is_fresh(&self) -> bool {
        self.fetched_at
            .is_some_and(|fetched| fetched.elapsed() < self.ttl)
    }

    fn is_throttled(&self) -> bool {
        self.last_attempt
            .is_some_and(|attempt| attempt.elapsed() < REFRESH_INTERVAL)
    }

    fn is_stale(&self) -> bool {
        self.fetched_at
            .is_none_or(|fetched| fetched.elapsed() >= MAX_STALE)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum JwksError {
    #[error("the key endpoint could not be reached: {0}")]
    Unreachable(String),
    #[error("the key endpoint answered with an unusable document: {0}")]
    Unusable(String),
}

pub struct JwksClient {
    url: String,
    http: reqwest::Client,
    cache: std::sync::RwLock<Cache>,
    refreshing: Mutex<()>,
}

impl std::fmt::Debug for JwksClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JwksClient")
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

fn max_age(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let value = headers.get(reqwest::header::CACHE_CONTROL)?.to_str().ok()?;
    value
        .split(',')
        .map(str::trim)
        .find_map(|directive| directive.strip_prefix("max-age="))
        .and_then(|seconds| seconds.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

impl JwksClient {
    pub fn new(url: String) -> Result<Self, JwksError> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(TOTAL_TIMEOUT)
            .user_agent(format!("ownpg/{}", crate::VERSION))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| JwksError::Unreachable(error.to_string()))?;
        Ok(Self {
            url,
            http,
            cache: std::sync::RwLock::new(Cache {
                keys: Arc::new(HashMap::new()),
                fetched_at: None,
                ttl: DEFAULT_TTL,
                last_attempt: None,
            }),
            refreshing: Mutex::new(()),
        })
    }

    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    fn snapshot(&self) -> Cache {
        self.cache
            .read()
            .map(|cache| cache.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
    }

    fn store(&self, update: impl FnOnce(&mut Cache)) {
        match self.cache.write() {
            Ok(mut cache) => update(&mut cache),
            Err(poisoned) => update(&mut poisoned.into_inner()),
        }
    }

    async fn fetch(&self) -> Result<(HashMap<String, Arc<VerifyingKey>>, Duration), JwksError> {
        let response = self
            .http
            .get(&self.url)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|error| JwksError::Unreachable(error.to_string()))?;
        if !response.status().is_success() {
            return Err(JwksError::Unreachable(format!(
                "status {}",
                response.status()
            )));
        }
        if response
            .content_length()
            .is_some_and(|length| length > BODY_CAP as u64)
        {
            return Err(JwksError::Unusable(format!(
                "the document is larger than {BODY_CAP} bytes"
            )));
        }
        let ttl = max_age(response.headers())
            .unwrap_or(DEFAULT_TTL)
            .clamp(MIN_TTL, MAX_TTL);
        let mut response = response;
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| JwksError::Unreachable(error.to_string()))?
        {
            if body.len() + chunk.len() > BODY_CAP {
                return Err(JwksError::Unusable(format!(
                    "the document is larger than {BODY_CAP} bytes"
                )));
            }
            body.extend_from_slice(&chunk);
        }
        Ok((parse_key_set(&body)?, ttl))
    }

    async fn refresh(&self, seen: Option<Instant>) -> Result<(), JwksError> {
        let _serial = self.refreshing.lock().await;
        let current = self.snapshot();
        if current.fetched_at != seen && current.is_fresh() {
            return Ok(());
        }
        if current.is_throttled() {
            return if current.keys.is_empty() {
                Err(JwksError::Unreachable(
                    "the last fetch failed less than a minute ago".to_owned(),
                ))
            } else {
                Ok(())
            };
        }
        self.store(|cache| cache.last_attempt = Some(Instant::now()));
        let (keys, ttl) = self.fetch().await?;
        self.store(|cache| {
            cache.keys = Arc::new(keys);
            cache.ttl = ttl;
            cache.fetched_at = Some(Instant::now());
        });
        Ok(())
    }

    pub async fn key(&self, kid: &str) -> Result<Option<Arc<VerifyingKey>>, JwksError> {
        let cache = self.snapshot();
        if cache.is_fresh() {
            if let Some(key) = cache.keys.get(kid) {
                return Ok(Some(Arc::clone(key)));
            }
            if cache.is_throttled() {
                return Ok(None);
            }
        }
        match self.refresh(cache.fetched_at).await {
            Ok(()) => {}
            Err(error) if cache.keys.is_empty() || cache.is_stale() => return Err(error),
            Err(error) => {
                tracing::warn!(%error, "the key endpoint did not answer; the cached keys stay in use");
            }
        }
        Ok(self.snapshot().keys.get(kid).cloned())
    }

    pub async fn ready(&self) -> Result<(), JwksError> {
        let cache = self.snapshot();
        if cache.is_fresh() {
            return Ok(());
        }
        if cache.is_throttled() {
            return if cache.keys.is_empty() || cache.is_stale() {
                Err(JwksError::Unreachable(
                    "the last fetch failed less than a minute ago and no usable keys are cached"
                        .to_owned(),
                ))
            } else {
                Ok(())
            };
        }
        match self.refresh(cache.fetched_at).await {
            Ok(()) => Ok(()),
            Err(error) if cache.keys.is_empty() || cache.is_stale() => Err(error),
            Err(_) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_age_is_read_from_cache_control() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CACHE_CONTROL,
            "public, max-age=600, must-revalidate".parse().unwrap(),
        );
        assert_eq!(max_age(&headers), Some(Duration::from_secs(600)));
        headers.insert(reqwest::header::CACHE_CONTROL, "no-store".parse().unwrap());
        assert_eq!(max_age(&headers), None);
    }

    #[test]
    fn key_families_accept_only_their_algorithms() {
        let rsa = VerifyingKey::Rsa {
            n: vec![1],
            e: vec![1],
        };
        assert!(rsa.accepts(Algorithm::Rs256));
        assert!(rsa.accepts(Algorithm::Ps512));
        assert!(!rsa.accepts(Algorithm::Es256));
        let ec = VerifyingKey::Ec {
            curve: Curve::P256,
            point: vec![4],
        };
        assert!(ec.accepts(Algorithm::Es256));
        assert!(!ec.accepts(Algorithm::Es384));
        assert!(VerifyingKey::Ed25519(vec![0; 32]).accepts(Algorithm::EdDsa));
        assert_eq!(Algorithm::parse("HS256"), None);
        assert_eq!(Algorithm::parse("none"), None);
    }

    #[test]
    fn a_key_set_keeps_only_signing_keys_with_an_id() {
        let x = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7u8; 32]);
        let body = serde_json::json!({
            "keys": [
                {"kty": "OKP", "crv": "Ed25519", "kid": "ed", "x": x},
                {"kty": "OKP", "crv": "Ed25519", "x": x},
                {"kty": "RSA", "kid": "enc", "use": "enc", "n": "AQAB", "e": "AQAB"},
                {"kty": "oct", "kid": "sym", "k": "AQAB"}
            ]
        });
        let keys = parse_key_set(serde_json::to_vec(&body).unwrap().as_slice()).unwrap();
        assert_eq!(keys.len(), 1);
        assert!(keys.contains_key("ed"));
        assert!(parse_key_set(br#"{"keys": []}"#).is_err());
    }
}
