//! Authenticated external gRPC client-bundle loading (BR-238).

use serde::Deserialize;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use url::{Host, Url};
use zeroize::Zeroizing;

/// Validated material for one authenticated external-v1 connection.
///
/// This type intentionally has no `Debug` implementation: two fields contain
/// production credentials and must never be formatted into logs.
pub struct ClientBundleConfig {
    pub endpoint_uri: String,
    pub tls_server_name: String,
    pub ca_pem: Vec<u8>,
    pub certificate_pem: Vec<u8>,
    pub private_key_pem: Zeroizing<Vec<u8>>,
    pub bearer_token: Zeroizing<String>,
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum BundleError {
    #[error("client bundle root is invalid")]
    InvalidRoot,
    #[error("client bundle manifest is invalid")]
    InvalidManifest,
    #[error("client bundle protocol version is unsupported")]
    UnsupportedProtocol,
    #[error("client bundle endpoint is invalid")]
    InvalidEndpoint,
    #[error("client bundle TLS server name is invalid")]
    InvalidTlsServerName,
    #[error("client bundle {role} path escapes the bundle root")]
    PathEscape { role: BundleFileRole },
    #[error("client bundle {role} must be a non-empty regular file")]
    InvalidFile { role: BundleFileRole },
    #[error("client bundle bearer token is invalid")]
    InvalidBearerToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleFileRole {
    Manifest,
    CertificateAuthority,
    ClientCertificate,
    ClientPrivateKey,
    BearerToken,
}

impl fmt::Display for BundleFileRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Manifest => "manifest",
            Self::CertificateAuthority => "certificate authority",
            Self::ClientCertificate => "client certificate",
            Self::ClientPrivateKey => "client private key",
            Self::BearerToken => "bearer token",
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BundleManifest {
    endpoint: String,
    tls_server_name: String,
    ca: String,
    certificate: String,
    private_key: String,
    bearer_token: String,
    protocol_version: u32,
}

pub fn load(path: &Path) -> Result<ClientBundleConfig, BundleError> {
    let root = canonical_bundle_root(path)?;
    let manifest_bytes = read_bundle_file(
        &root,
        Path::new("connection.json"),
        BundleFileRole::Manifest,
    )?;
    let manifest: BundleManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|_| BundleError::InvalidManifest)?;
    if manifest.protocol_version != 1 {
        return Err(BundleError::UnsupportedProtocol);
    }
    let endpoint_uri = normalize_https_endpoint(&manifest.endpoint)?;
    validate_tls_server_name(&manifest.tls_server_name)?;

    let ca_pem =
        read_declared_file(&root, &manifest.ca, BundleFileRole::CertificateAuthority)?.to_vec();
    let certificate_pem = read_declared_file(
        &root,
        &manifest.certificate,
        BundleFileRole::ClientCertificate,
    )?
    .to_vec();
    let private_key_pem = read_declared_file(
        &root,
        &manifest.private_key,
        BundleFileRole::ClientPrivateKey,
    )?;
    let token_bytes =
        read_declared_file(&root, &manifest.bearer_token, BundleFileRole::BearerToken)?;
    let token_text =
        std::str::from_utf8(&token_bytes).map_err(|_| BundleError::InvalidBearerToken)?;
    let mut bearer_token = Zeroizing::new(token_text.to_owned());
    let trimmed_length = bearer_token.trim_end_matches(['\r', '\n']).len();
    bearer_token.truncate(trimmed_length);
    if bearer_token.is_empty() || bearer_token.chars().any(char::is_whitespace) {
        return Err(BundleError::InvalidBearerToken);
    }

    Ok(ClientBundleConfig {
        endpoint_uri,
        tls_server_name: manifest.tls_server_name,
        ca_pem,
        certificate_pem,
        private_key_pem,
        bearer_token,
    })
}

fn canonical_bundle_root(path: &Path) -> Result<PathBuf, BundleError> {
    let root = fs::canonicalize(path).map_err(|_| BundleError::InvalidRoot)?;
    let metadata = fs::metadata(&root).map_err(|_| BundleError::InvalidRoot)?;
    if !metadata.is_dir() {
        return Err(BundleError::InvalidRoot);
    }
    Ok(root)
}

fn read_declared_file(
    root: &Path,
    declared_path: &str,
    role: BundleFileRole,
) -> Result<Zeroizing<Vec<u8>>, BundleError> {
    if declared_path.is_empty() || Path::new(declared_path).is_absolute() {
        return Err(BundleError::InvalidFile { role });
    }
    read_bundle_file(root, Path::new(declared_path), role)
}

fn read_bundle_file(
    root: &Path,
    declared_path: &Path,
    role: BundleFileRole,
) -> Result<Zeroizing<Vec<u8>>, BundleError> {
    let candidate = root.join(declared_path);
    let before = fs::symlink_metadata(&candidate).map_err(|_| BundleError::InvalidFile { role })?;
    if !before.is_file() || before.file_type().is_symlink() || before.len() == 0 {
        return Err(BundleError::InvalidFile { role });
    }
    let canonical = fs::canonicalize(&candidate).map_err(|_| BundleError::InvalidFile { role })?;
    if !canonical.starts_with(root) {
        return Err(BundleError::PathEscape { role });
    }

    let bytes =
        Zeroizing::new(fs::read(&canonical).map_err(|_| BundleError::InvalidFile { role })?);
    let after = fs::symlink_metadata(&candidate).map_err(|_| BundleError::InvalidFile { role })?;
    if bytes.is_empty()
        || !after.is_file()
        || after.file_type().is_symlink()
        || before.len() != after.len()
        || after.len() != u64::try_from(bytes.len()).unwrap_or(u64::MAX)
        || before.modified().ok() != after.modified().ok()
    {
        return Err(BundleError::InvalidFile { role });
    }
    Ok(bytes)
}

fn normalize_https_endpoint(endpoint: &str) -> Result<String, BundleError> {
    if endpoint.is_empty() || endpoint.trim() != endpoint {
        return Err(BundleError::InvalidEndpoint);
    }
    let endpoint_uri = if endpoint.contains("://") {
        endpoint.to_owned()
    } else {
        format!("https://{endpoint}")
    };
    let parsed = Url::parse(&endpoint_uri).map_err(|_| BundleError::InvalidEndpoint)?;
    if parsed.scheme() != "https"
        || parsed.host().is_none()
        || parsed.port().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(BundleError::InvalidEndpoint);
    }
    Ok(parsed.origin().ascii_serialization())
}

fn validate_tls_server_name(server_name: &str) -> Result<(), BundleError> {
    if server_name.is_empty()
        || server_name.trim() != server_name
        || server_name.chars().any(char::is_whitespace)
        || Host::parse(server_name).is_err()
    {
        return Err(BundleError::InvalidTlsServerName);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestBundle {
        root: PathBuf,
    }

    impl TestBundle {
        fn valid() -> Self {
            let sequence = TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "TEST_CODE_grpc_client_bundle_{}_{}",
                std::process::id(),
                sequence
            ));
            fs::create_dir(&root).expect("create isolated test bundle");
            fs::write(root.join("ca.pem"), b"TEST_CODE_CA").expect("write test CA");
            fs::write(root.join("certificate.pem"), b"TEST_CODE_CERTIFICATE")
                .expect("write test certificate");
            fs::write(root.join("private-key.pem"), b"TEST_CODE_PRIVATE_KEY")
                .expect("write test private key");
            fs::write(root.join("bearer-token.txt"), b"TEST_CODE_BEARER_TOKEN\n")
                .expect("write test token");
            let bundle = Self { root };
            bundle.write_manifest(Self::valid_manifest());
            bundle
        }

        fn valid_manifest() -> serde_json::Value {
            json!({
                "endpoint": "127.0.0.1:50051",
                "tls_server_name": "magic-market.local",
                "ca": "ca.pem",
                "certificate": "certificate.pem",
                "private_key": "private-key.pem",
                "bearer_token": "bearer-token.txt",
                "protocol_version": 1
            })
        }

        fn path(&self) -> &Path {
            &self.root
        }

        fn write_manifest(&self, manifest: serde_json::Value) {
            fs::write(
                self.root.join("connection.json"),
                serde_json::to_vec(&manifest).expect("serialize test manifest"),
            )
            .expect("write test manifest");
        }
    }

    impl Drop for TestBundle {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).expect("remove isolated test bundle");
        }
    }

    #[test]
    fn rejects_protocol_versions_other_than_v1() {
        let bundle = TestBundle::valid();
        let mut manifest = TestBundle::valid_manifest();
        manifest["protocol_version"] = json!(2);
        bundle.write_manifest(manifest);

        let error = match load(bundle.path()) {
            Ok(_) => panic!("protocol v2 must fail closed"),
            Err(error) => error,
        };

        assert_eq!(error, BundleError::UnsupportedProtocol);
    }

    #[test]
    fn rejects_bundle_root_that_is_not_a_directory() {
        let bundle = TestBundle::valid();

        let error = match load(&bundle.path().join("ca.pem")) {
            Ok(_) => panic!("regular file bundle root must fail closed"),
            Err(error) => error,
        };

        assert_eq!(error, BundleError::InvalidRoot);
    }

    #[test]
    fn loads_valid_v1_bundle_and_normalizes_https_endpoint() {
        let bundle = TestBundle::valid();

        let config = load(bundle.path()).expect("valid v1 bundle");

        assert_eq!(config.endpoint_uri, "https://127.0.0.1:50051");
        assert_eq!(config.tls_server_name, "magic-market.local");
        assert_eq!(config.ca_pem, b"TEST_CODE_CA");
        assert_eq!(config.certificate_pem, b"TEST_CODE_CERTIFICATE");
        assert_eq!(&*config.private_key_pem, b"TEST_CODE_PRIVATE_KEY");
        assert_eq!(&*config.bearer_token, "TEST_CODE_BEARER_TOKEN");
    }

    #[test]
    fn rejects_declared_file_that_escapes_canonical_bundle_root() {
        let bundle = TestBundle::valid();
        let outside = bundle.path().with_extension("TEST_CODE_outside_ca");
        fs::write(&outside, b"TEST_CODE_OUTSIDE_CA").expect("write outside test CA");
        let mut manifest = TestBundle::valid_manifest();
        manifest["ca"] = json!(format!(
            "../{}",
            outside.file_name().unwrap().to_string_lossy()
        ));
        bundle.write_manifest(manifest);

        let result = load(bundle.path());
        fs::remove_file(outside).expect("remove outside test CA");
        let error = match result {
            Ok(_) => panic!("outside CA path must fail closed"),
            Err(error) => error,
        };

        assert_eq!(
            error,
            BundleError::PathEscape {
                role: BundleFileRole::CertificateAuthority
            }
        );
    }

    #[test]
    fn rejects_absolute_declared_path_even_when_it_points_inside_bundle() {
        let bundle = TestBundle::valid();
        let mut manifest = TestBundle::valid_manifest();
        manifest["ca"] = json!(bundle.path().join("ca.pem").to_string_lossy());
        bundle.write_manifest(manifest);

        let error = match load(bundle.path()) {
            Ok(_) => panic!("absolute declared CA path must fail closed"),
            Err(error) => error,
        };

        assert_eq!(
            error,
            BundleError::InvalidFile {
                role: BundleFileRole::CertificateAuthority
            }
        );
    }

    #[test]
    fn rejects_symlinked_manifest_even_when_target_is_inside_root() {
        let bundle = TestBundle::valid();
        let real_manifest = bundle.path().join("real-connection.json");
        fs::rename(bundle.path().join("connection.json"), &real_manifest)
            .expect("move test manifest");
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            "real-connection.json",
            bundle.path().join("connection.json"),
        )
        .expect("create test manifest symlink");

        let error = match load(bundle.path()) {
            Ok(_) => panic!("symlinked manifest must fail closed"),
            Err(error) => error,
        };

        assert_eq!(
            error,
            BundleError::InvalidFile {
                role: BundleFileRole::Manifest
            }
        );
    }

    #[test]
    fn rejects_non_regular_and_empty_declared_files() {
        let bundle = TestBundle::valid();
        fs::remove_file(bundle.path().join("certificate.pem")).expect("remove test certificate");
        fs::create_dir(bundle.path().join("certificate.pem"))
            .expect("create non-regular test certificate");
        let directory_error = match load(bundle.path()) {
            Ok(_) => panic!("directory certificate must fail closed"),
            Err(error) => error,
        };
        assert_eq!(
            directory_error,
            BundleError::InvalidFile {
                role: BundleFileRole::ClientCertificate
            }
        );

        fs::remove_dir(bundle.path().join("certificate.pem"))
            .expect("remove non-regular test certificate");
        fs::write(bundle.path().join("certificate.pem"), b"")
            .expect("write empty test certificate");
        let empty_error = match load(bundle.path()) {
            Ok(_) => panic!("empty certificate must fail closed"),
            Err(error) => error,
        };
        assert_eq!(
            empty_error,
            BundleError::InvalidFile {
                role: BundleFileRole::ClientCertificate
            }
        );
    }

    #[test]
    fn rejects_non_https_endpoint_and_invalid_tls_server_name() {
        let bundle = TestBundle::valid();
        let mut manifest = TestBundle::valid_manifest();
        manifest["endpoint"] = json!("http://127.0.0.1:50051");
        bundle.write_manifest(manifest);
        let endpoint_error = match load(bundle.path()) {
            Ok(_) => panic!("plaintext endpoint must fail closed"),
            Err(error) => error,
        };
        assert_eq!(endpoint_error, BundleError::InvalidEndpoint);

        let mut manifest = TestBundle::valid_manifest();
        manifest["tls_server_name"] = json!("https://magic-market.local");
        bundle.write_manifest(manifest);
        let name_error = match load(bundle.path()) {
            Ok(_) => panic!("URL-shaped TLS name must fail closed"),
            Err(error) => error,
        };
        assert_eq!(name_error, BundleError::InvalidTlsServerName);
    }

    #[test]
    fn errors_never_include_declared_secret_paths_or_secret_bytes() {
        const SECRET_MARKER: &str = "TEST_CODE_SECRET_MUST_NOT_APPEAR";
        let bundle = TestBundle::valid();
        let secret_path = format!("../{SECRET_MARKER}.pem");
        let mut manifest = TestBundle::valid_manifest();
        manifest["private_key"] = json!(secret_path);
        bundle.write_manifest(manifest);

        let key_error = match load(bundle.path()) {
            Ok(_) => panic!("secret path escape must fail closed"),
            Err(error) => error,
        };
        let key_text = format!("{key_error} {key_error:?}");
        assert!(!key_text.contains(SECRET_MARKER));

        let mut manifest = TestBundle::valid_manifest();
        manifest["bearer_token"] = json!("bearer-token.txt");
        bundle.write_manifest(manifest);
        fs::write(
            bundle.path().join("bearer-token.txt"),
            format!("{SECRET_MARKER} invalid"),
        )
        .expect("write invalid secret test token");
        let token_error = match load(bundle.path()) {
            Ok(_) => panic!("whitespace-bearing token must fail closed"),
            Err(error) => error,
        };
        let token_text = format!("{token_error} {token_error:?}");
        assert!(!token_text.contains(SECRET_MARKER));
    }
}
