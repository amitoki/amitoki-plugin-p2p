use stegrdb_plugin_sdk::{PluginManifest, PROTOCOL_VERSION};
pub fn manifest() -> PluginManifest {
    PluginManifest {
        name: "p2p".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        protocol_version: PROTOCOL_VERSION,
        description: "相互認証したQUIC接続でクライアント間を直接中継".into(),
        config_schema: serde_json::json!({
            "type":"object", "additionalProperties":false,
            "required":["listen","certificate","private_key","peers"],
            "properties":{
                "listen":{"type":"string","description":"QUIC待受アドレス（例: 0.0.0.0:7443）"},
                "certificate":{"type":"string","minLength":1,"description":"自ノードのDER証明書のパス"},
                "private_key":{"type":"string","minLength":1,"description":"自ノードのPKCS#8 DER秘密鍵のパス"},
                "queue_capacity":{"type":"integer","minimum":1,"maximum":65536,"default":4096,"description":"受信待ちフレームの最大件数"},
                "discovery":{"type":"object","additionalProperties":false,"required":["url","token_env","advertise"],"description":"任意のHTTPS接続情報交換サーバ","properties":{
                    "url":{"type":"string","minLength":1},"token_env":{"type":"string","minLength":1},"advertise":{"type":"string","minLength":1}
                }},
                "peers":{"type":"array","minItems":1,"maxItems":32,"description":"接続先の一覧。JSON配列で指定","items":{
                    "type":"object","additionalProperties":false,"required":["node_id","certificate"],
                    "properties":{"node_id":{"type":"string","minLength":1},"address":{"type":"string"},"certificate":{"type":"string","minLength":1}}
                }}
            }
        }),
    }
}
