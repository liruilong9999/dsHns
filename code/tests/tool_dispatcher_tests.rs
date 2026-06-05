//! 工具系统测试。
//!
//! 这些测试覆盖工具注册、参数校验、审批控制、工作区边界、熔断与调用总次数限制。

use dshns_agent::domain::tool::{
    SessionApprovalMode, ToolCallRequest, ToolExecutionStatus, ToolPermission,
};
use dshns_agent::infra::tool_system::{ToolDispatcher, ToolRuntimeConfig};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn 应注册默认工具及其元数据() {
    let workspace_root = create_temp_directory("tool-registry-workspace");
    let skill_root = create_skill_root("tool-registry-skills");
    let dispatcher =
        ToolDispatcher::new(ToolRuntimeConfig::new(workspace_root.clone(), skill_root));

    let metadata = dispatcher
        .registry()
        .metadata("run_shell")
        .expect("缺少 run_shell 元数据");
    assert_eq!(metadata.name, "run_shell");
    assert_eq!(metadata.default_permission, ToolPermission::WorkspaceOnly);
    assert!(metadata.visible);
    assert!(!metadata.background);

    assert!(dispatcher.registry().metadata("read_file").is_some());
    assert!(dispatcher.registry().metadata("write_file").is_some());
    assert!(dispatcher.registry().metadata("load_skill").is_some());
    assert!(dispatcher.registry().metadata("plan_tool").is_some());
    assert!(dispatcher.registry().metadata("create_agent").is_some());
}

#[test]
fn 应支持首批工具成功路径() {
    let workspace_root = create_temp_directory("tool-success-workspace");
    let skill_root = create_skill_root("tool-success-skills");
    let input_file = workspace_root.join("input.txt");
    fs::write(&input_file, "第一行\n第二行\n第三行\n").expect("写入读取测试文件失败");
    let replace_file = workspace_root.join("replace.txt");
    fs::write(&replace_file, "甲\n乙\n丙\n丁\n").expect("写入替换测试文件失败");
    create_skill(
        skill_root.as_path(),
        "demo_skill",
        "# Demo Skill\n\n这是一个测试技能。",
    );

    let mut dispatcher = ToolDispatcher::new(ToolRuntimeConfig::new(
        workspace_root.clone(),
        skill_root.clone(),
    ));

    let read_response = dispatcher.execute(
        ToolCallRequest::new(
            "read_file",
            "SES-0001",
            "AGT-0001",
            "ROUND-0001",
            json!({
                "path": input_file.to_string_lossy().to_string()
            }),
        ),
        SessionApprovalMode::AllowAll,
    );
    assert_success(&read_response, "read_file");
    assert_eq!(
        read_response.result_payload["content"].as_str(),
        Some("第一行\n第二行\n第三行\n")
    );

    let shell_response = dispatcher.execute(
        ToolCallRequest::new(
            "run_shell",
            "SES-0001",
            "AGT-0001",
            "ROUND-0002",
            json!({
                "command": "Write-Output '工具执行成功'"
            }),
        ),
        SessionApprovalMode::Auto,
    );
    assert_success(&shell_response, "run_shell");
    assert_eq!(shell_response.exit_code, Some(0));
    assert!(
        shell_response.result_payload["stdout"]
            .as_str()
            .expect("缺少 stdout")
            .contains("工具执行成功")
    );
    assert!(
        shell_response.result_payload["cwd"]
            .as_str()
            .expect("缺少 cwd")
            .contains(
                workspace_root
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .as_ref()
            )
    );

    let create_response = dispatcher.execute(
        ToolCallRequest::new(
            "write_file",
            "SES-0001",
            "AGT-0001",
            "ROUND-0003",
            json!({
                "path": workspace_root.join("created.txt").to_string_lossy().to_string(),
                "content": "新建文件内容",
                "mode": "create"
            }),
        ),
        SessionApprovalMode::Auto,
    );
    assert_success(&create_response, "write_file");
    assert_eq!(
        fs::read_to_string(workspace_root.join("created.txt")).expect("读取创建文件失败"),
        "新建文件内容"
    );

    let replace_response = dispatcher.execute(
        ToolCallRequest::new(
            "write_file",
            "SES-0001",
            "AGT-0001",
            "ROUND-0004",
            json!({
                "path": replace_file.to_string_lossy().to_string(),
                "content": "新的第二行\n新的第三行",
                "mode": "replace_range",
                "range": {
                    "start_line": 2,
                    "end_line": 3
                }
            }),
        ),
        SessionApprovalMode::Auto,
    );
    assert_success(&replace_response, "write_file");
    assert_eq!(
        fs::read_to_string(&replace_file).expect("读取替换文件失败"),
        "甲\n新的第二行\n新的第三行\n丁\n"
    );

    let load_skill_response = dispatcher.execute(
        ToolCallRequest::new(
            "load_skill",
            "SES-0001",
            "AGT-0001",
            "ROUND-0005",
            json!({
                "name": "demo_skill"
            }),
        ),
        SessionApprovalMode::Auto,
    );
    assert_success(&load_skill_response, "load_skill");
    assert!(
        load_skill_response.result_payload["content"]
            .as_str()
            .expect("缺少技能内容")
            .contains("这是一个测试技能")
    );

    let plan_response = dispatcher.execute(
        ToolCallRequest::new(
            "plan_tool",
            "SES-0001",
            "AGT-0001",
            "ROUND-0006",
            json!({
                "steps": ["收集信息", "执行修改", "验证结果"]
            }),
        ),
        SessionApprovalMode::AllowAll,
    );
    assert_success(&plan_response, "plan_tool");
    assert_eq!(
        plan_response.result_payload["steps"]
            .as_array()
            .map(|items| items.len()),
        Some(3)
    );
}

#[test]
fn 应拒绝未注册工具错误参数和越界访问() {
    let workspace_root = create_temp_directory("tool-boundary-workspace");
    let skill_root = create_skill_root("tool-boundary-skills");
    let outside_root = create_temp_directory("tool-outside-workspace");
    let mut dispatcher =
        ToolDispatcher::new(ToolRuntimeConfig::new(workspace_root.clone(), skill_root));

    let unknown_response = dispatcher.execute(
        ToolCallRequest::new(
            "unknown_tool",
            "SES-0001",
            "AGT-0001",
            "ROUND-0100",
            json!({}),
        ),
        SessionApprovalMode::Auto,
    );
    assert_eq!(unknown_response.status, ToolExecutionStatus::Failed);
    assert_eq!(
        unknown_response.error_code.as_deref(),
        Some("TOOL_NOT_REGISTERED")
    );

    let invalid_response = dispatcher.execute(
        ToolCallRequest::new(
            "run_shell",
            "SES-0001",
            "AGT-0001",
            "ROUND-0101",
            json!({
                "command": 123
            }),
        ),
        SessionApprovalMode::Auto,
    );
    assert_eq!(invalid_response.status, ToolExecutionStatus::Failed);
    assert_eq!(
        invalid_response.error_code.as_deref(),
        Some("INVALID_ARGUMENT")
    );

    let boundary_response = dispatcher.execute(
        ToolCallRequest::new(
            "run_shell",
            "SES-0001",
            "AGT-0001",
            "ROUND-0102",
            json!({
                "command": "Get-ChildItem",
                "cwd": outside_root.to_string_lossy().to_string()
            }),
        ),
        SessionApprovalMode::AllowAll,
    );
    assert_eq!(boundary_response.status, ToolExecutionStatus::Blocked);
    assert_eq!(
        boundary_response.error_code.as_deref(),
        Some("WORKSPACE_BOUNDARY_VIOLATION")
    );

    let deny_in_auto = dispatcher.execute(
        ToolCallRequest::new(
            "plan_tool",
            "SES-0001",
            "AGT-0001",
            "ROUND-0103",
            json!({
                "steps": ["第一步"]
            }),
        ),
        SessionApprovalMode::Auto,
    );
    assert_eq!(deny_in_auto.status, ToolExecutionStatus::Blocked);
    assert_eq!(deny_in_auto.error_code.as_deref(), Some("TOOL_AUTO_DENIED"));
}

#[test]
fn 应执行审批矩阵熔断与总次数限制() {
    let workspace_root = create_temp_directory("tool-policy-workspace");
    let skill_root = create_skill_root("tool-policy-skills");
    let mut dispatcher = ToolDispatcher::new(ToolRuntimeConfig::new(workspace_root, skill_root));

    let ask_deny = dispatcher.execute(
        ToolCallRequest::new(
            "plan_tool",
            "SES-0001",
            "AGT-0001",
            "ROUND-0200",
            json!({
                "steps": ["人工确认"]
            }),
        ),
        SessionApprovalMode::Ask,
    );
    assert_eq!(ask_deny.status, ToolExecutionStatus::Blocked);
    assert_eq!(ask_deny.error_code.as_deref(), Some("APPROVAL_REQUIRED"));

    let allow_all_deny = dispatcher.execute(
        ToolCallRequest::new(
            "plan_tool",
            "SES-0001",
            "AGT-0001",
            "ROUND-0201",
            json!({
                "steps": ["允许执行"]
            }),
        ),
        SessionApprovalMode::AllowAll,
    );
    assert_success(&allow_all_deny, "plan_tool");

    for _ in 0..5 {
        let failed = dispatcher.execute(
            ToolCallRequest::new(
                "run_shell",
                "SES-0001",
                "AGT-0001",
                "ROUND-0202",
                json!({
                    "command": 100
                }),
            ),
            SessionApprovalMode::Auto,
        );
        assert_eq!(failed.status, ToolExecutionStatus::Failed);
    }

    let circuit_open = dispatcher.execute(
        ToolCallRequest::new(
            "run_shell",
            "SES-0001",
            "AGT-0001",
            "ROUND-0202",
            json!({
                "command": "Write-Output '不会执行'"
            }),
        ),
        SessionApprovalMode::Auto,
    );
    assert_eq!(circuit_open.status, ToolExecutionStatus::Blocked);
    assert_eq!(
        circuit_open.error_code.as_deref(),
        Some("TOOL_CIRCUIT_OPEN")
    );

    for index in 0..12 {
        let path =
            create_text_file_in_workspace(index, &dispatcher.runtime_config().workspace_root_path);
        let response = dispatcher.execute(
            ToolCallRequest::new(
                "read_file",
                "SES-0001",
                "AGT-0001",
                "ROUND-0203",
                json!({
                    "path": path.to_string_lossy().to_string()
                }),
            ),
            SessionApprovalMode::AllowAll,
        );
        assert_eq!(response.status, ToolExecutionStatus::Success);
    }

    let exceeded = dispatcher.execute(
        ToolCallRequest::new(
            "plan_tool",
            "SES-0001",
            "AGT-0001",
            "ROUND-0203",
            json!({
                "steps": ["第十三次"]
            }),
        ),
        SessionApprovalMode::AllowAll,
    );
    assert_eq!(exceeded.status, ToolExecutionStatus::Blocked);
    assert_eq!(
        exceeded.error_code.as_deref(),
        Some("TOOL_CALL_LIMIT_EXCEEDED")
    );
}

/// 断言工具执行成功。
fn assert_success(response: &dshns_agent::domain::tool::ToolResponse, expected_tool_name: &str) {
    assert_eq!(response.status, ToolExecutionStatus::Success);
    assert_eq!(response.tool_name, expected_tool_name);
    assert!(response.ok);
}

/// 创建临时目录。
fn create_temp_directory(prefix: &str) -> PathBuf {
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间早于纪元")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("dshns-{prefix}-{unique_suffix}"));
    fs::create_dir_all(&path).expect("创建临时目录失败");
    path
}

/// 创建技能根目录。
fn create_skill_root(prefix: &str) -> PathBuf {
    create_temp_directory(prefix)
}

/// 在技能根目录中创建测试技能。
fn create_skill(skill_root: &std::path::Path, skill_name: &str, content: &str) {
    let skill_directory = skill_root.join(skill_name);
    fs::create_dir_all(&skill_directory).expect("创建技能目录失败");
    fs::write(skill_directory.join("SKILL.md"), content).expect("写入技能文件失败");
}

/// 为总次数限制测试创建文本文件。
fn create_text_file_in_workspace(index: usize, workspace: &std::path::Path) -> PathBuf {
    let path = workspace.join(format!("file-{index}.txt"));
    fs::write(&path, format!("内容-{index}")).expect("写入轮次测试文件失败");
    path
}
