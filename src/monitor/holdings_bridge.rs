//! Windows 持仓桥客户端（只读 HTTP，数据源是 TDX「资金股份」导出文件）。
//!
//! 桥由 Windows 侧提供，见 `client-bundle/WINDOWS_HOLDINGS_BRIDGE_20261020_V1.md`：
//!
//! - `GET /health`  → ok / tdxw_running / export_mtime / age_seconds / stale
//! - `GET /holdings` → snapshot_at / age_seconds / stale / cash / positions[]
//!
//! ## 为什么不用 gRPC
//!
//! `magic-market-grpc-server` 的 63 个 op 全是行情/数据类，明文写着「没有账户、
//! 资产、持仓、委托、撤单或成交写接口」。持仓只能走这条终端导出桥。
//!
//! ## 两条硬规则
//!
//! 1. **持仓量取 `stock_quantity`，不是 `quantity`**。当日买入的票 `quantity=0`
//!    但 `stock_quantity=500`，用 `quantity` 会让它从持仓里消失。
//! 2. **过期不写**。`age_seconds` 超过 `MAX_ACCEPT_AGE_SECS` 直接拒绝——
//!    写入一份十几小时前的快照比不写更糟：它会顶掉当天真实的确认快照。

/// 超过这个年龄的导出快照拒收（12 小时）。
///
/// 盘后同步时导出通常几分钟前刷新过；十几小时前说明终端没刷新，
/// 拿它顶掉当天快照会把持仓写回旧状态。
pub const MAX_ACCEPT_AGE_SECS: f64 = 12.0 * 3600.0;

#[derive(Debug, Clone)]
pub struct BridgePosition {
    pub code: String,
    pub name: String,
    /// 库存数量（实际持有）。不是 `quantity`（可卖数量）。
    pub stock_quantity: f64,
    pub cost_price: f64,
    pub flags: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct BridgeHoldings {
    pub snapshot_at: String,
    pub age_seconds: f64,
    pub stale: bool,
    pub account_tail: String,
    pub positions: Vec<BridgePosition>,
}

/// 拉一次 `/holdings`（异步，经 `/usr/bin/curl` 子进程）。
///
/// ## 为什么不用 reqwest
///
/// 2026-10-20 实测：同一个 `http://10.211.55.3:18090/holdings`，
///
/// | 调用方                          | 结果 |
/// |---------------------------------|------|
/// | `/usr/bin/curl`（shell）        | 200  |
/// | `/usr/bin/curl`（launchd job）  | 200  |
/// | `/usr/bin/python3`（两种上下文）| 200  |
/// | monitor 进程内 reqwest（blocking 和 async 都试过） | `No route to host (os error 65)` |
///
/// monitor 同进程连外网 HTTPS（61.170.82.99:443）和 localhost 都正常，只有
/// Parallels 内网 10.211.55.3 不通；plist 没有任何网络限制，也没有代理变量。
/// 系统二进制通、自建二进制不通，指向 macOS 本地网络权限对后台自建二进制
/// 的拒绝（EHOSTUNREACH 是它被拒时的表现，不是真的没有路由——路由表里
/// `10.211.55.3 → bridge100` 一直在）。
///
/// 所以传输交给 `/usr/bin/curl`，解析留在 Rust。token 通过 `-K -` 从 stdin
/// 传入，不进 argv（`ps` 看不到）。
pub async fn fetch_async(base_url: &str, token: &str) -> Result<BridgeHoldings, String> {
    use tokio::io::AsyncWriteExt;

    let url = format!("{}/holdings", base_url.trim_end_matches('/'));
    // curl 配置文件格式：header / url 各一行。token 只出现在这条管道里。
    let config = format!("header = \"Authorization: Bearer {token}\"\nurl = \"{url}\"\n");

    let mut child = tokio::process::Command::new("/usr/bin/curl")
        .args([
            "-sS",
            "-m",
            "20",
            "-K",
            "-",
            "-w",
            "\n__HTTP_CODE__%{http_code}",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| format!("启动 curl 失败: {error}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(config.as_bytes())
            .await
            .map_err(|error| format!("向 curl 写配置失败: {error}"))?;
        // 显式关闭 stdin，否则 curl 会一直等配置结束。
        drop(stdin);
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|error| format!("等待 curl 失败: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "请求 {url} 失败: curl exit={:?} stderr={}",
            output.status.code(),
            stderr.trim()
        ));
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let (body, code) = match raw.rsplit_once("__HTTP_CODE__") {
        Some((body, code)) => (body, code.trim()),
        None => return Err(format!("curl 输出缺少状态码标记: {}", raw.trim())),
    };
    if code != "200" {
        return Err(format!("{url} 返回 HTTP {code}"));
    }

    let value: serde_json::Value =
        serde_json::from_str(body.trim()).map_err(|error| format!("解析 /holdings JSON 失败: {error}"))?;

    parse_holdings(&value)
}

/// 同步版本，供 CLI 使用（同样走 curl，避免 CLI 与 monitor 行为不一致）。
pub fn fetch(base_url: &str, token: &str) -> Result<BridgeHoldings, String> {
    use std::io::Write;

    let url = format!("{}/holdings", base_url.trim_end_matches('/'));
    let config = format!("header = \"Authorization: Bearer {token}\"\nurl = \"{url}\"\n");

    let mut child = std::process::Command::new("/usr/bin/curl")
        .args([
            "-sS",
            "-m",
            "20",
            "-K",
            "-",
            "-w",
            "\n__HTTP_CODE__%{http_code}",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| format!("启动 curl 失败: {error}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(config.as_bytes())
            .map_err(|error| format!("向 curl 写配置失败: {error}"))?;
        drop(stdin);
    }

    let output = child
        .wait_with_output()
        .map_err(|error| format!("等待 curl 失败: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "请求 {url} 失败: curl exit={:?} stderr={}",
            output.status.code(),
            stderr.trim()
        ));
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let (body, code) = match raw.rsplit_once("__HTTP_CODE__") {
        Some((body, code)) => (body, code.trim()),
        None => return Err(format!("curl 输出缺少状态码标记: {}", raw.trim())),
    };
    if code != "200" {
        return Err(format!("{url} 返回 HTTP {code}"));
    }

    let value: serde_json::Value =
        serde_json::from_str(body.trim()).map_err(|error| format!("解析 /holdings JSON 失败: {error}"))?;

    parse_holdings(&value)
}

/// 把 `/holdings` 的 JSON 解析成结构体。独立出来便于测试。
pub fn parse_holdings(value: &serde_json::Value) -> Result<BridgeHoldings, String> {
    let snapshot_at = value
        .get("snapshot_at")
        .and_then(|v| v.as_str())
        .ok_or("缺少 snapshot_at")?
        .to_string();
    let age_seconds = value
        .get("age_seconds")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let stale = value
        .get("stale")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let account_tail = value
        .get("account_tail")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let positions_value = value
        .get("positions")
        .and_then(|v| v.as_array())
        .ok_or("缺少 positions 数组")?;

    let mut positions = Vec::new();
    for item in positions_value {
        let code = match item.get("code").and_then(|v| v.as_str()) {
            Some(code) => code.to_string(),
            None => continue,
        };
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        // 用 stock_quantity（库存），不是 quantity（可卖）。
        let stock_quantity = item
            .get("stock_quantity")
            .and_then(|v| v.as_f64())
            .or_else(|| item.get("quantity").and_then(|v| v.as_f64()))
            .unwrap_or(0.0);
        let cost_price = item
            .get("cost_price")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let flags = item
            .get("flags")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        // 库存为 0 的票不是持仓（已清仓或只有可卖记录），不进快照。
        if stock_quantity <= 0.0 {
            continue;
        }
        positions.push(BridgePosition {
            code,
            name,
            stock_quantity,
            cost_price,
            flags,
        });
    }

    Ok(BridgeHoldings {
        snapshot_at,
        age_seconds,
        stale,
        account_tail,
        positions,
    })
}

/// 转成 `import_user_position_snapshot` 吃的 JSON。
///
/// `effective_at` 用桥的 `snapshot_at`（导出时刻），不是拉取时刻——
/// 否则一份两小时前的导出会冒充「现在的持仓」。
pub fn to_snapshot_json(holdings: &BridgeHoldings) -> String {
    let items: Vec<serde_json::Value> = holdings
        .positions
        .iter()
        .map(|position| {
            serde_json::json!({
                "code": position.code,
                "name": position.name,
                "quantity": position.stock_quantity as u64,
                "cost_price": position.cost_price,
            })
        })
        .collect();

    serde_json::json!({
        "schema_version": 1,
        "effective_at": holdings.snapshot_at,
        "confirm_empty": false,
        "items": items,
    })
    .to_string()
}

/// 判断这份快照能不能落库。返回 Err 时调用方应跳过写入并记 warn。
pub fn accept(holdings: &BridgeHoldings) -> Result<(), String> {
    if holdings.positions.is_empty() {
        return Err("桥返回 0 只持仓，拒绝用空快照顶掉现有记录".to_string());
    }
    if holdings.age_seconds > MAX_ACCEPT_AGE_SECS {
        return Err(format!(
            "导出快照已 {:.1} 小时前（上限 {:.0} 小时），拒绝写入",
            holdings.age_seconds / 3600.0,
            MAX_ACCEPT_AGE_SECS / 3600.0
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> serde_json::Value {
        serde_json::json!({
            "snapshot_at": "2026-10-20T17:07:37+08:00",
            "age_seconds": 120.0,
            "stale": false,
            "account_tail": "4106",
            "positions": [
                {"code": "002131", "name": "利欧股份", "quantity": 1000,
                 "stock_quantity": 1000, "cost_price": 10.081, "flags": []},
                {"code": "603915", "name": "国茂股份", "quantity": 0,
                 "stock_quantity": 500, "cost_price": 12.79,
                 "flags": ["quantity_mismatch", "not_sellable"]},
                {"code": "000001", "name": "已清仓", "quantity": 0,
                 "stock_quantity": 0, "cost_price": 9.9, "flags": []}
            ]
        })
    }

    #[test]
    fn uses_stock_quantity_not_sellable_quantity() {
        let holdings = parse_holdings(&sample()).unwrap();
        let guomao = holdings
            .positions
            .iter()
            .find(|p| p.code == "603915")
            .expect("国茂必须在持仓里");
        // quantity=0 但 stock_quantity=500 —— 当日买入 T+1
        assert_eq!(guomao.stock_quantity, 500.0);
    }

    #[test]
    fn zero_stock_quantity_is_not_a_position() {
        let holdings = parse_holdings(&sample()).unwrap();
        assert_eq!(holdings.positions.len(), 2);
        assert!(holdings.positions.iter().all(|p| p.code != "000001"));
    }

    #[test]
    fn empty_positions_are_rejected() {
        let holdings = BridgeHoldings {
            snapshot_at: "2026-10-20T17:07:37+08:00".to_string(),
            age_seconds: 60.0,
            stale: false,
            account_tail: "4106".to_string(),
            positions: vec![],
        };
        assert!(accept(&holdings).is_err());
    }

    #[test]
    fn too_old_snapshot_is_rejected() {
        let mut holdings = parse_holdings(&sample()).unwrap();
        holdings.age_seconds = MAX_ACCEPT_AGE_SECS + 1.0;
        assert!(accept(&holdings).is_err());
    }

    #[test]
    fn snapshot_json_uses_bridge_time_not_now() {
        let holdings = parse_holdings(&sample()).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&to_snapshot_json(&holdings)).unwrap();
        assert_eq!(json["effective_at"], "2026-10-20T17:07:37+08:00");
        assert_eq!(json["items"][1]["quantity"], 500);
    }
}