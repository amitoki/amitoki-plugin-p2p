use std::{fs::OpenOptions, io::Write, os::unix::fs::OpenOptionsExt, path::Path};

pub fn create_identity(directory: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // 既存IDの上書きは対向の信頼設定を壊すため、ディレクトリごと新規作成に限る。
    std::fs::create_dir(directory)?;
    let identity = rcgen::generate_simple_self_signed(vec!["stegrdb.invalid".into()])?;
    let mut key = OpenOptions::new().write(true).create_new(true).mode(0o600).open(directory.join("key.der"))?;
    key.write_all(&identity.key_pair.serialize_der())?;
    std::fs::write(directory.join("cert.der"), identity.cert.der())?;
    Ok(())
}
