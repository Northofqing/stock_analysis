//! 用户可见输出合同（WP5，2026-10-27）。
//!
//! 默认保持 legacy 文案；只有 `TICKET_OUTPUT_CONTRACT=1` 才启用
//! “参考价/参考区间/非下单指令”语义。这样代码可以先落地、测试，
//! 但在 5 日 shadow 通过前不改变用户收到的推送。

pub fn reference_language_enabled() -> bool {
    std::env::var("TICKET_OUTPUT_CONTRACT")
        .map(|value| value == "1")
        .unwrap_or(false)
}
