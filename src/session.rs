use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::{
    fs::{DirBuilder, File, OpenOptions},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
};
use stegrdb_relay::{RelayContext, RelayError};

pub fn claim_node(context: &RelayContext) -> Result<File, RelayError> {
    let user = unsafe { libc::geteuid() };
    let directory = std::env::temp_dir().join(format!("stegrdb-p2p-{user}"));
    if let Err(error) = DirBuilder::new().mode(0o700).create(&directory) {
        if error.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(RelayError::permanent("ノード占有ディレクトリを作成できません"));
        }
    }
    let metadata = directory.symlink_metadata().map_err(|_| RelayError::permanent("ノード占有ディレクトリを読めません"))?;
    if !metadata.is_dir() || metadata.uid() != user || metadata.mode() & 0o777 != 0o700 {
        return Err(RelayError::permanent("ノード占有ディレクトリの所有者または権限が不正です"));
    }
    let identity = serde_json::to_vec(&[&context.channel, &context.node_id]).expect("string serialization");
    let path = directory.join(format!("{:x}.lock", Sha256::digest(identity)));
    // ロックファイルを削除すると、別inode上で同じIDを多重起動できるため残す。
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| RelayError::permanent("ノード占有ファイルを開けません"))?;
    file.try_lock_exclusive().map_err(|_| RelayError::permanent("このchannel/node_idは同じユーザの別プロセスが使用中です"))?;
    Ok(file)
}
