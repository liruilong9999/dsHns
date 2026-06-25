use dshns_core::error::DshnsError;

pub struct MemoryStore { root: std::path::PathBuf }

impl MemoryStore {
    pub fn new() -> Result<Self, DshnsError> {
        let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME"))
            .map(std::path::PathBuf::from)
            .map_err(|_| DshnsError::Config("无法获取 HOME".into()))?;
        let root = home.join(".dsHns_rs/memory");
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    pub fn list(&self) -> Result<Vec<String>, DshnsError> {
        Ok(std::fs::read_dir(&self.root)?.filter_map(|e| {
            e.ok().and_then(|e| e.file_name().to_str().map(|s| s.to_string()))
        }).filter(|n| n.ends_with(".md")).collect())
    }
}
