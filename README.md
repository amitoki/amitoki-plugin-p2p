# stegrdb-plugin-p2p

stegrdbの外部プロセス型P2Pプラグイン。相互認証したQUIC接続でEthernetフレームをクライアント間に直接送る。本体の再ビルドは不要。

任意のNext.jsサーバから接続先を取得できる。サーバへ送るのは部屋名・ノード名・IP/ポート・証明書のfingerprintだけ。パケット本体はVercelやRedisを通らない。

## 2ノードで始める

stegrdb 0.3以降を使う。GitHubのprivateリポジトリを読めるトークンを`STEGRDB_GITHUB_TOKEN`へ設定して実行する。

```bash
stegrdb plugin add p2p
stegrdb plugin describe p2p
~/.local/share/stegrdb/plugins/p2p/stegrdb-plugin-p2p identity --output ./node-a
```

別ノードでも`identity --output ./node-b`を実行する。`cert.der`だけを相手へ渡し、信頼する相手から受け取った証明書を配置する。`key.der`は自ノードだけが保持する。既存の鍵ディレクトリは上書きしない。

ノードaのstegrdb.toml例。アドレスと絶対パスは自分の環境へ変更する。ノードbではnode_id・自分の鍵・相手のアドレスと証明書を入れ替える。

```toml
node_id = "node-a"
channel = "example"
interface = "eth1"
[relay]
plugin = "p2p"
[relay.options]
listen = "0.0.0.0:7443"
certificate = "/home/user/node-a/cert.der"
private_key = "/home/user/node-a/key.der"
queue_capacity = 4096
[[relay.options.peers]]
node_id = "node-b"
address = "192.168.1.20:7443"
certificate = "/home/user/trusted/node-b.der"
[firewall]
policy = "whitelist"
rules = [{ type = "EtherType", value = 2054 }, { type = "EtherType", value = 2048 }]
```

このフィルタ例はARP・IPv4を許可する。中継LANとQUIC用のインターフェースは分ける。3台以上では各ノードのpeersへ自分以外の全相手を設定する。

同じ設定は`stegrdb plugin configure p2p`でも保存できる。順に証明書の絶対パス、秘密鍵の絶対パス、待受アドレス、接続先のJSON配列、キュー上限を入力する（項目順は表示に従う）。非対話では次のように指定する。

```bash
stegrdb plugin configure p2p \
  --set listen=0.0.0.0:7443 \
  --set certificate=/home/user/node-a/cert.der \
  --set private_key=/home/user/node-a/key.der \
  --set 'peers=[{"node_id":"node-b","address":"192.168.1.20:7443","certificate":"/home/user/trusted/node-b.der"}]'
stegrdb plugin validate p2p
stegrdb --config stegrdb.toml --check-config
```

CLIへ保存した項目は`relay.options`から省略できる。両方に書いた項目はTOML側を優先する。

## Next.jsを接続情報の交換に使う

`signaling/`がVercelへ配置できるNext.jsアプリ。Node.js 24以降を使い、Root Directoryを`signaling`へ設定する。状態はUpstash Redisへ保存し、Vercelのインスタンスのメモリには依存しない。実際のVercel・Upstash環境の作成やデプロイはこのリポジトリでは行っていない。

必要なサーバ環境変数:

- `UPSTASH_REDIS_REST_URL`: RedisのREST URL。
- `UPSTASH_REDIS_REST_TOKEN`: Redis用トークン。
- `SIGNALING_TOKEN`: 32文字以上のランダムな参加トークン。

トークンはVercelの環境変数へ設定し、Gitへコミットしない。`openssl rand -hex 32`で生成できる。同じトークンを各クライアントの`STEGRDB_SIGNALING_TOKEN`へ設定する。

```toml
[relay.options.discovery]
url = "https://your-project.vercel.app/api/peers"
token_env = "STEGRDB_SIGNALING_TOKEN"
advertise = "203.0.113.10:7443"
```

`advertise`には相手から到達できる自分のUDPアドレスを指定する。discoveryを使う場合、peersのaddressは省略できる。信頼する証明書とnode_idは事前に指定する。登録内容は60秒で失効し、15秒ごとに更新する。取得したfingerprintと信頼する証明書が一致した接続先だけを採用し、さらにQUIC接続時にも証明書を検証する。

交換サーバ停止後も取得済みアドレスで通信を続ける。新しいアドレスの取得・変更には交換サーバが必要になる。サーバはパケットや任意の追加フィールドを拒否する。

この版は相互に到達できるUDPアドレスを前提にする。NAT下ではポート転送などを用意する。STUN/TURNや自動hole punching、ブラウザのWebRTCクライアントは未実装。ブラウザだけでOSのLANフレームを中継する実装ではない。

## 配送と障害時の動作

各ノードは設定した全相手へ送信する。相手のメモリキューが受け付けた時点で送信成功となり、NIC注入の完了とは区別する。受信はACKまで非破壊。同じ送信元・UUIDの再送を未ACK期間中と、ACK後の直近65536件で重複除去する。

キューは既定4096件・最大64MiB。満杯の相手は再試行を返し、本体が送信待ちを保持する。1ノードが停止するとその相手を含むバッチは再試行になるため、ほかの相手にも再送が届く。全相手への配送を優先する方式で、停止ノードを無視して無制限に先へ進まない。

キューと重複情報はメモリにあり、受信側のクラッシュでは受信済み・未注入フレームを失い得る。送信側も強制終了では未送信フレームを失う。ディスク永続性やexactly-onceは保証しない。NIC注入直後のクラッシュや重複除去範囲を超えた再送では重複し得る。

同じユーザのローカルプロセスではchannel/node_idの多重起動を拒否する。別マシンへ同じ秘密鍵・node_idを複製して使わない。公開先はUDPポートで、証明書を信頼した相手だけがパケットを送信できる。

## ビルド・試験

Ubuntu/DebianでRust未導入の場合:

```bash
sudo apt-get install -y build-essential curl ca-certificates docker.io
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
. "$HOME/.cargo/env"
rustup component add rustfmt clippy
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked
```

Next.jsの試験はNode.js 24以降とDockerが必要。Node.jsがない場合は、[nvm](https://github.com/nvm-sh/nvm)を使って導入できる。

```bash
curl -fsSL https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.3/install.sh | bash
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm install 24
node --version
```

Dockerを実行できるユーザで次を実行する。

```bash
npm --prefix signaling ci
npm --prefix signaling run build
bash scripts/test-signaling.sh
```

試験用Redisを作成して終了時に削除する。認証・部屋分離・ID衝突・過大入力を検証し、実際のQUICで3ノードを接続する。接続情報交換サーバ停止後もフレーム本体が届くことを確認する。

本体の`scripts/vm-lab up --relay p2p`と`test --relay p2p`では、3台の実VMでICMP・TCP・UDPと停止後の再配送を確認する。DBを停止して試験し、使用中のプラグイン削除拒否と、本体を変更しない追加削除も検証する。
