//! F10 验证探针（2026-10-18）。
//!
//! 验证 Mac 能否直连 TDX 7709 拿 F10 公司资料，以及分类名是否带 NUL 填充。
//!
//! 用法:
//!   cargo run --release --bin f10_probe -- 600396
//!
//! 只做取数和打印，不落库、不接 AI。验证通过后再决定要不要接进主链路。

use magic_tdx_rs::net::f10_client::TdxF10Client;

/// 用户实测（2026-10-18）：600396 的分类名返回带 NUL 填充，
/// 导致 `get_content_by_name("关联个股")` 精确匹配失败。
/// 这里统一 trim 一遍，既打印干净，也便于判断是不是所有票都这样。
fn clean(name: &str) -> &str {
    name.trim_end_matches('\0').trim_end_matches('\u{0}').trim()
}

fn main() {
    let code = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "600396".to_string());
    let market: u8 = if code.starts_with('6') { 1 } else { 0 };

    println!("[F10] code={code} market={market} server=117.34.114.14:7709");

    let client = TdxF10Client::new("117.34.114.14", 7709, Some(15.0));

    // 1. 列分类（保留 categories 供后面按前缀匹配）
    let categories = match client.get_category(market, &code) {
        Ok(categories) => {
            println!("[F10] 分类数={}", categories.len());
            let padded = categories
                .iter()
                .filter(|category| category.name.len() > 12)
                .count();
            for category in &categories {
                let cleaned: String = category
                    .name
                    .chars()
                    .take_while(|ch| !ch.is_control())
                    .collect();
                let mark = if category.name.len() > 12 {
                    " [填充]"
                } else {
                    ""
                };
                println!("  - {cleaned}  ({}B){mark}", category.name.len());
            }
            println!("[F10] 带填充的分类数={padded}");
            categories
        }
        Err(error) => {
            eprintln!("[F10] 列分类失败: {error}");
            std::process::exit(1);
        }
    };

    // 2. 取需要的分类：按前缀匹配 + get_content（内部走 filename_raw，不受填充影响）
    for name in ["经营分析", "行业分析", "最新提示", "股本结构"] {
        let Some(category) = categories
            .iter()
            .find(|category| category.name.starts_with(name))
        else {
            println!("\n[F10] {name} —— 分类列表里找不到");
            continue;
        };
        match client.get_content(market, &code, category) {
            Ok(content) => {
                println!("\n[F10] === {name} === ({} 字节)", content.byte_length);
                let preview: String = content.content.chars().take(1200).collect();
                println!("{preview}");
            }
            Err(error) => println!("\n[F10] {name} 失败: {error}"),
        }
    }
}
