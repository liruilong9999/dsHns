//! 文件系统辅助函数。

use std::fs;
use std::path::Path;

use anyhow::{anyhow, Context, Result};

/// 确保目录存在，不存在时自动创建。
pub fn ensure_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("创建目录失败：{}", path.display()))
}

/// 读取可选的 UTF-8 文本文件。
pub fn read_optional_utf8(path: &Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }

    let content =
        fs::read_to_string(path).with_context(|| format!("读取文件失败：{}", path.display()))?;
    Ok(Some(content))
}

/// 以 UTF-8 方式写入文本文件，并自动创建父目录。
pub fn write_utf8(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        ensure_directory(parent)?;
    }

    fs::write(path, content).with_context(|| format!("写入文件失败：{}", path.display()))
}

/// 按 1 基行号替换文件中的指定行范围。
pub fn replace_file_range(
    path: &Path,
    start_line: usize,
    end_line: usize,
    new_content: &str,
) -> Result<()> {
    if !path.exists() {
        return Err(anyhow!("目标文件不存在：{}", path.display()));
    }

    if start_line == 0 || end_line == 0 || end_line < start_line {
        return Err(anyhow!("replace_range 行号非法，要求 start_line 与 end_line 均为正整数，且 end_line 不小于 start_line"));
    }

    let original = fs::read_to_string(path)
        .with_context(|| format!("读取待替换文件失败：{}", path.display()))?;
    let mut lines: Vec<String> = original.lines().map(ToOwned::to_owned).collect();

    if start_line > lines.len() || end_line > lines.len() {
        return Err(anyhow!(
            "replace_range 超出文件范围：文件共 {} 行，收到 {}-{}",
            lines.len(),
            start_line,
            end_line
        ));
    }

    let replacement_lines: Vec<String> = if new_content.is_empty() {
        Vec::new()
    } else {
        new_content.lines().map(ToOwned::to_owned).collect()
    };

    lines.splice(start_line - 1..end_line, replacement_lines);

    let rewritten = if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    };

    write_utf8(path, &rewritten)
}

#[cfg(test)]
mod tests {
    //! 文件替换能力的单元测试。

    use std::fs;
    use std::path::PathBuf;

    use super::{replace_file_range, write_utf8};

    /// 验证 replace_range 只替换指定行范围。
    #[test]
    fn should_replace_specific_range() {
        let path = PathBuf::from("target/test_replace_range.txt");
        write_utf8(&path, "a\nb\nc\nd\n").expect("初始化测试文件失败");

        replace_file_range(&path, 2, 3, "x\ny").expect("替换文件范围失败");
        let actual = fs::read_to_string(&path).expect("读取替换结果失败");

        assert_eq!(actual, "a\nx\ny\nd\n");
    }
}
