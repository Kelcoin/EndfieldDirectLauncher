//! README 与代码的防锈契约：文档骨架、参数集合、引用文件、表格与行尾。

use crate::help_text;

const README: &str = include_str!("../README.md");
const README_BYTES: &[u8] = include_bytes!("../README.md");

/// 文档骨架：二级标题必须正好是这些，且顺序不变。
const SECTIONS: [&str; 12] = [
    "目录",
    "功能",
    "快速开始",
    "命令行参数",
    "与自动化脚本一起用（MaaEnd）",
    "日志与排错",
    "常见问题",
    "工作原理",
    "已知限制与未验证项",
    "验证记录",
    "开发",
    "许可",
];

fn sections() -> Vec<&'static str> {
    README
        .lines()
        .filter_map(|line| line.strip_prefix("## "))
        .map(str::trim)
        .collect()
}

/// `help_text()` 里出现的长选项（`--game-dir=<dir>` 归一成 `--game-dir`）。
fn help_options() -> Vec<String> {
    let mut out: Vec<String> = help_text()
        .split_whitespace()
        .filter(|token| token.starts_with("--"))
        .map(|token| {
            token
                .split('=')
                .next()
                .unwrap_or("")
                .trim_end_matches([',', ';', ')', ':'])
                .to_string()
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// 参数表首列里的长选项。
fn table_options() -> Vec<String> {
    let mut out: Vec<String> = README
        .lines()
        .filter(|line| line.starts_with("| `--"))
        .filter_map(|line| line.split('`').nth(1))
        .filter_map(|cell| cell.split_whitespace().next())
        .filter(|name| name.starts_with("--"))
        .map(str::to_string)
        .collect();
    out.sort();
    out.dedup();
    out
}

/// README 里出现的本地文件引用（`assets/…` 与 `](LICENSE)`）。
fn referenced_paths() -> Vec<String> {
    let mut paths = Vec::new();
    for line in README.lines() {
        let mut index = 0;
        while let Some(found) = line[index..].find("assets/") {
            let start = index + found;
            let tail = &line[start..];
            let end = tail
                .find(|character: char| {
                    !(character.is_ascii_alphanumeric()
                        || character == '.'
                        || character == '_'
                        || character == '-'
                        || character == '/')
                })
                .unwrap_or(tail.len());
            paths.push(tail[..end].to_string());
            index = start + end;
        }
    }
    if README.contains("](LICENSE)") {
        paths.push("LICENSE".to_string());
    }
    paths
}

#[test]
fn the_sections_are_the_standard_set() {
    assert_eq!(
        sections(),
        SECTIONS.to_vec(),
        "README 的二级标题被改动或换序了",
    );
}

#[test]
fn every_option_in_help_is_documented_and_only_those() {
    assert_eq!(
        table_options(),
        help_options(),
        "参数表与 help_text() 的选项集合不一致",
    );
}

#[test]
fn every_option_has_a_detail_section() {
    for option in table_options() {
        let needle = format!("**`{option}");
        assert!(
            README.contains(&needle),
            "{option} 在参数表里，但没有对应的详细说明小节",
        );
    }
}

#[test]
fn referenced_files_exist() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let paths = referenced_paths();
    assert!(
        paths.iter().any(|path| path == "assets/icon.png"),
        "README 少了图标引用",
    );
    for path in &paths {
        assert!(root.join(path).exists(), "README 引用了不存在的文件：{path}");
    }
}

#[test]
fn no_external_images() {
    for line in README.lines() {
        let lower = line.to_ascii_lowercase();
        assert!(
            !(line.contains("![") && lower.contains("](http")),
            "README 不许用外链图片：{line}",
        );
        assert!(
            !(lower.contains("<img") && lower.contains("src=\"http")),
            "README 不许用外链图片：{line}",
        );
    }
}

#[test]
fn tables_keep_their_column_count() {
    let mut block: Vec<&'static str> = Vec::new();
    let mut blocks: Vec<Vec<&'static str>> = Vec::new();
    for line in README.lines() {
        if line.starts_with('|') {
            block.push(line);
        } else if !block.is_empty() {
            blocks.push(std::mem::take(&mut block));
        }
    }
    if !block.is_empty() {
        blocks.push(block);
    }
    assert!(blocks.len() >= 3, "表格数量少于预期：{}", blocks.len());
    for rows in blocks {
        let pipes: Vec<usize> = rows.iter().map(|row| row.matches('|').count()).collect();
        assert!(
            pipes.windows(2).all(|pair| pair[0] == pair[1]),
            "表格列数不一致：{}",
            rows[0],
        );
    }
}

#[test]
fn code_fences_are_balanced_and_labelled() {
    let fences: Vec<&'static str> = README
        .lines()
        .filter(|line| line.starts_with("```"))
        .collect();
    assert_eq!(fences.len() % 2, 0, "代码围栏没有成对：{}", fences.len());
    for fence in &fences {
        let label = fence.trim_start_matches('`');
        assert!(
            label.is_empty() || label.chars().all(|character| character.is_ascii_alphanumeric()),
            "围栏标签异常：{fence}",
        );
    }
}

#[test]
fn the_file_is_plain_utf8_lf_without_trailing_blanks() {
    assert!(
        !README_BYTES.windows(2).any(|pair| pair == b"\r\n"),
        "README 必须只有 LF 行尾",
    );
    assert!(README_BYTES.ends_with(b"\n"), "README 末尾缺换行");
    for (index, line) in README.lines().enumerate() {
        assert_eq!(line.trim_end(), line, "第 {} 行有行尾空白", index + 1);
    }
}

#[test]
fn no_stale_version_numbers() {
    for stale in ["0.1.0", "0.1.1", "0.1.2"] {
        assert!(!README.contains(stale), "README 残留旧版本号 {stale}");
    }
    assert!(
        README.contains("<版本>"),
        "README 应使用 <版本> 占位，而不是写死一个版本号",
    );
}