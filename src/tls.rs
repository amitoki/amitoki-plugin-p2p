use crate::options::Options;
use quinn::{
    crypto::rustls::{QuicClientConfig, QuicServerConfig},
    Endpoint,
};
use rustls::{
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    RootCertStore,
};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc, time::Duration};
use stegrdb_relay::RelayError;

// 接続が途切れた相手への再接続を妨げず、半開き接続を回収する。
pub const NETWORK_TIMEOUT: Duration = Duration::from_secs(5);
const KEEP_ALIVE: Duration = Duration::from_secs(2);
const MAX_STREAMS: u32 = 4;
pub struct TlsEndpoint {
    pub endpoint: Endpoint,
    pub identities: HashMap<Vec<u8>, String>,
}

pub fn fingerprint(certificate: &[u8]) -> Vec<u8> {
    Sha256::digest(certificate).to_vec()
}
pub fn create_endpoint(options: &Options) -> Result<TlsEndpoint, RelayError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let certificate = std::fs::read(&options.certificate).map_err(|_| RelayError::permanent("自ノードの証明書を読めません"))?;
    let key = std::fs::read(&options.private_key).map_err(|_| RelayError::permanent("自ノードの秘密鍵を読めません"))?;
    let own_fingerprint = fingerprint(&certificate);
    let certificates = vec![CertificateDer::from(certificate)];
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key));
    let mut roots = RootCertStore::empty();
    let mut identities = HashMap::new();
    for peer in &options.peers {
        let certificate = std::fs::read(&peer.certificate).map_err(|_| RelayError::permanent("接続先の証明書を読めません"))?;
        let fingerprint = fingerprint(&certificate);
        if fingerprint == own_fingerprint || identities.insert(fingerprint, peer.node_id.clone()).is_some() {
            return Err(RelayError::permanent("各ノードに別々の証明書を指定してください"));
        }
        roots.add(CertificateDer::from(certificate)).map_err(|_| RelayError::permanent("接続先の証明書が不正です"))?;
    }
    let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots.clone())).build().map_err(|_| RelayError::permanent("クライアント認証を構成できません"))?;
    let mut server = rustls::ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(certificates.clone(), key.clone_key())
        .map_err(|_| RelayError::permanent("証明書と秘密鍵が不正です"))?;
    let mut client = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(certificates, key)
        .map_err(|_| RelayError::permanent("クライアント証明書が不正です"))?;
    server.alpn_protocols = vec![b"stegrdb-p2p/1".to_vec()];
    client.alpn_protocols = server.alpn_protocols.clone();
    let mut transport = quinn::TransportConfig::default();
    transport
        .max_concurrent_bidi_streams(MAX_STREAMS.into())
        .max_concurrent_uni_streams(0u32.into())
        .max_idle_timeout(Some(NETWORK_TIMEOUT.try_into().expect("valid timeout")))
        .keep_alive_interval(Some(KEEP_ALIVE));
    let mut server = quinn::ServerConfig::with_crypto(Arc::new(
        QuicServerConfig::try_from(server).map_err(|_| RelayError::permanent("QUICサーバを構成できません"))?,
    ));
    server.transport_config(Arc::new(transport));
    let mut endpoint = Endpoint::server(server, options.listen).map_err(|_| RelayError::permanent("P2Pの待受アドレスを使用できません"))?;
    endpoint.set_default_client_config(quinn::ClientConfig::new(Arc::new(
        QuicClientConfig::try_from(client).map_err(|_| RelayError::permanent("QUICクライアントを構成できません"))?,
    )));
    Ok(TlsEndpoint { endpoint, identities })
}

pub fn authenticated_node(connection: &quinn::Connection, identities: &HashMap<Vec<u8>, String>) -> Option<String> {
    let identity = connection.peer_identity()?.downcast::<Vec<CertificateDer<'static>>>().ok()?;
    identities.get(&fingerprint(identity.first()?.as_ref())).cloned()
}
