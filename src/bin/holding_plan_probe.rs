//! holding_plan 的 owner 读取探针。
//!
//! 背景：P4（HoldingPlan 次日动作）自 2026-09-23 起停供，日志只有
//! `holding_plan_owner_read_failed`，`{e}` 渲染为空。改成 `{e:#}` 后仍要等
//! 交易时段 I-04 触发才能看到真因。
//!
//! `inspect_holding_plan_occurrence` 是 lib pub，`CoordinatorConfig::production`
//! 接受任意 owner identity —— 不匹配的 owner 走 `Ok(None)` → Missing，不是错误。
//! 所以这里能在任意时刻复现同一条读取路径，不必等 10:30。
//!
//! 用法：
//!   STOCK_ROOT=<运行根> holding_plan_probe <代码> <日期>
//!   例：STOCK_ROOT=/Users/.../stock-analysis-runtime holding_plan_probe 601811 2026-10-16

use anyhow::{Context, Result};
use chrono::NaiveDate;
use stock_analysis::durable_delivery::{CoordinatorConfig, DurableDeliveryCoordinator};
use stock_analysis::market_domain::{AssetClass, Exchange, InstrumentId};

fn exchange_for(code: &str) -> Exchange {
    match code.chars().next() {
        Some('6') => Exchange::Shanghai,
        Some('8') | Some('4') => Exchange::Beijing,
        _ => Exchange::Shenzhen,
    }
}

fn main() -> Result<()> {
    let code = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "600000".to_string());
    let date_str = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "2026-10-16".to_string());
    let date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d")
        .with_context(|| format!("日期格式应为 YYYY-MM-DD: {date_str}"))?;
    let inst = InstrumentId::new(exchange_for(&code), &code, AssetClass::Equity)?;

    println!(
        "[probe] code={code} exchange={:?} date={date}",
        inst.exchange()
    );

    let config = CoordinatorConfig::production("probe-holding-plan");
    let coordinator = DurableDeliveryCoordinator::open_existing_monitor_schema9(config)
        .context("打开 durable_delivery（schema9）失败")?;
    println!("[probe] coordinator 已打开");

    match coordinator.inspect_holding_plan_occurrence(date, &inst) {
        Ok(obs) => println!("[probe] OK: {obs:?}"),
        Err(e) => {
            println!("[probe] ERR Display : {e}");
            println!("[probe] ERR 因果链  : {e:#}");
            println!("[probe] ERR Debug   : {e:?}");
        }
    }
    Ok(())
}
