mod discovery;
pub mod identity;
pub mod manifest;
mod options;
mod queue;
mod session;
mod tls;
mod transport;

use amitoki_plugin_sdk::wire::MAX_BATCH;
use amitoki_relay::{Delivery, Frame, Receipt, Relay, RelayContext, RelayError, RelayPlugin};
use async_trait::async_trait;
use futures_util::future::join_all;
use options::Options;
use serde_json::Value;
use std::sync::{Arc, Mutex, RwLock};
use tokio::{sync::Mutex as AsyncMutex, task::JoinHandle};
use transport::{PeerSender, ServerState};

pub struct P2pPlugin;
struct P2pRelay {
    _lease: std::fs::File,
    endpoint: quinn::Endpoint,
    state: Arc<ServerState>,
    peers: Vec<PeerSender>,
    listener: JoinHandle<()>,
    discovery: Option<JoinHandle<()>>,
}
impl Drop for P2pRelay {
    fn drop(&mut self) {
        self.endpoint.close(0u32.into(), b"shutdown");
        self.listener.abort();
        if let Some(task) = &self.discovery {
            task.abort();
        }
    }
}
#[async_trait]
impl RelayPlugin for P2pPlugin {
    fn name(&self) -> &'static str {
        "p2p"
    }
    async fn connect(&self, context: RelayContext, value: Value) -> Result<Arc<dyn Relay>, RelayError> {
        manifest::manifest().validate_options(&value)?;
        let options: Options = serde_json::from_value(value).map_err(|_| RelayError::permanent("P2P設定のアドレスまたは型が不正です"))?;
        options.validate(&context)?;
        let lease = session::claim_node(&context)?;
        let tls = tls::create_endpoint(&options)?;
        let endpoint = tls.endpoint;
        let state = Arc::new(ServerState {
            context,
            queue: Mutex::new(queue::Queue::new(options.queue_capacity)),
            identities: tls.identities,
            addresses: RwLock::new(options.peers.iter().filter_map(|peer| peer.address.map(|address| (peer.node_id.clone(), address))).collect()),
        });
        let discovery = discovery::start(&options, state.clone())?;
        let listener = tokio::spawn(transport::accept_connections(endpoint.clone(), state.clone()));
        let peers = options
            .peers
            .into_iter()
            .map(|peer| PeerSender {
                peer,
                endpoint: endpoint.clone(),
                connection: AsyncMutex::new(None),
                state: state.clone(),
            })
            .collect();
        Ok(Arc::new(P2pRelay {
            _lease: lease,
            endpoint,
            state,
            peers,
            listener,
            discovery,
        }))
    }
}
#[async_trait]
impl Relay for P2pRelay {
    async fn publish(&self, frames: &[Frame]) -> Result<(), RelayError> {
        for frame in frames {
            frame.validate()?;
        }
        for batch in frames.chunks(MAX_BATCH) {
            let outcomes = join_all(self.peers.iter().map(|peer| peer.publish(batch))).await;
            for outcome in outcomes {
                outcome?;
            }
        }
        Ok(())
    }
    async fn receive(&self, limit: usize) -> Result<Vec<Delivery>, RelayError> {
        Ok(self.state.queue.lock().expect("queue lock").receive(limit.min(MAX_BATCH)))
    }
    async fn acknowledge(&self, receipts: &[Receipt]) -> Result<(), RelayError> {
        self.state.queue.lock().expect("queue lock").acknowledge(receipts);
        Ok(())
    }
}
