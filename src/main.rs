use amitoki_plugin_p2p::{identity::create_identity, manifest::manifest, P2pPlugin};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    match arguments.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["--describe"] => println!("{}", serde_json::to_string_pretty(&manifest())?),
        ["--stdio"] => amitoki_plugin_sdk::serve(P2pPlugin, manifest()).await?,
        ["identity", "--output", directory] => {
            create_identity(std::path::Path::new(directory))?;
            println!("証明書と秘密鍵を作成しました: {directory}");
        },
        _ => return Err("--stdio / --describe / identity --output 新規ディレクトリを指定してください".into()),
    }
    Ok(())
}
