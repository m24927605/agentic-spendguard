//! HARDEN_08 per-tenant SVID contract tests.

use spendguard_output_predictor::plugin_svid::{
    paths_for_client_cert_id, subject_uri_for_tenant, tenant_from_subject_uri,
    validate_client_cert_id,
};
use uuid::Uuid;

#[test]
fn per_tenant_svid_subject_is_exact_spiffe_uri() {
    let tenant = Uuid::parse_str("018fcf9a-3d2d-7b37-9f21-0f27de0b20c1").unwrap();
    let subject = subject_uri_for_tenant(&tenant);
    assert_eq!(
        subject,
        "spiffe://spendguard.platform/predictor-client/018fcf9a-3d2d-7b37-9f21-0f27de0b20c1"
    );
    assert_eq!(tenant_from_subject_uri(&subject).unwrap(), tenant);
}

#[test]
fn client_cert_id_maps_to_bounded_subdirectory() {
    let base = std::path::Path::new("/etc/spendguard/plugin-client-svid");
    let paths = paths_for_client_cert_id(base, "tenant-a_01").unwrap();
    assert_eq!(
        paths.cert_pem,
        std::path::Path::new("/etc/spendguard/plugin-client-svid/tenant-a_01/tls.crt")
    );
    assert_eq!(
        paths.key_pem,
        std::path::Path::new("/etc/spendguard/plugin-client-svid/tenant-a_01/tls.key")
    );
    assert_eq!(
        paths.trust_ca_pem,
        std::path::Path::new("/etc/spendguard/plugin-client-svid/tenant-a_01/ca.crt")
    );
}

#[test]
fn client_cert_id_path_traversal_fails_closed() {
    for bad in [
        "../tenant",
        "tenant/one",
        "tenant.one",
        "tenant one",
        "..",
        "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
    ] {
        assert!(
            validate_client_cert_id(bad).is_err(),
            "unsafe client_cert_id must be rejected: {bad}"
        );
    }
}

#[tokio::test]
async fn plugin_client_presents_tenant_svid_over_real_mtls() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::Arc;

    use rcgen::{
        BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType,
        ExtendedKeyUsagePurpose, IsCa, KeyPair, SanType,
    };
    use sha2::Digest;
    use spendguard_output_predictor::endpoint_cache::PluginEndpoint;
    use spendguard_output_predictor::plugin_client::{PluginClient, PluginClientTls};
    use spendguard_output_predictor::proto::output_predictor_plugin::v1::{
        customer_predictor_server::{CustomerPredictor, CustomerPredictorServer},
        health_check_response, HealthCheckRequest, HealthCheckResponse, PredictRequest,
        PredictResponse,
    };
    use tempfile::TempDir;
    use tonic::transport::{Certificate as TonicCertificate, Identity, Server, ServerTlsConfig};
    use tonic::{Request, Response, Status};

    #[derive(Default)]
    struct MockPlugin;

    #[tonic::async_trait]
    impl CustomerPredictor for MockPlugin {
        async fn predict(
            &self,
            request: Request<PredictRequest>,
        ) -> Result<Response<PredictResponse>, Status> {
            let auth = request
                .peer_certs()
                .ok_or_else(|| Status::permission_denied("missing client cert"))?;
            if auth.is_empty() {
                return Err(Status::permission_denied("missing client cert"));
            }
            Ok(Response::new(PredictResponse {
                predicted_output_tokens: 42,
                confidence: 0.9,
                sample_size: 30,
                plugin_version: "test-plugin".into(),
                feature_hash: "feature-hash".into(),
            }))
        }

        async fn health_check(
            &self,
            _request: Request<HealthCheckRequest>,
        ) -> Result<Response<HealthCheckResponse>, Status> {
            Ok(Response::new(HealthCheckResponse {
                status: health_check_response::Status::Serving as i32,
                plugin_version: "test-plugin".into(),
                checked_at: None,
            }))
        }
    }

    fn ca_cert() -> (Certificate, KeyPair) {
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::default();
        params.distinguished_name = DistinguishedName::new();
        params
            .distinguished_name
            .push(DnType::CommonName, "spendguard-test-ca");
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let cert = params.self_signed(&key).unwrap();
        (cert, key)
    }

    fn signed_cert(
        ca: &Certificate,
        ca_key: &KeyPair,
        common_name: &str,
        sans: Vec<SanType>,
        eku: ExtendedKeyUsagePurpose,
    ) -> (String, String) {
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(vec![]).unwrap();
        params.distinguished_name = DistinguishedName::new();
        params
            .distinguished_name
            .push(DnType::CommonName, common_name);
        params.subject_alt_names = sans;
        params.extended_key_usages = vec![eku];
        let cert = params.signed_by(&key, ca, ca_key).unwrap();
        (cert.pem(), key.serialize_pem())
    }

    let tenant = Uuid::parse_str("018fcf9a-3d2d-7b37-9f21-0f27de0b20c1").unwrap();
    let client_cert_id = "tenant-018fcf9a";
    let (ca, ca_key) = ca_cert();
    let ca_pem = ca.pem();
    let (server_cert_pem, server_key_pem) = signed_cert(
        &ca,
        &ca_key,
        "localhost",
        vec![SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST))],
        ExtendedKeyUsagePurpose::ServerAuth,
    );
    let (client_cert_pem, client_key_pem) = signed_cert(
        &ca,
        &ca_key,
        "spendguard-predictor-client",
        vec![SanType::URI(
            subject_uri_for_tenant(&tenant).try_into().unwrap(),
        )],
        ExtendedKeyUsagePurpose::ClientAuth,
    );

    let tmp = TempDir::new().unwrap();
    let svid_dir = tmp.path().join("svid").join(client_cert_id);
    std::fs::create_dir_all(&svid_dir).unwrap();
    std::fs::write(svid_dir.join("tls.crt"), &client_cert_pem).unwrap();
    std::fs::write(svid_dir.join("tls.key"), &client_key_pem).unwrap();
    std::fs::write(svid_dir.join("ca.crt"), &ca_pem).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
    let tls = ServerTlsConfig::new()
        .identity(Identity::from_pem(server_cert_pem.clone(), server_key_pem))
        .client_ca_root(TonicCertificate::from_pem(ca_pem.clone()));
    let server = tokio::spawn(async move {
        Server::builder()
            .tls_config(tls)
            .unwrap()
            .add_service(CustomerPredictorServer::new(MockPlugin))
            .serve_with_incoming(incoming)
            .await
            .unwrap();
    });

    let server_der = {
        let (_, pem) = x509_parser::pem::parse_x509_pem(server_cert_pem.as_bytes()).unwrap();
        pem.contents
    };
    let endpoint = Arc::new(PluginEndpoint {
        plugin_endpoint_id: Uuid::new_v4(),
        tenant_id: tenant,
        endpoint_url: format!("https://127.0.0.1:{}", addr.port()),
        server_cert_fingerprint: hex::encode(sha2::Sha256::digest(&server_der)),
        client_cert_id: client_cert_id.into(),
        enabled: true,
    });
    let client = PluginClient::new(Some(PluginClientTls::PerTenantSvidDir {
        svid_dir: tmp.path().join("svid"),
    }))
    .unwrap();
    let response = client
        .predict(
            &tenant,
            endpoint,
            PredictRequest {
                spendguard_call_id: "018fcf9a-3d2d-7b37-9f21-0f27de0b20c2".into(),
                tenant_id: tenant.to_string(),
                model: "gpt-4o".into(),
                agent_id: "agent-a".into(),
                prompt_class: "chat_short".into(),
                input_tokens: 10,
                max_tokens_requested: 100,
                classifier_version: "v1alpha1".into(),
                prompt_class_fingerprint: "abc".into(),
                features: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(response.predicted_output_tokens, 42);
    server.abort();
}
