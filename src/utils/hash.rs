//! 哈希辅助函数。

use sha2::{Digest, Sha256};

/// 计算文本内容的 SHA-256 十六进制摘要。
pub fn sha256_hex(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}
