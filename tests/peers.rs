use bytes::Bytes;
use serde_json::{json, Value};
use std::{net::UdpSocket, sync::Arc};
use stegrdb_plugin_p2p::{identity::create_identity, P2pPlugin};
use stegrdb_relay::{Frame, Relay, RelayContext, RelayPlugin};

struct Lab {
    directory: tempfile::TempDir,
    addresses: Vec<String>,
}
impl Lab {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let sockets: Vec<_> = (0..3).map(|_| UdpSocket::bind("127.0.0.1:0").unwrap()).collect();
        let addresses = sockets.iter().map(|socket| socket.local_addr().unwrap().to_string()).collect();
        for node in 0..3 {
            create_identity(&directory.path().join(node.to_string())).unwrap();
        }
        Self { directory, addresses }
    }
    fn options(&self, node: usize) -> Value {
        json!({"listen": self.addresses[node], "certificate":self.directory.path().join(node.to_string()).join("cert.der"),
            "private_key":self.directory.path().join(node.to_string()).join("key.der"),
            "peers":(0..3).filter(|other| *other != node).map(|other| json!({"node_id":other.to_string(),"address":self.addresses[other],"certificate":self.directory.path().join(other.to_string()).join("cert.der")})).collect::<Vec<_>>()})
    }
    fn context(&self, node: usize, channel: &str) -> RelayContext {
        RelayContext {
            node_id: node.to_string(),
            channel: format!("{channel}-{}", self.directory.path().file_name().unwrap().to_string_lossy()),
        }
    }
    async fn connect(&self, node: usize, channel: &str) -> Arc<dyn Relay> {
        P2pPlugin.connect(self.context(node, channel), self.options(node)).await.unwrap()
    }
}
fn frame(value: u8) -> Frame {
    Frame::new(Bytes::from(vec![value; 1514])).unwrap()
}

#[tokio::test]
async fn three_peers_deliver_in_order_and_deduplicate_retries_before_and_after_ack() {
    let lab = Lab::new();
    let peers = [
        lab.connect(0, "test").await,
        lab.connect(1, "test").await,
        lab.connect(2, "test").await,
    ];
    let frames = vec![frame(1), frame(2), frame(3)];
    peers[0].publish(&frames).await.unwrap();
    peers[0].publish(&frames).await.unwrap();
    assert!(peers[0].receive(10).await.unwrap().is_empty());
    for peer in &peers[1..] {
        let received = peer.receive(10).await.unwrap();
        assert_eq!(received.iter().map(|delivery| delivery.frame.clone()).collect::<Vec<_>>(), frames);
        assert_eq!(peer.receive(10).await.unwrap().len(), 3);
        let receipts: Vec<_> = received.into_iter().map(|delivery| delivery.receipt).collect();
        peer.acknowledge(&receipts).await.unwrap();
        peer.acknowledge(&receipts).await.unwrap();
    }
    peers[0].publish(&frames).await.unwrap();
    for peer in &peers[1..] {
        assert!(peer.receive(10).await.unwrap().is_empty());
    }
    peers[2].publish(&[frame(4)]).await.unwrap();
    assert_eq!(peers[0].receive(10).await.unwrap().len(), 1);
    assert_eq!(peers[1].receive(10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_different_channel_is_rejected_without_delivering_frames() {
    let lab = Lab::new();
    let a = lab.connect(0, "one").await;
    let b = lab.connect(1, "other").await;
    let _c = lab.connect(2, "one").await;
    assert!(!a.publish(&[frame(5)]).await.unwrap_err().is_retryable());
    assert!(b.receive(10).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_full_queue_rejects_a_batch_and_accepts_it_after_ack() {
    let lab = Lab::new();
    let a = lab.connect(0, "test").await;
    let mut options = lab.options(1);
    options["queue_capacity"] = json!(1);
    let b = P2pPlugin.connect(lab.context(1, "test"), options).await.unwrap();
    let _c = lab.connect(2, "test").await;
    a.publish(&[frame(6)]).await.unwrap();
    let next = frame(7);
    assert!(a.publish(std::slice::from_ref(&next)).await.unwrap_err().is_retryable());
    let received = b.receive(10).await.unwrap();
    assert_eq!(received.len(), 1);
    b.acknowledge(&[received[0].receipt.clone()]).await.unwrap();
    a.publish(std::slice::from_ref(&next)).await.unwrap();
    assert_eq!(b.receive(10).await.unwrap()[0].frame, next);
}

#[tokio::test]
async fn a_certificate_assigned_to_the_wrong_node_is_rejected() {
    let lab = Lab::new();
    let mut options = lab.options(0);
    let address = options["peers"][0]["address"].clone();
    options["peers"][0]["address"] = options["peers"][1]["address"].clone();
    options["peers"][1]["address"] = address;
    let a = P2pPlugin.connect(lab.context(0, "test"), options).await.unwrap();
    let b = lab.connect(1, "test").await;
    let c = lab.connect(2, "test").await;
    assert!(!a.publish(&[frame(8)]).await.unwrap_err().is_retryable());
    assert!(b.receive(10).await.unwrap().is_empty());
    assert!(c.receive(10).await.unwrap().is_empty());
}

#[tokio::test]
#[ignore = "scripts/test-signaling.shで実Redisと接続情報交換サーバを起動して実行"]
async fn discovery_connects_three_peers_and_payloads_continue_after_signaling_stops() {
    use std::time::Duration;
    let lab = Lab::new();
    let url = std::env::var("STEGRDB_TEST_SIGNALING_URL").unwrap();
    let room = uuid::Uuid::new_v4().to_string();
    let mut peers = Vec::new();
    for node in 0..3 {
        let mut options = lab.options(node);
        for peer in options["peers"].as_array_mut().unwrap() {
            peer.as_object_mut().unwrap().remove("address");
        }
        options["discovery"] = json!({"url":url,"token_env":"STEGRDB_TEST_SIGNALING_TOKEN","advertise":lab.addresses[node]});
        peers.push(
            P2pPlugin
                .connect(
                    RelayContext {
                        node_id: node.to_string(),
                        channel: room.clone(),
                    },
                    options,
                )
                .await
                .unwrap(),
        );
    }
    let initial = frame(9);
    // 登録の更新周期を跨いで、実際に全相手へ送れるまで待つ。
    const DISCOVERY_DEADLINE: Duration = Duration::from_secs(35);
    const DISCOVERY_POLL: Duration = Duration::from_millis(100);
    tokio::time::timeout(DISCOVERY_DEADLINE, async {
        loop {
            match peers[0].publish(std::slice::from_ref(&initial)).await {
                Ok(()) => break,
                Err(error) => assert!(error.is_retryable(), "{error}"),
            }
            tokio::time::sleep(DISCOVERY_POLL).await;
        }
    })
    .await
    .unwrap();
    for peer in &peers[1..] {
        let delivery = peer.receive(10).await.unwrap();
        assert_eq!(delivery.len(), 1);
        peer.acknowledge(&[delivery[0].receipt.clone()]).await.unwrap();
    }
    std::fs::write(std::env::var("STEGRDB_TEST_SIGNALING_STOP_FILE").unwrap(), "stop").unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while reqwest::get(&url).await.is_ok() {
            tokio::time::sleep(DISCOVERY_POLL).await;
        }
    })
    .await
    .unwrap();
    let payload = frame(10);
    peers[0].publish(std::slice::from_ref(&payload)).await.unwrap();
    for peer in &peers[1..] {
        assert_eq!(peer.receive(10).await.unwrap()[0].frame, payload);
    }
}

#[tokio::test]
async fn an_untrusted_certificate_cannot_receive_frames() {
    let lab = Lab::new();
    create_identity(&lab.directory.path().join("impostor")).unwrap();
    let a = lab.connect(0, "test").await;
    let mut options = lab.options(1);
    options["certificate"] = json!(lab.directory.path().join("impostor/cert.der"));
    options["private_key"] = json!(lab.directory.path().join("impostor/key.der"));
    let b = P2pPlugin.connect(lab.context(1, "test"), options).await.unwrap();
    let _c = lab.connect(2, "test").await;
    assert!(a.publish(&[frame(11)]).await.is_err());
    assert!(b.receive(10).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_second_process_identity_on_another_port_is_rejected() {
    let lab = Lab::new();
    let _first = lab.connect(0, "test").await;
    let mut options = lab.options(0);
    options["listen"] = json!("127.0.0.1:0");
    assert!(P2pPlugin.connect(lab.context(0, "test"), options).await.is_err());
}
