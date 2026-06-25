#[cfg(test)]
mod integration {
    #[test]
    #[ignore]
    fn test_simple_chat() {
        // DEEPSEEK_API_KEY=sk-xxx cargo test --test integration_test -- --ignored
        todo!("设置 API Key 后运行实际对话测试")
    }

    #[test]
    #[ignore]
    fn test_tool_call_roundtrip() {
        // 测试: "读取 Cargo.toml" → read_file → 返回内容 → 模型总结
        todo!("设置 API Key 后运行工具调用测试")
    }

    #[test]
    #[ignore]
    fn test_safety_block_rm_rf() {
        // 验证 exec_shell 的硬限制拦截
    }
}
