//! 工具相关实体定义。

use serde::{Deserialize, Serialize};

/// 工具风险等级。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolRiskLevel {
    /// 只读风险。
    ReadOnly,
    /// 写文件风险。
    Write,
    /// 执行命令风险。
    Execute,
    /// 网络访问风险。
    Network,
    /// 子 Agent 风险。
    Agent,
}

/// 工具失败类型。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolFailureType {
    /// 非法参数。
    InvalidArgs,
    /// 审批拒绝。
    ApprovalDenied,
    /// 执行异常。
    ExecError,
}

/// 工具结果投影类型。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolProjectionType {
    /// 直接内联全文。
    InlineFull,
    /// 仅返回摘要。
    Summary,
}

/// `write_file` 的按行替换参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplaceRange {
    /// 起始行号，1 基。
    pub start_line: usize,
    /// 结束行号，1 基。
    pub end_line: usize,
    /// 新内容。
    pub new_content: String,
}

/// 工具调用记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    /// 工具调用标识。
    pub id: String,
    /// 会话标识。
    pub session_id: String,
    /// 轮次。
    pub round_no: i64,
    /// 工具名称。
    pub tool_name: String,
    /// 参数 JSON。
    pub arguments_json: String,
    /// 状态。
    pub status: String,
    /// 是否成功。
    pub success: bool,
    /// 失败类型。
    pub failure_type: Option<ToolFailureType>,
    /// 错误信息。
    pub error_message: String,
    /// 创建时间。
    pub created_at: String,
    /// 更新时间。
    pub updated_at: String,
}

/// 工具结果索引记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultRecord {
    /// 工具调用标识。
    pub tool_call_id: String,
    /// 工具名称。
    pub tool_name: String,
    /// 结果句柄。
    pub handle: String,
    /// 正文文件路径。
    pub body_file_path: String,
    /// 投影类型。
    pub projection_type: ToolProjectionType,
    /// 投影正文。
    pub projection_content: String,
    /// 摘要信息。
    pub summary: String,
    /// 前部预览。
    #[serde(default)]
    pub preview_head: String,
    /// 尾部预览。
    #[serde(default)]
    pub preview_tail: String,
    /// 字符数。
    pub char_count: usize,
    /// 字节数。
    #[serde(default)]
    pub byte_count: usize,
    /// 是否成功。
    pub success: bool,
    /// 是否被截断。
    pub truncated: bool,
    /// 是否外置化。
    pub externalized: bool,
    /// 更新时间。
    pub updated_at: String,
}
