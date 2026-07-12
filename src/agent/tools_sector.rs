use crate::agent::tool::Tool;
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct FetchSectorTool {
    client: reqwest::Client,
}

impl FetchSectorTool {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .user_agent(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
                 AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            )
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }
}

/// 转 A 股代码为东财 SECUCODE，格式 `601991.SH` / `000001.SZ` / `430489.BJ`
fn to_secucode_dot(code: &str) -> String {
    let suffix = if code.starts_with('6') || code.starts_with("688") || code.starts_with("900") {
        "SH"
    } else if code.starts_with('0') || code.starts_with('3') || code.starts_with("200") {
        "SZ"
    } else if code.starts_with('8') || code.starts_with('4') {
        "BJ"
    } else {
        "SH"
    };
    format!("{}.{}", code, suffix)
}

#[async_trait]
impl Tool for FetchSectorTool {
    fn name(&self) -> &str {
        "fetch_sector_concepts"
    }

    fn description(&self) -> &str {
        "获取指定 A 股的所属行业板块与概念板块（来自东方财富 F10 核心题材），用于评估板块联动与题材属性。"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "股票代码，如 '600519' 或 '000001'"
                }
            },
            "required": ["code"]
        })
    }

    async fn call(&self, input: serde_json::Value) -> anyhow::Result<String> {
        let code = input
            .get("code")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'code' parameter"))?;

        let secucode = to_secucode_dot(code);
        // 东财 F10 「核心题材-所属板块」接口（v1）
        let url = format!(
            "https://datacenter-web.eastmoney.com/api/data/v1/get\
             ?reportName=RPT_F10_CORETHEME_BOARDTYPE\
             &columns=ALL\
             &filter=(SECUCODE%3D%22{}%22)\
             &pageNumber=1&pageSize=200",
            secucode
        );
        log::debug!("[板块] {}", url);

        let resp = self
            .client
            .get(&url)
            .header("Referer", "https://emweb.securities.eastmoney.com/")
            .send()
            .await;
        let body: Value = match resp {
            Ok(r) => match r.json().await {
                Ok(v) => v,
                Err(e) => {
                    return Ok(
                        json!({"error": format!("板块接口 JSON 解析失败: {}", e)}).to_string()
                    )
                }
            },
            Err(e) => return Ok(json!({"error": format!("板块接口请求失败: {}", e)}).to_string()),
        };

        let arr = body
            .pointer("/result/data")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        if arr.is_empty() {
            return Ok(json!({"error": "未查询到板块数据", "secucode": secucode}).to_string());
        }

        // 按 BOARD_RANK 升序：rank 越小越"主营"。该接口 BOARD_TYPE 当前返回 null，
        // 暂不区分行业/概念，直接平铺成 boards 列表，再标注 rank。
        let mut boards: Vec<(i64, String)> = Vec::new();
        for item in &arr {
            let name = item
                .get("BOARD_NAME")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if name.is_empty() {
                continue;
            }
            let rank = item
                .get("BOARD_RANK")
                .and_then(|v| v.as_i64())
                .unwrap_or(99);
            boards.push((rank, name.to_string()));
        }
        boards.sort_by_key(|(r, _)| *r);
        let board_names: Vec<&str> = boards.iter().map(|(_, n)| n.as_str()).collect();
        // 头部 3 个一般是主营行业，后面是概念/指数（启发式，仅作提示）
        let primary: Vec<&str> = board_names.iter().take(3).copied().collect();
        let secondary: Vec<&str> = board_names.iter().skip(3).copied().collect();

        let result = json!({
            "fetched": true,
            "secucode": secucode,
            "primary_boards": primary,
            "secondary_boards": secondary,
            "all_boards": board_names,
            "board_count": board_names.len(),
            "note": "数据源：东方财富 F10 核心题材 (RPT_F10_CORETHEME_BOARDTYPE)；按 BOARD_RANK 排序，前 3 项一般为主营行业，其余为概念/指数。"
        });

        Ok(result.to_string())
    }
}
