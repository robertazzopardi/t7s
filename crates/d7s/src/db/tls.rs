//! TLS settings for server connections, read from the connection URL query:
//! `sslmode=` (Postgres) / `ssl-mode=` (MySQL) and `sslrootcert=` (CA bundle path).

use std::{
    io,
    sync::{Arc, OnceLock},
};

use rustls::{
    ClientConfig, DigitallySignedStruct, Error as TlsError, RootCertStore,
    SignatureScheme,
    client::danger::{
        HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
    },
    crypto::{CryptoProvider, aws_lc_rs},
    pki_types::{CertificateDer, ServerName, UnixTime, pem::PemObject},
};
// Loaded once: reading the OS store on every query-time connect is slow.
static NATIVE: OnceLock<Vec<CertificateDer<'static>>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TlsMode {
    /// Plain TCP (default, matches the behaviour before TLS support).
    #[default]
    Disable,
    /// Try TLS, fall back to plain; certificate is not verified.
    Prefer,
    /// TLS required; certificate is not verified.
    Require,
    /// TLS required; certificate chain and host name verified.
    VerifyFull,
}

impl TlsMode {
    fn parse(s: &str) -> Option<Self> {
        Some(match s.to_lowercase().replace('_', "-").as_str() {
            "disable" | "disabled" => Self::Disable,
            "prefer" | "preferred" => Self::Prefer,
            "require" | "required" => Self::Require,
            // rustls always checks the host name, so verify-ca is as strict as verify-full.
            "verify-ca" | "verify-full" | "verify-identity" => Self::VerifyFull,
            _ => return None,
        })
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Prefer => "prefer",
            Self::Require => "require",
            Self::VerifyFull => "verify-full",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TlsConfig {
    pub mode: TlsMode,
    /// PEM file with extra root certificates (self-signed or private CA).
    pub ca_path: Option<String>,
    /// Unrecognized `sslmode` / `ssl-mode` from the URL (connect must fail).
    invalid_mode: Option<String>,
}

impl TlsConfig {
    #[must_use]
    pub fn from_url(url: &str) -> Self {
        let mut cfg = Self::default();
        let Ok(url) = url::Url::parse(url) else {
            return cfg;
        };
        for (k, v) in url.query_pairs() {
            match k.as_ref() {
                "sslmode" | "ssl-mode" | "ssl_mode" => {
                    if let Some(mode) = TlsMode::parse(&v) {
                        cfg.mode = mode;
                        cfg.invalid_mode = None;
                    } else {
                        // Do not fall back to disable — a typo must not go plaintext.
                        cfg.invalid_mode = Some(v.into_owned());
                    }
                }
                "sslrootcert" | "ssl-ca" | "ssl_ca" if !v.is_empty() => {
                    cfg.ca_path = Some(v.into_owned());
                }
                _ => {}
            }
        }
        cfg
    }

    /// Fail fast when the URL had an unrecognized TLS mode.
    ///
    /// # Errors
    ///
    /// Returns an error if `sslmode` / `ssl-mode` was present but not recognized.
    pub fn check_valid(&self) -> io::Result<()> {
        if let Some(bad) = &self.invalid_mode {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "unknown TLS mode `{bad}` (use disable, prefer, require, or verify-full)"
                ),
            ));
        }
        Ok(())
    }

    /// rustls config for the current mode (not meaningful for `Disable`).
    ///
    /// # Errors
    ///
    /// Returns an error if the CA file cannot be read or parsed, or if the URL
    /// TLS mode was unrecognized.
    pub fn client_config(&self) -> io::Result<ClientConfig> {
        self.check_valid()?;
        let provider = Arc::new(aws_lc_rs::default_provider());
        let builder = ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .map_err(io::Error::other)?;
        if self.mode != TlsMode::VerifyFull {
            return Ok(builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(NoVerify(provider)))
                .with_no_client_auth());
        }
        let mut roots = RootCertStore::empty();
        roots.add_parsable_certificates(
            NATIVE
                .get_or_init(|| rustls_native_certs::load_native_certs().certs)
                .clone(),
        );
        if let Some(path) = &self.ca_path {
            for cert in
                CertificateDer::pem_file_iter(path).map_err(io::Error::other)?
            {
                roots
                    .add(cert.map_err(io::Error::other)?)
                    .map_err(io::Error::other)?;
            }
        }
        Ok(builder.with_root_certificates(roots).with_no_client_auth())
    }
}

/// Accepts any certificate (libpq `require` semantics) but still checks handshake signatures.
#[derive(Debug)]
struct NoVerify(Arc<CryptoProvider>);

impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        msg: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls12_signature(
            msg,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        msg: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls13_signature(
            msg,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::{TlsConfig, TlsMode};
    use crate::db::{
        Database,
        connection::{Connection, ConnectionType, with_tls_query},
    };

    #[test]
    fn parses_mode_and_ca_from_url() {
        let c = TlsConfig::from_url(
            "postgres://u@h:5432/db?sslmode=verify-full&sslrootcert=/ca.pem",
        );
        assert_eq!(c.mode, TlsMode::VerifyFull);
        assert_eq!(c.ca_path.as_deref(), Some("/ca.pem"));
        assert_eq!(
            TlsConfig::from_url("mysql://u@h/db?ssl-mode=REQUIRED").mode,
            TlsMode::Require
        );
        assert_eq!(
            TlsConfig::from_url("postgres://u@h/db").mode,
            TlsMode::Disable
        );
        assert!(
            TlsConfig::from_url("postgres://u@h/db")
                .check_valid()
                .is_ok()
        );
    }

    #[test]
    fn unknown_sslmode_is_rejected() {
        let c = TlsConfig::from_url("postgres://u@h/db?sslmode=typo");
        assert!(c.check_valid().is_err());
        assert!(c.client_config().is_err());
        let msg = c.check_valid().unwrap_err().to_string();
        assert!(msg.contains("typo"), "{msg}");
    }

    #[test]
    fn missing_ca_file_is_an_error() {
        let c = TlsConfig {
            mode: TlsMode::VerifyFull,
            ca_path: Some("/nonexistent/ca.pem".into()),
            ..TlsConfig::default()
        };
        assert!(c.client_config().is_err());
    }

    // ---- live tests: need `just docker-tls-up` (includes plain Postgres on 5432) ----

    use ConnectionType::{Mysql, Postgres};

    /// (type, host port, user, db) of the TLS-only servers.
    const TLS_ONLY: [(ConnectionType, &str, &str, &str); 2] = [
        (Postgres, "15432", "d7s_user", "d7s_test"),
        (Mysql, "3308", "root", "d7s_test"),
    ];
    /// Same engines without TLS configured.
    const PLAIN: [(ConnectionType, &str, &str, &str); 2] = [
        (Postgres, "5432", "d7s_user", "d7s_test"),
        (Mysql, "3310", "root", "d7s_test"),
    ];

    fn cert(name: &str) -> String {
        format!("{}/docker/certs/{name}", env!("CARGO_MANIFEST_DIR"))
    }

    fn db(
        t: ConnectionType,
        host: &str,
        port: &str,
        user: &str,
        dbname: &str,
        mode: &str,
        ca: &str,
    ) -> Box<dyn Database> {
        let base = format!("{t}://{user}@{host}:{port}/{dbname}");
        Connection {
            r#type: t,
            url: with_tls_query(t, &base, mode, ca),
            password: Some("d7s_password".into()),
            ..Connection::default()
        }
        .to_database()
    }

    async fn ok(
        (t, port, user, dbname): (ConnectionType, &str, &str, &str),
        mode: &str,
        ca: &str,
    ) -> bool {
        db(t, "localhost", port, user, dbname, mode, ca)
            .test()
            .await
    }

    /// Ask the server itself whether this session is encrypted.
    async fn server_says_encrypted(
        (t, port, user, dbname): (ConnectionType, &str, &str, &str),
        mode: &str,
        ca: &str,
    ) -> bool {
        let sql = if t == Postgres {
            "SELECT CASE WHEN ssl THEN 'yes' ELSE 'no' END \
             FROM pg_stat_ssl WHERE pid = pg_backend_pid()"
        } else {
            "SELECT CASE WHEN VARIABLE_VALUE <> '' THEN 'yes' ELSE 'no' END \
             FROM information_schema.SESSION_STATUS \
             WHERE VARIABLE_NAME = 'SSL_CIPHER'"
        };
        let rows = db(t, "localhost", port, user, dbname, mode, ca)
            .execute_sql(sql)
            .await
            .unwrap_or_else(|e| panic!("{t} {mode}: {e}"));
        rows.first()
            .and_then(|r| r.values.first())
            .is_some_and(|v| v == "yes")
    }

    #[tokio::test]
    #[ignore = "needs docker TLS services"]
    async fn live_server_confirms_session_is_encrypted() {
        let ca = cert("ca.crt");
        for s in TLS_ONLY {
            for (mode, ca) in [
                ("require", ""),
                ("prefer", ""),
                ("verify-full", ca.as_str()),
            ] {
                assert!(
                    server_says_encrypted(s, mode, ca).await,
                    "{} {mode} not encrypted",
                    s.0
                );
            }
        }
        // `prefer` against a plain server must stay plain, and say so.
        for s in PLAIN {
            assert!(
                !server_says_encrypted(s, "prefer", "").await,
                "{} prefer",
                s.0
            );
            assert!(!server_says_encrypted(s, "disable", "").await);
        }
    }

    #[tokio::test]
    #[ignore = "needs docker TLS services"]
    async fn live_tls_only_servers_reject_plain_text() {
        let ca = cert("ca.crt");
        for s in TLS_ONLY {
            assert!(!ok(s, "disable", "").await, "{} disable", s.0);
            assert!(ok(s, "require", "").await, "{} require", s.0);
            assert!(ok(s, "prefer", "").await, "{} prefer", s.0);
            assert!(ok(s, "verify-full", &ca).await, "{} verify ok", s.0);
        }
    }

    #[tokio::test]
    #[ignore = "needs docker TLS services"]
    async fn live_require_fails_but_prefer_falls_back_on_plain_servers() {
        for s in PLAIN {
            assert!(ok(s, "disable", "").await, "{} disable", s.0);
            assert!(ok(s, "prefer", "").await, "{} prefer", s.0);
            assert!(!ok(s, "require", "").await, "{} require", s.0);
        }
    }

    #[tokio::test]
    #[ignore = "needs docker TLS services"]
    async fn live_verify_full_rejects_bad_trust() {
        let (good, other) = (cert("ca.crt"), cert("other-ca.crt"));
        for s in TLS_ONLY {
            // No CA given: private CA is not in the system roots.
            assert!(!ok(s, "verify-full", "").await, "{} no CA", s.0);
            assert!(!ok(s, "verify-full", &other).await, "{} wrong CA", s.0);
            assert!(
                !ok(s, "verify-full", "/nonexistent/ca.pem").await,
                "{} bad path",
                s.0
            );
            // Right CA, but cert only names `localhost`, not 127.0.0.1.
            let (t, port, user, dbname) = s;
            let wrong_host =
                db(t, "127.0.0.1", port, user, dbname, "verify-full", &good);
            assert!(!wrong_host.test().await, "{t} host mismatch");
            // ...while require (no verification) accepts the same address.
            let r = db(t, "127.0.0.1", port, user, dbname, "require", "");
            assert!(r.test().await, "{t} require via IP");
        }
    }
}
