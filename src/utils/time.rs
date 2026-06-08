//! 时间辅助函数。

use chrono::{SecondsFormat, Utc};

/// 生成统一的 RFC3339 UTC 时间戳。
pub fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}
