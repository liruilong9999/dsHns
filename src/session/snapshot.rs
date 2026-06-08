//! 会话快照读写逻辑。
use std::fs;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use configparser::ini::Ini;

use crate::domain::{ApprovalMode, Message, Session, SessionStatus};
use crate::utils::fs::{ensure_directory, read_optional_utf8, write_utf8};
use crate::utils::hash::sha256_hex;
use crate::utils::time::now_rfc3339;

/// 会话快照文件名。
pub const SESSION_INI_NAME: &str = "session.ini";
/// 消息快照文件名。
pub const MEMORY_JSON_NAME: &str = "memory.json";
/// 消息快照备份文件名。
pub const MEMORY_JSON_BACKUP_NAME: &str = "memory.json.bak";
/// 工具结果索引文件名。
pub const TOOL_RESULT_INDEX_NAME: &str = "tool_results/index.json";
/// 工作记忆文件名。
pub const WORKING_MEMORY_NAME: &str = "working_memory.json";

/// 写入 `session.ini` 快照。
pub fn write_session_ini(session: &Session) -> Result<()> {
    ensure_directory(&session.session_dir)?;
    let mut ini = Ini::new();
    ini.set("session", "id", Some(session.id.clone()));
    ini.set(
        "session",
        "directory_id",
        Some(session.directory_id.clone()),
    );
    ini.set("session", "name", Some(session.name.clone()));
    ini.set(
        "session",
        "project_name",
        Some(session.project_name.clone()),
    );
    ini.set(
        "session",
        "project_path",
        Some(session.project_path.clone()),
    );
    ini.set(
        "session",
        "working_directory",
        Some(session.working_directory.clone()),
    );
    ini.set("session", "model", Some(session.model.clone()));
    ini.set(
        "session",
        "approval_mode",
        Some(session.approval_mode.as_str().to_string()),
    );
    ini.set(
        "session",
        "status",
        Some(session.status.as_str().to_string()),
    );
    ini.set(
        "session",
        "system_prompt",
        Some(session.system_prompt.clone()),
    );
    ini.set(
        "session",
        "stream_output",
        Some(session.stream_output.to_string()),
    );
    ini.set("state", "round", Some(session.round.to_string()));
    ini.set(
        "state",
        "is_finished",
        Some(session.is_finished.to_string()),
    );
    ini.set(
        "snapshot",
        "version",
        Some(session.snapshot_version.to_string()),
    );
    ini.set(
        "snapshot",
        "last_round_no",
        Some(session.last_round_no.to_string()),
    );
    ini.set(
        "snapshot",
        "content_hash",
        Some(session.content_hash.clone()),
    );
    ini.set("meta", "created_at", Some(session.created_at.clone()));
    ini.set("meta", "updated_at", Some(session.updated_at.clone()));
    ini.write(
        session
            .session_dir
            .join(SESSION_INI_NAME)
            .to_string_lossy()
            .as_ref(),
    )
    .with_context(|| {
        format!(
            "写入 session.ini 失败：{}",
            session.session_dir.join(SESSION_INI_NAME).display()
        )
    })?;
    Ok(())
}

/// 读取 `session.ini` 快照。
pub fn read_session_ini(session_dir: &Path) -> Result<Session> {
    let path = session_dir.join(SESSION_INI_NAME);
    let mut ini = Ini::new();
    ini.load(path.to_string_lossy().as_ref())
        .map_err(|error| anyhow!("读取 session.ini 失败：{}，原因：{}", path.display(), error))?;

    let project_path = ini.get("session", "project_path").unwrap_or_default();
    let now = now_rfc3339();
    let session = Session {
        id: ini
            .get("session", "id")
            .or_else(|| {
                session_dir
                    .file_name()
                    .and_then(|value| value.to_str())
                    .map(|value| value.to_string())
            })
            .unwrap_or_default(),
        directory_id: ini
            .get("session", "directory_id")
            .unwrap_or_else(|| sha256_hex(&project_path)),
        name: ini
            .get("session", "name")
            .unwrap_or_else(|| "restored-session".to_string()),
        project_name: ini
            .get("session", "project_name")
            .unwrap_or_else(|| "workspace".to_string()),
        project_path: project_path.clone(),
        working_directory: ini
            .get("session", "working_directory")
            .unwrap_or_else(|| project_path.clone()),
        model: ini
            .get("session", "model")
            .unwrap_or_else(|| "deepseek-v4-flash".to_string()),
        approval_mode: ApprovalMode::from_str(
            &ini.get("session", "approval_mode")
                .unwrap_or_else(|| "AskUser".to_string()),
        ),
        status: SessionStatus::from_str(
            &ini.get("session", "status")
                .unwrap_or_else(|| "selected".to_string()),
        ),
        stream_output: ini
            .get("session", "stream_output")
            .and_then(|value| value.parse::<bool>().ok())
            .unwrap_or(true),
        round: ini
            .get("state", "round")
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or_default(),
        is_finished: ini
            .get("state", "is_finished")
            .and_then(|value| value.parse::<bool>().ok())
            .unwrap_or(false),
        session_dir: session_dir.to_path_buf(),
        snapshot_version: ini
            .get("snapshot", "version")
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(1),
        last_round_no: ini
            .get("snapshot", "last_round_no")
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or_default(),
        content_hash: ini.get("snapshot", "content_hash").unwrap_or_default(),
        system_prompt: ini.get("session", "system_prompt").unwrap_or_default(),
        created_at: ini.get("meta", "created_at").unwrap_or_else(|| now.clone()),
        updated_at: ini.get("meta", "updated_at").unwrap_or(now),
    };
    Ok(session)
}

/// 写入 `memory.json` 快照。
pub fn write_memory_json(session_dir: &Path, messages: &[Message]) -> Result<()> {
    let target = session_dir.join(MEMORY_JSON_NAME);
    if target.exists() {
        let backup = session_dir.join(MEMORY_JSON_BACKUP_NAME);
        fs::copy(&target, &backup).with_context(|| {
            format!(
                "备份 memory.json 失败：{} -> {}",
                target.display(),
                backup.display()
            )
        })?;
    }

    let json = serde_json::to_string_pretty(messages)?;
    write_utf8(&target, &json)
}

/// 读取 `memory.json` 快照。
pub fn read_memory_json(session_dir: &Path) -> Result<Vec<Message>> {
    let path = session_dir.join(MEMORY_JSON_NAME);
    let content = read_optional_utf8(&path)?.unwrap_or_else(|| "[]".to_string());
    let messages = serde_json::from_str(&content)
        .with_context(|| format!("解析 memory.json 失败：{}", path.display()))?;
    Ok(messages)
}

/// 初始化工具结果索引文件。
pub fn ensure_tool_result_index(session_dir: &Path) -> Result<()> {
    let path = session_dir.join(TOOL_RESULT_INDEX_NAME);
    if !path.exists() {
        write_utf8(&path, "[]\n")?;
    }
    Ok(())
}

/// 写入工作记忆文件。
pub fn write_working_memory(session_dir: &Path, content: &str) -> Result<()> {
    write_utf8(&session_dir.join(WORKING_MEMORY_NAME), content)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        read_memory_json, read_session_ini, write_memory_json, write_session_ini,
        MEMORY_JSON_BACKUP_NAME,
    };
    use crate::domain::{ApprovalMode, Message, Session};
    use crate::utils::fs::read_optional_utf8;

    #[test]
    fn should_write_and_read_session_ini() {
        let session_dir = PathBuf::from("target/test_session_ini");
        let session = Session::new(
            "session-1".to_string(),
            "directory-1".to_string(),
            "demo".to_string(),
            "demo-project".to_string(),
            "D:/demo".to_string(),
            "D:/demo".to_string(),
            "deepseek-v4-flash".to_string(),
            ApprovalMode::AskUser,
            true,
            session_dir.clone(),
            "system prompt".to_string(),
        );
        write_session_ini(&session).expect("写入 session.ini 失败");
        let loaded = read_session_ini(&session_dir).expect("读取 session.ini 失败");
        assert_eq!(loaded.id, session.id);
        assert_eq!(loaded.name, session.name);
    }

    #[test]
    fn should_backup_memory_json_before_overwrite() {
        let session_dir = PathBuf::from("target/test_memory_backup");
        write_memory_json(&session_dir, &[Message::user("first")]).expect("write first memory");
        write_memory_json(&session_dir, &[Message::user("second")]).expect("write second memory");

        let backup = read_optional_utf8(&session_dir.join(MEMORY_JSON_BACKUP_NAME))
            .expect("read memory backup")
            .expect("missing memory backup");
        let current = read_memory_json(&session_dir).expect("read current memory");

        assert!(backup.contains("first"));
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].content, "second");
    }
}
