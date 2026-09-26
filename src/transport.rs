use crate::{
    options::Peer,
    queue::Queue,
    tls::{authenticated_node, NETWORK_TIMEOUT},
};
use amitoki_plugin_sdk::wire::{read_message, write_message, MAX_BATCH};
use amitoki_relay::{Frame, RelayContext, RelayError};
use quinn::{Connection, Endpoint};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
};
use tokio::{
    sync::{Mutex as AsyncMutex, Semaphore},
    task::JoinSet,
};

#[derive(Serialize, Deserialize)]
struct Publish {
    channel: String,
    node_id: String,
    frames: Vec<Frame>,
}
#[derive(Serialize, Deserialize)]
enum Accepted {
    Ok,
    Error { retryable: bool },
}
pub struct ServerState {
    pub context: RelayContext,
    pub queue: Mutex<Queue>,
    pub identities: HashMap<Vec<u8>, String>,
    pub addresses: RwLock<HashMap<String, std::net::SocketAddr>>,
}
pub struct PeerSender {
    pub peer: Peer,
    pub endpoint: Endpoint,
    pub connection: AsyncMutex<Option<Connection>>,
    pub state: Arc<ServerState>,
}

pub async fn accept_connections(endpoint: Endpoint, state: Arc<ServerState>) {
    // 各相手の接続と再接続中の接続を許容し、それ以上は受け付けない。
    let permits = Arc::new(Semaphore::new(state.identities.len() * 2));
    let mut sessions = JoinSet::new();
    loop {
        tokio::select! {
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break; };
                let Ok(permit) = permits.clone().try_acquire_owned() else { incoming.refuse(); continue; };
                let state = state.clone();
                sessions.spawn(async move {
                    let _permit = permit;
                    if let Ok(Ok(connection)) = tokio::time::timeout(NETWORK_TIMEOUT, incoming).await {
                        if let Some(node) = authenticated_node(&connection, &state.identities) { serve_connection(connection, node, state).await; }
                        else { connection.close(1u32.into(), b"unauthorized"); }
                    }
                });
            }
            _ = sessions.join_next(), if !sessions.is_empty() => {}
        }
    }
}

async fn serve_connection(connection: Connection, node: String, state: Arc<ServerState>) {
    while let Ok((mut output, mut input)) = connection.accept_bi().await {
        let exchange = async {
            let request: Publish = read_message(&mut input).await?;
            let accepted = if request.channel != state.context.channel || request.node_id != node || request.frames.len() > MAX_BATCH {
                Accepted::Error { retryable: false }
            } else {
                match state.queue.lock().expect("queue lock").accept(&node, request.frames) {
                    Ok(()) => Accepted::Ok,
                    Err(error) => Accepted::Error { retryable: error.is_retryable() },
                }
            };
            write_message(&mut output, &accepted).await?;
            output.finish().map_err(std::io::Error::other)
        };
        if !matches!(tokio::time::timeout(NETWORK_TIMEOUT, exchange).await, Ok(Ok(()))) {
            connection.close(2u32.into(), b"invalid request");
            break;
        }
    }
}

impl PeerSender {
    pub async fn publish(&self, frames: &[Frame]) -> Result<(), RelayError> {
        let mut cached = self.connection.lock().await;
        if cached.as_ref().is_some_and(|connection| connection.close_reason().is_some()) {
            *cached = None;
        }
        let outcome = tokio::time::timeout(NETWORK_TIMEOUT, self.exchange(&mut cached, frames)).await;
        match outcome {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                *cached = None;
                Err(error)
            },
            Err(_) => {
                *cached = None;
                Err(RelayError::retryable("P2Pの応答待ちがタイムアウトしました"))
            },
        }
    }

    async fn exchange(&self, cached: &mut Option<Connection>, frames: &[Frame]) -> Result<(), RelayError> {
        let address = *self.state.addresses.read().expect("address lock").get(&self.peer.node_id).ok_or_else(|| RelayError::retryable("接続先のアドレスが未登録です"))?;
        if cached.as_ref().is_some_and(|connection| connection.remote_address() != address) {
            *cached = None;
        }
        if cached.is_none() {
            let connection = self.endpoint.connect(address, "stegrdb.invalid").map_err(|_| RelayError::permanent("P2Pの接続先が不正です"))?.await.map_err(|_| interrupted())?;
            if authenticated_node(&connection, &self.state.identities).as_deref() != Some(&self.peer.node_id) {
                connection.close(1u32.into(), b"wrong identity");
                return Err(RelayError::permanent("接続先の証明書が指定ノードと一致しません"));
            }
            *cached = Some(connection);
        }
        let connection = cached.as_ref().expect("connected");
        let (mut output, mut input) = connection.open_bi().await.map_err(|_| interrupted())?;
        let request = Publish {
            channel: self.state.context.channel.clone(),
            node_id: self.state.context.node_id.clone(),
            frames: frames.to_vec(),
        };
        write_message(&mut output, &request).await.map_err(|_| interrupted())?;
        output.finish().map_err(|_| interrupted())?;
        match read_message(&mut input).await.map_err(|_| interrupted())? {
            Accepted::Ok => Ok(()),
            Accepted::Error { retryable: true } => Err(RelayError::retryable("接続先の受信キューが満杯です")),
            Accepted::Error { retryable: false } => Err(RelayError::permanent("接続先がノード・channel・フレームを拒否しました")),
        }
    }
}
fn interrupted() -> RelayError {
    RelayError::retryable("P2P接続が切断されました")
}
