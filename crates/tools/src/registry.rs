use std::collections::HashMap;
use std::sync::Arc;
use dshns_core::tool::{Tool, ToolDef};

pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self { Self { tools: HashMap::new() } }
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.definition().function.name.clone();
        self.tools.insert(name, tool);
    }
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> { self.tools.get(name).cloned() }
    pub fn get_names(&self) -> Vec<String> { self.tools.keys().cloned().collect() }
    pub fn to_api_tools(&self) -> Vec<ToolDef> { self.tools.values().map(|t| t.definition()).collect() }
    pub fn to_api_tools_excluding(&self, exclude: &[&str]) -> Vec<ToolDef> {
        self.tools.values()
            .filter(|t| !exclude.contains(&t.definition().function.name.as_str()))
            .map(|t| t.definition()).collect()
    }
}
