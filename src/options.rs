use serde::Deserialize;
use std::{collections::HashSet, net::SocketAddr, path::PathBuf};
use stegrdb_relay::{RelayContext, RelayError};

// ノード当たり64MiBを上限にし、接続先の増加も明示的に制限する。
pub const MAX_QUEUE_BYTES: usize = 64 * 1024 * 1024;
pub const DEFAULT_QUEUE_CAPACITY: usize = 4096;
pub const MAX_PEERS: usize = 32;
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Peer {
    pub node_id: String,
    #[serde(default)]
    pub address: Option<SocketAddr>,
    pub certificate: PathBuf,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryOptions {
    pub url: String,
    pub token_env: String,
    pub advertise: SocketAddr,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub listen: SocketAddr,
    pub certificate: PathBuf,
    pub private_key: PathBuf,
    pub peers: Vec<Peer>,
    #[serde(default)]
    pub discovery: Option<DiscoveryOptions>,
    #[serde(default = "default_capacity")]
    pub queue_capacity: usize,
}
fn default_capacity() -> usize {
    DEFAULT_QUEUE_CAPACITY
}
impl Options {
    pub fn validate(&self, context: &RelayContext) -> Result<(), RelayError> {
        context.validate()?;
        if self.peers.is_empty() || self.peers.len() > MAX_PEERS || !(1..=65536).contains(&self.queue_capacity) {
            return Err(RelayError::permanent("P2Pの接続先数または受信キュー上限が範囲外です"));
        }
        let mut names = HashSet::new();
        let mut addresses = HashSet::new();
        for peer in &self.peers {
            RelayContext {
                node_id: peer.node_id.clone(),
                channel: context.channel.clone(),
            }
            .validate()?;
            if peer.node_id == context.node_id
                || !names.insert(&peer.node_id)
                || peer.address.is_some_and(|address| !addresses.insert(address) || address.port() == 0)
                || (peer.address.is_none() && self.discovery.is_none())
            {
                return Err(RelayError::permanent("P2Pの接続先が自ノード、重複、またはポート未指定です"));
            }
        }
        Ok(())
    }
}
