use crate::{
    options::Options,
    tls::{fingerprint, NETWORK_TIMEOUT},
    transport::ServerState,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use stegrdb_relay::RelayError;
use tokio::task::JoinHandle;

// 登録は60秒で失効する。15秒ごとに更新し、一時的なHTTP障害を許容する。
const REFRESH_INTERVAL: Duration = Duration::from_secs(15);
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
#[derive(Serialize, Deserialize)]
struct Registration {
    room: String,
    node_id: String,
    address: SocketAddr,
    certificate_sha256: String,
}
#[derive(Deserialize)]
struct Directory {
    peers: Vec<Registration>,
}
struct Discovery {
    client: Client,
    url: reqwest::Url,
    token: String,
    registration: Registration,
    state: Arc<ServerState>,
}

pub fn start(options: &Options, state: Arc<ServerState>) -> Result<Option<JoinHandle<()>>, RelayError> {
    let Some(settings) = &options.discovery else {
        return Ok(None);
    };
    let url = reqwest::Url::parse(&settings.url).map_err(|_| RelayError::permanent("接続情報交換URLが不正です"))?;
    let local = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    if (url.scheme() != "https" && !(url.scheme() == "http" && local))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || settings.advertise.port() == 0
    {
        return Err(RelayError::permanent("接続情報交換はHTTPSで指定してください（HTTPはローカル試験のみ）"));
    }
    let token = std::env::var(&settings.token_env).map_err(|_| RelayError::permanent("接続情報交換用トークンの環境変数が未設定です"))?;
    if token.len() < 32 {
        return Err(RelayError::permanent("接続情報交換用トークンは32文字以上で指定してください"));
    }
    let certificate = std::fs::read(&options.certificate).map_err(|_| RelayError::permanent("証明書を読み込めません"))?;
    let registration = Registration {
        room: state.context.channel.clone(),
        node_id: state.context.node_id.clone(),
        address: settings.advertise,
        certificate_sha256: hex(&fingerprint(&certificate)),
    };
    let client =
        Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(NETWORK_TIMEOUT).build().map_err(|_| RelayError::permanent("HTTPSクライアントを作成できません"))?;
    let discovery = Discovery {
        client,
        url,
        token,
        registration,
        state,
    };
    Ok(Some(tokio::spawn(async move {
        let mut interval = tokio::time::interval(REFRESH_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if discovery.refresh().await.is_err() {
                eprintln!("接続情報の更新に失敗しました。既存のP2P接続を維持して再試行します");
            }
        }
    })))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
impl Discovery {
    async fn refresh(&self) -> Result<(), ()> {
        let mut response = self.client.post(self.url.clone()).bearer_auth(&self.token).json(&self.registration).send().await.map_err(|_| ())?.error_for_status().map_err(|_| ())?;
        let mut contents = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
            if contents.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(());
            }
            contents.extend_from_slice(&chunk);
        }
        let directory: Directory = serde_json::from_slice(&contents).map_err(|_| ())?;
        if directory.peers.len() > crate::options::MAX_PEERS + 1 {
            return Err(());
        }
        let mut addresses = self.state.addresses.write().expect("address lock");
        for peer in directory.peers {
            let identity = self.state.identities.iter().find(|(_, node)| **node == peer.node_id);
            if peer.room == self.state.context.channel && peer.address.port() != 0 && identity.is_some_and(|(certificate, _)| hex(certificate) == peer.certificate_sha256) {
                addresses.insert(peer.node_id, peer.address);
            }
        }
        Ok(())
    }
}
