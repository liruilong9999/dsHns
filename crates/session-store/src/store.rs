use std::path::PathBuf;
use dshns_core::message::Message;
use dshns_core::session::{Session, SessionId, SessionMeta};
use dshns_core::error::DshnsError;

pub struct SessionStore { root: PathBuf }

impl SessionStore {
    pub fn new() -> Result<Self, DshnsError> {
        let home = home_dir()?;
        let root = home.join(".dsHns_rs/sessions");
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    fn session_dir(&self, id: &SessionId) -> PathBuf { self.root.join(id.to_dir_name()) }

    pub fn create(&self, working_dir: &std::path::Path) -> Result<SessionId, DshnsError> {
        let meta = SessionMeta::new(working_dir.to_path_buf());
        let dir = self.session_dir(&meta.id);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("meta.json"), serde_json::to_string_pretty(&meta).unwrap())?;
        std::fs::write(dir.join("messages.jsonl"), "")?;
        Ok(meta.id)
    }

    pub fn append_message(&self, id: &SessionId, msg: &Message) -> Result<(), DshnsError> {
        let path = self.session_dir(id).join("messages.jsonl");
        let mut line = serde_json::to_string(msg).unwrap();
        line.push('\n');
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        f.write_all(line.as_bytes())?;
        Ok(())
    }

    pub fn append_messages(&self, id: &SessionId, msgs: &[Message]) -> Result<(), DshnsError> {
        for m in msgs { self.append_message(id, m)?; }
        Ok(())
    }

    pub fn load(&self, id: &SessionId) -> Result<Session, DshnsError> {
        let dir = self.session_dir(id);
        if !dir.exists() { return Err(DshnsError::SessionNotFound(id.to_string())); }
        let meta: SessionMeta =
            serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json"))?)
                .map_err(|e| DshnsError::Other(format!("解析 meta.json 失败: {e}")))?;
        let content = std::fs::read_to_string(dir.join("messages.jsonl"))?;
        let messages: Vec<Message> = content.lines().filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str(l).ok()).collect();
        Ok(Session { meta, messages })
    }

    pub fn list(&self) -> Result<Vec<SessionMeta>, DshnsError> {
        let mut sessions = Vec::new();
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let mp = entry.path().join("meta.json");
                if mp.exists() {
                    if let Ok(meta) = serde_json::from_str::<SessionMeta>(&std::fs::read_to_string(&mp)?) {
                        sessions.push(meta);
                    }
                }
            }
        }
        sessions.sort_by_key(|s| s.updated_at);
        sessions.reverse();
        Ok(sessions)
    }
}

fn home_dir() -> Result<PathBuf, DshnsError> {
    std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from).map_err(|_| DshnsError::Config("无法获取 HOME".into()))
}
