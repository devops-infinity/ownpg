use std::path::Path;
use std::sync::Arc;

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use tokio_postgres_rustls::MakeRustlsConnect;

use crate::config::{ConnectionSettings, SslMode};
use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verification {
    None,
    Chain,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootSource {
    File,
    Platform,
    Bundled,
    NotNeeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TlsPlan {
    pub driver_mode: tokio_postgres::config::SslMode,
    pub verification: Verification,
    pub roots: RootSource,
    pub client_certificate: bool,
}

impl TlsPlan {
    #[must_use]
    pub fn describe(&self) -> String {
        let verify = match self.verification {
            Verification::None => "unverified",
            Verification::Chain => "chain verified, host name not checked",
            Verification::Full => "chain and host name verified",
        };
        let roots = match self.roots {
            RootSource::File => " with the configured root certificate",
            RootSource::Platform => " with the platform trust store",
            RootSource::Bundled => " with the bundled Mozilla roots",
            RootSource::NotNeeded => "",
        };
        let client = if self.client_certificate {
            ", presenting a client certificate"
        } else {
            ""
        };
        format!("{verify}{roots}{client}")
    }
}

pub struct Tls {
    pub connector: MakeRustlsConnect,
    pub plan: TlsPlan,
}

impl std::fmt::Debug for Tls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tls").field("plan", &self.plan).finish()
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn tls_error(target: &str, detail: impl Into<String>) -> Error {
    Error::TlsFailed {
        target: target.to_owned(),
        detail: detail.into(),
        source: None,
    }
}

pub fn build(connection: &ConnectionSettings, target: &str) -> Result<Tls> {
    let mode = connection.sslmode.value;
    let rootcert = connection
        .sslrootcert
        .as_ref()
        .map(|value| value.value.as_path());
    let client_certificate = connection.sslcert.is_some();

    let driver_mode = match mode {
        SslMode::Disable => tokio_postgres::config::SslMode::Disable,
        SslMode::Allow | SslMode::Prefer => tokio_postgres::config::SslMode::Prefer,
        SslMode::Require | SslMode::VerifyCa | SslMode::VerifyFull => {
            tokio_postgres::config::SslMode::Require
        }
    };

    let (verification, roots) = match (mode, rootcert) {
        (SslMode::Disable, _) => (Verification::None, RootSource::NotNeeded),
        (SslMode::Allow | SslMode::Prefer | SslMode::Require, None) => {
            (Verification::None, RootSource::NotNeeded)
        }
        (SslMode::Allow | SslMode::Prefer | SslMode::Require | SslMode::VerifyCa, Some(_)) => {
            (Verification::Chain, RootSource::File)
        }
        (SslMode::VerifyFull, Some(_)) => (Verification::Full, RootSource::File),
        (SslMode::VerifyCa, None) => (Verification::Chain, RootSource::Platform),
        (SslMode::VerifyFull, None) => (Verification::Full, RootSource::Platform),
    };

    let provider = provider();
    let builder = ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .map_err(|error| tls_error(target, format!("no usable protocol version: {error}")))?;

    let (roots, store) = match roots {
        RootSource::NotNeeded => (RootSource::NotNeeded, None),
        RootSource::File => {
            let path = rootcert.ok_or_else(|| tls_error(target, "no root certificate path"))?;
            (RootSource::File, Some(load_roots(path, target)?))
        }
        RootSource::Platform | RootSource::Bundled => {
            let (source, store) = platform_or_bundled_roots();
            (source, Some(store))
        }
    };

    let with_verifier = match (verification, store) {
        (Verification::None, _) => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerification {
                provider: Arc::clone(&provider),
            })),
        (Verification::Chain, Some(store)) => {
            let inner =
                WebPkiServerVerifier::builder_with_provider(Arc::new(store), Arc::clone(&provider))
                    .build()
                    .map_err(|error| {
                        tls_error(target, format!("the root store is unusable: {error}"))
                    })?;
            builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(ChainOnly { inner }))
        }
        (Verification::Full, Some(store)) => builder.with_root_certificates(store),
        (Verification::Chain | Verification::Full, None) => {
            return Err(tls_error(
                target,
                "verification was requested without a root store",
            ));
        }
    };

    let config = match (&connection.sslcert, &connection.sslkey) {
        (Some(cert), Some(key)) => {
            let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(&cert.value)
                .map_err(|error| {
                    tls_error(
                        target,
                        format!("the client certificate could not be read: {error}"),
                    )
                })?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|error| {
                    tls_error(
                        target,
                        format!("the client certificate could not be parsed: {error}"),
                    )
                })?;
            let key = PrivateKeyDer::from_pem_file(&key.value).map_err(|error| {
                tls_error(target, format!("the client key could not be read: {error}"))
            })?;
            with_verifier
                .with_client_auth_cert(certs, key)
                .map_err(|error| {
                    tls_error(
                        target,
                        format!("the client certificate is unusable: {error}"),
                    )
                })?
        }
        (Some(_), None) => {
            return Err(tls_error(target, "sslcert is set but sslkey is not"));
        }
        (None, Some(_)) => {
            return Err(tls_error(target, "sslkey is set but sslcert is not"));
        }
        (None, None) => with_verifier.with_no_client_auth(),
    };

    Ok(Tls {
        connector: MakeRustlsConnect::new(config),
        plan: TlsPlan {
            driver_mode,
            verification,
            roots,
            client_certificate,
        },
    })
}

fn load_roots(path: &Path, target: &str) -> Result<RootCertStore> {
    if path.as_os_str() == "system" {
        return Ok(platform_or_bundled_roots().1);
    }
    let mut store = RootCertStore::empty();
    let certs = CertificateDer::pem_file_iter(path).map_err(|error| {
        tls_error(
            target,
            format!("{} could not be read: {error}", path.display()),
        )
    })?;
    let mut added = 0usize;
    for cert in certs {
        let cert = cert.map_err(|error| {
            tls_error(
                target,
                format!(
                    "{} holds an unreadable certificate: {error}",
                    path.display()
                ),
            )
        })?;
        store.add(cert).map_err(|error| {
            tls_error(
                target,
                format!("{} holds an unusable certificate: {error}", path.display()),
            )
        })?;
        added += 1;
    }
    if added == 0 {
        return Err(tls_error(
            target,
            format!("{} holds no certificate", path.display()),
        ));
    }
    Ok(store)
}

fn platform_or_bundled_roots() -> (RootSource, RootCertStore) {
    let native = rustls_native_certs::load_native_certs();
    if native.certs.is_empty() {
        return (
            RootSource::Bundled,
            RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
            },
        );
    }
    let mut store = RootCertStore::empty();
    store.add_parsable_certificates(native.certs);
    (RootSource::Platform, store)
}

#[derive(Debug)]
struct NoVerification {
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for NoVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[derive(Debug)]
struct ChainOnly {
    inner: Arc<WebPkiServerVerifier>,
}

impl ServerCertVerifier for ChainOnly {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        match self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
            )) => Ok(ServerCertVerified::assertion()),
            other => other,
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ChannelBinding, Origin, Resolved, SslMode};
    use std::time::Duration;

    fn connection(mode: SslMode, rootcert: Option<&str>) -> ConnectionSettings {
        ConnectionSettings {
            dsn: None,
            service: None,
            host: Some(Resolved::new("db.internal".to_owned(), Origin::Flag)),
            hostaddr: None,
            port: Resolved::preset(5432),
            user: Resolved::preset("u".to_owned()),
            password: None,
            sslmode: Resolved::new(mode, Origin::Flag),
            sslrootcert: rootcert.map(|path| Resolved::new(path.into(), Origin::Flag)),
            sslcert: None,
            sslkey: None,
            channel_binding: Resolved::preset(ChannelBinding::Prefer),
            connect_timeout: Resolved::preset(Duration::from_secs(5)),
            application_name: Resolved::preset("ownpg".to_owned()),
            options: None,
            pooled: Resolved::preset(None),
        }
    }

    #[test]
    fn every_libpq_mode_maps_onto_a_driver_mode_and_a_verification_level() {
        let cases = [
            (
                SslMode::Disable,
                tokio_postgres::config::SslMode::Disable,
                Verification::None,
            ),
            (
                SslMode::Allow,
                tokio_postgres::config::SslMode::Prefer,
                Verification::None,
            ),
            (
                SslMode::Prefer,
                tokio_postgres::config::SslMode::Prefer,
                Verification::None,
            ),
            (
                SslMode::Require,
                tokio_postgres::config::SslMode::Require,
                Verification::None,
            ),
            (
                SslMode::VerifyCa,
                tokio_postgres::config::SslMode::Require,
                Verification::Chain,
            ),
            (
                SslMode::VerifyFull,
                tokio_postgres::config::SslMode::Require,
                Verification::Full,
            ),
        ];
        for (mode, driver, verification) in cases {
            let tls = build(&connection(mode, None), "db").unwrap();
            assert_eq!(tls.plan.driver_mode, driver, "{mode}");
            assert_eq!(tls.plan.verification, verification, "{mode}");
            assert!(!tls.plan.client_certificate);
        }
    }

    #[test]
    fn require_with_a_root_certificate_verifies_the_chain_like_libpq() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("root.pem");
        std::fs::write(&path, SELF_SIGNED_ROOT).unwrap();
        let tls = build(&connection(SslMode::Require, path.to_str()), "db").unwrap();
        assert_eq!(tls.plan.verification, Verification::Chain);
        assert_eq!(tls.plan.roots, RootSource::File);
        assert!(tls.plan.describe().contains("host name not checked"));
    }

    #[test]
    fn a_missing_or_empty_root_file_is_a_tls_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("none.pem");
        let error = build(&connection(SslMode::VerifyFull, missing.to_str()), "db").unwrap_err();
        assert_eq!(error.id(), crate::error::ErrorId::TlsFailed);
        let empty = dir.path().join("empty.pem");
        std::fs::write(&empty, "").unwrap();
        let error = build(&connection(SslMode::VerifyFull, empty.to_str()), "db").unwrap_err();
        assert!(
            error.to_string().contains("holds no certificate"),
            "{error}"
        );
    }

    #[test]
    fn a_client_certificate_needs_its_key() {
        let mut settings = connection(SslMode::Require, None);
        settings.sslcert = Some(Resolved::new("/x/cert.pem".into(), Origin::Flag));
        let error = build(&settings, "db").unwrap_err();
        assert!(error.to_string().contains("sslkey"), "{error}");
    }

    #[test]
    fn the_plan_description_reads_plainly() {
        let plan = TlsPlan {
            driver_mode: tokio_postgres::config::SslMode::Require,
            verification: Verification::Full,
            roots: RootSource::Platform,
            client_certificate: true,
        };
        assert_eq!(
            plan.describe(),
            "chain and host name verified with the platform trust store, presenting a client certificate"
        );
    }

    const SELF_SIGNED_ROOT: &str = "-----BEGIN CERTIFICATE-----\nMIIBijCCAS+gAwIBAgIUHMtNQ88LzKYbwaZX1zHzJW58xf8wCgYIKoZIzj0EAwIw\nGjEYMBYGA1UEAwwPb3ducGctdGVzdC1yb290MB4XDTI2MDkxMjExMTY0MFoXDTM2\nMDkwOTExMTY0MFowGjEYMBYGA1UEAwwPb3ducGctdGVzdC1yb290MFkwEwYHKoZI\nzj0CAQYIKoZIzj0DAQcDQgAEuHF4Mwx0dc4T3Da2rTdlINjzsvqDu2JiLHZO3mCK\nSfY2WXoX/xEgtt7JwYdPKKXLKqYGjC/z/R2XhJ53JF+WIKNTMFEwHQYDVR0OBBYE\nFA9nN7SxKlodp/jEkk5T8GRqHn2SMB8GA1UdIwQYMBaAFA9nN7SxKlodp/jEkk5T\n8GRqHn2SMA8GA1UdEwEB/wQFMAMBAf8wCgYIKoZIzj0EAwIDSQAwRgIhANOiLcbW\n5UbCn5lq+zp8CV3yT8dK1DZx6lEJ2ucLnxyYAiEAkCJqzi4GRU0NBZMWtjI5Bu0g\nz8ut9/AqZgN0+a7tCNY=\n-----END CERTIFICATE-----\n";
}
