//! TLS settings for server connections, read from the connection URL query:
//! `sslmode=` (Postgres) / `ssl-mode=` (MySQL) and `sslrootcert=` (CA bundle path).

use std::{io, sync::Arc};

use rustls::{
    ClientConfig, DigitallySignedStruct, Error as TlsError, RootCertStore,
    SignatureScheme,
    client::danger::{
        HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
    },
    crypto::{CryptoProvider, aws_lc_rs},
    pki_types::{CertificateDer, ServerName, UnixTime, pem::PemObject},
};

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
            "prefer" | "preferred" | "allow" => Self::Prefer,
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
                    cfg.mode = TlsMode::parse(&v).unwrap_or(cfg.mode);
                }
                "sslrootcert" | "ssl-ca" | "ssl_ca" if !v.is_empty() => {
                    cfg.ca_path = Some(v.into_owned());
                }
                _ => {}
            }
        }
        cfg
    }

    /// rustls config for the current mode (not meaningful for `Disable`).
    ///
    /// # Errors
    ///
    /// Returns an error if the CA file cannot be read or parsed.
    pub fn client_config(&self) -> io::Result<ClientConfig> {
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
            rustls_native_certs::load_native_certs().certs,
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
    }
}
