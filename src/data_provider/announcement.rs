//! Registered business rules: BR-059.
//! A股公告抓取（东方财富公告API）。
//!
//! 策略：标题即风控——含关键词直接告警，不等正文。
//! 非高危公告用东方财富编辑摘要，不做 PDF 解析。
//! 单次超200条自动熔断，仅扫描标题。

use anyhow::Result;
use log::{info, warn};
use serde::Deserialize;
use std::sync::Arc;

const ANNOUNCE_URL: &str = "https://np-anotice-stock.eastmoney.com/api/security/ann";
const MAX_PER_FETCH: usize = 200;

#[derive(Debug, Deserialize)]
struct AnnResponse {
    data: Option<AnnData>,
}

#[derive(Debug, Deserialize)]
struct AnnData {
    list: Vec<AnnItem>,
}

#[derive(Debug, Deserialize)]
struct DetailResponse {
    data: Option<DetailData>,
}

#[derive(Debug, Deserialize)]
struct DetailData {
    content: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct AnnItem {
    art_code: String,
    title: String,
    notice_date: String,
    /// 关联股票列表（codes[0] 通常是主股票）
    codes: Vec<AnnCode>,
    /// 公告分类（columns[0].column_name 如"召开股东大会通知"）
    columns: Option<Vec<AnnColumn>>,
}

#[derive(Debug, Deserialize, Clone)]
struct AnnCode {
    stock_code: String,
    short_name: String,
}

#[derive(Debug, Deserialize, Clone)]
struct AnnColumn {
    column_name: Option<String>,
}

/// 告警级别
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AnnLevel {
    Emergency,
    Important,
    Info,
    Skip,
}

/// 解析后的公告
#[derive(Debug, Clone)]
pub struct Announcement {
    pub code: String,
    pub name: String,
    pub title: String,
    pub date: String,
    pub summary: String,
    pub content: String,
    pub level: AnnLevel,
    pub reason: String,
    /// 外部唯一标识（来源于 art_code）
    pub external_id: Option<String>,
    /// 东方财富公告详情页 URL
    pub url: Option<String>,
}

// ── 标题关键词哨兵 ──

const EMERGENCY_KEYWORDS: &[&str] = &[
    // 监管立案与调查
    "立案调查",
    "信息披露违法违规",
    "财务造假",
    "涉嫌信息披露违规",
    "涉嫌操纵市场",
    "涉嫌内幕交易",
    "涉嫌欺诈发行",
    // 退市相关
    "终止上市",
    "强制退市",
    "退市整理期",
    "退市风险警示",
    "面值退市",
    "交易类强制退市",
    // ST风险
    "ST风险",
    "被实施退市风险警示",
    "实施其他风险警示",
    // 审计问题
    "无法表示意见",
    "否定意见",
    "审计否定意见",
    "审计保留意见",
    "否定意见的审计报告",
    "非标准审计意见",
    // 破产/清算
    "破产",
    "清算",
    "破产重整",
    "破产清算",
    "申请破产",
    // 公司治理严重问题
    "实际控制人失联",
    "实际控制人被",
    "违规担保",
    "非经营性资金占用",
    "大股东占用资金",
    "资金占用",
    // 债务违约
    "债务违约",
    "债券违约",
    "实质性违约",
    // 暂停上市
    "暂停上市",
    "暂停交易",
    // 重大违法
    "重大违法违规",
    "重大违法强制退市",
    // 交易所处分
    "公开谴责",
    "通报批评",
];

const IMPORTANT_KEYWORDS: &[&str] = &[
    // 减持相关（代码内置 <1% 过滤仅对"减持"生效）
    "减持",
    "预减持",
    "减持计划",
    "减持结果",
    // 监管问询与处分
    "问询函",
    "关注函",
    "监管函",
    "警示函",
    "责令改正",
    "监管问询",
    "行政监管措施",
    "行政处罚",
    "立案告知书",
    // 业绩负面
    "业绩预亏",
    "业绩预减",
    "业绩修正",
    "业绩变脸",
    "业绩下滑",
    "预计亏损",
    "净利润亏损",
    // 资产减值
    "商誉减值",
    "资产减值",
    "计提减值",
    "计提资产减值",
    "信用减值",
    "存货跌价",
    // 诉讼与司法
    "重大诉讼",
    "诉讼",
    "仲裁",
    "司法拍卖",
    "司法冻结",
    "轮候冻结",
    "质押",
    "冻结",
    // 重组负面（先于 Positive 的"重组/并购"被检查，避免误命中）
    "重组终止",
    "重组失败",
    "重组暂停",
    "终止重组",
    "终止筹划重组",
    "终止重大资产重组",
    "终止本次重组",
    "终止发行",
    "终止本次发行",
    "并购失败",
    "并购终止",
    // 公司治理
    "高管辞职",
    "总经理辞职",
    "财务负责人辞职",
    "会计师事务所变更",
    "审计机构变更",
    "独立董事辞职",
    "董事会秘书辞职",
    // 债务与担保
    "债务逾期",
    "借款逾期",
    "贷款逾期",
    "对外担保",
    "担保逾期",
    "大额担保",
    // 经营风险
    "限售股解禁",
    "暂停生产",
    "停产",
    "安全事故",
    "重大事故",
    "爆炸",
    "环保处罚",
    "环保督察",
    "责令停产",
    // 其他风险
    "募集资金变更",
    "募投项目延期",
    "控制权变更",
    "实际控制人变更",
    "关联方占用",
    "违规关联交易",
];

const POSITIVE_KEYWORDS: &[&str] = &[
    // 回购与增持
    "回购",
    "增持",
    "股份增持",
    "回购股份",
    "回购方案",
    "回购注销",
    // 分红与送转
    "分红",
    "现金分红",
    "高分红",
    "派息",
    "高送转",
    "高比例分红",
    "中期分红",
    "特别分红",
    // 业绩利好
    "业绩预增",
    "业绩预喜",
    "扭亏为盈",
    "扭亏",
    "业绩快报",
    "业绩高增",
    "业绩大幅增长",
    // 股权激励与员工持股
    "股权激励",
    "限制性股票",
    "股票期权",
    "员工持股",
    "员工持股计划",
    "股权激励计划",
    // 订单与合同
    "中标",
    "中标项目",
    "中标合同",
    "中标公告",
    "重大合同",
    "签订合同",
    "获得订单",
    // 战略合作
    "战略合作",
    "战略合作框架",
    "战略合作协议",
    // 重组与并购利好（Important 的"重组终止/失败"先检查，此处不再误命中）
    "重组",
    "重大资产重组",
    "并购",
    "资产注入",
    "借壳上市",
    "整体上市",
    // 批文与新业务
    "获得批文",
    "新产品获批",
    "新药获批",
    "获批上市",
    "获得注册证",
    "产品获批",
    "临床试验获批",
    // 定增与融资
    "定增",
    "非公开发行",
    "募资",
    "再融资",
    "引进战略投资者",
    "战投",
    "混改",
    // 举牌与收购
    "举牌",
    "要约收购",
    "收购完成",
    // 产能与技术突破
    "产能释放",
    "产能扩建",
    "投产",
    "竣工投产",
    "技术突破",
    "重大突破",
    // 业务拓展
    "业务拓展",
    "海外布局",
    "国际化布局",
    "签订协议",
    "战略协议",
];

fn classify_title(title: &str, _code: &str, _name: &str) -> (AnnLevel, String) {
    // review #14 性能: 原 fallback 分支 3 次 .map(to_string).collect() 触发 3 次堆分配
    // (EMERGENCY 50 + IMPORTANT 50 + POSITIVE 30 ≈ 130 个 String). 公告热路径
    // (200+ 条/批), 每天触发数百次. 改为按配置来源直接传 &[&str], const 路径零分配.
    // review #14 性能: 原 fallback 分支 3 次 .map(to_string).collect() 触发 3 次堆分配
    // (EMERGENCY 50 + IMPORTANT 50 + POSITIVE 30 ≈ 130 个 String). 公告热路径
    // (200+ 条/批), 每天触发数百次. 改为按配置来源直接传 &[&str], const 路径零分配.
    // 用 enum 统一两种来源 (const &[&str] / config Vec<String>).
    enum KwList<'a> {
        Static(&'a [&'a str]),
        Owned(&'a [String]),
    }
    impl<'a> KwList<'a> {
        fn first_match(&self, s: &str) -> Option<String> {
            match self {
                KwList::Static(v) => v.iter().find(|k| s.contains(**k)).map(|k| k.to_string()),
                KwList::Owned(v) => v.iter().find(|k| s.contains(k.as_str())).cloned(),
            }
        }
        /// review #14: 跳过指定 keyword, 找下一个匹配 (用于减持 <1% 降级场景).
        fn first_match_skip(&self, s: &str, skip: &str) -> Option<String> {
            match self {
                KwList::Static(v) => v
                    .iter()
                    .find(|k| **k != skip && s.contains(**k))
                    .map(|k| k.to_string()),
                KwList::Owned(v) => v
                    .iter()
                    .find(|k| k.as_str() != skip && s.contains(k.as_str()))
                    .cloned(),
            }
        }
    }
    // 模块级静态缓存: 配置 keyword 一旦加载就永久驻留 (review #14 + 15).
    // review #15 修正: 原 Lazy<(Vec, Vec, Vec)> + is_empty() 区分不了"配置未加载"
    // 和"配置加载但 Vec 为空". 用 OnceLock<Option<(Vec,Vec,Vec)>> 显式区分.
    // 同时去掉冗余的 `|| get_announce_keywords().is_some()` (每次 classify_title
    // 调一次 ArcSwap load + Arc clone, 完全违背零分配意图).
    type KeywordGroups = (Vec<String>, Vec<String>, Vec<String>);
    static CACHED_CFG: std::sync::OnceLock<Option<KeywordGroups>> = std::sync::OnceLock::new();
    // review #15 改进: 首次 init 时如果 config 缺失 (None), 显式 log warn.
    // AGENTS.md §2.2 要求 "missing data fields MUST be left blank or logged as warnings;
    // MUST NOT be silently filled" — config 缺失 = 走 const fallback 是 silent fill.
    // 用 OnceLock 状态一次性记录是否 warn 过 (避免每次 classify_title 重复打).
    static CFG_MISSING_WARNED: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    let (emergency, important, positive) = CACHED_CFG
        .get_or_init(|| {
            crate::config::get_announce_keywords().map(|cfg| {
                (
                    cfg.emergency.clone(),
                    cfg.important.clone(),
                    cfg.positive.clone(),
                )
            })
        })
        .as_ref()
        .map(|(e, i, p)| (KwList::Owned(e), KwList::Owned(i), KwList::Owned(p)))
        .unwrap_or_else(|| {
            // review #15: 仅首次 fallback 时 log warn 一次 (避免每公告重复打印).
            if !CFG_MISSING_WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                log::warn!(
                    "[announcement] announce_keywords 配置缺失, 走 const 编译期 fallback \
                     ({} EMERGENCY / {} IMPORTANT / {} POSITIVE). 检查 config/chain.toml \
                     announce_keywords 段是否合法.",
                    EMERGENCY_KEYWORDS.len(),
                    IMPORTANT_KEYWORDS.len(),
                    POSITIVE_KEYWORDS.len()
                );
            }
            (
                KwList::Static(EMERGENCY_KEYWORDS),
                KwList::Static(IMPORTANT_KEYWORDS),
                KwList::Static(POSITIVE_KEYWORDS),
            )
        });

    if let Some(kw) = emergency.first_match(title) {
        return (AnnLevel::Emergency, format!("标题含'{kw}'，直接告警"));
    }
    // 减持特例: <1% 不算重要, 跳过此 kw 重找下一个.
    // review #14: 改 first_match_skip 替代之前的 inner-loop continue.
    let important_kw = if let Some(pct) = title
        .find("减持")
        .and_then(|_| extract_reduction_pct(title))
    {
        if pct < 1.0 {
            important.first_match_skip(title, "减持")
        } else {
            important.first_match(title)
        }
    } else {
        important.first_match(title)
    };
    if let Some(kw) = important_kw {
        return (AnnLevel::Important, format!("标题含'{kw}'"));
    }
    if let Some(kw) = positive.first_match(title) {
        return (AnnLevel::Info, format!("利好: '{kw}'"));
    }
    (AnnLevel::Skip, String::new())
}

fn extract_reduction_pct(title: &str) -> Option<f64> {
    // 从标题提取减持比例，如"减持不超过3%"→3.0
    for part in title.split(|c: char| !c.is_ascii_digit() && c != '.' && c != '%') {
        if let Some(pct_str) = part.strip_suffix('%') {
            if let Ok(pct) = pct_str.parse::<f64>() {
                return Some(pct);
            }
        }
    }
    None
}

fn validate_announcement_response(resp: AnnResponse) -> Result<Vec<AnnItem>> {
    let list = resp
        .data
        .ok_or_else(|| anyhow::anyhow!("公告响应缺少 data"))?
        .list;
    for (index, item) in list.iter().enumerate() {
        if item.art_code.trim().is_empty() || item.title.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "公告第 {} 行 art_code/title 缺失",
                index + 1
            ));
        }
        let notice_date = item
            .notice_date
            .get(..10)
            .ok_or_else(|| anyhow::anyhow!("公告 {} notice_date 字段不足", item.art_code))?;
        chrono::NaiveDate::parse_from_str(notice_date, "%Y-%m-%d")
            .map_err(|error| anyhow::anyhow!("公告 {} notice_date 非法: {error}", item.art_code))?;
        let code = item
            .codes
            .first()
            .ok_or_else(|| anyhow::anyhow!("公告 {} 缺少关联股票", item.art_code))?;
        if code.stock_code.len() != 6
            || !code.stock_code.bytes().all(|byte| byte.is_ascii_digit())
            || code.short_name.trim().is_empty()
        {
            return Err(anyhow::anyhow!(
                "公告 {} 股票 code/name 非法",
                item.art_code
            ));
        }
    }
    Ok(list)
}

fn parse_announcement_http_response(
    status: u16,
    body: std::result::Result<String, String>,
) -> Result<Vec<AnnItem>> {
    if !(200..300).contains(&status) {
        return Err(anyhow::anyhow!("公告 HTTP 状态异常: {status}"));
    }
    let body = body.map_err(|error| anyhow::anyhow!("公告正文读取失败: {error}"))?;
    if body.trim().is_empty() {
        return Err(anyhow::anyhow!("公告响应正文为空"));
    }
    let response: AnnResponse = serde_json::from_str(&body)
        .map_err(|error| anyhow::anyhow!("公告响应 JSON 非法: {error}"))?;
    validate_announcement_response(response)
}

fn parse_announcement_detail_http_response(
    status: u16,
    body: std::result::Result<String, String>,
    art_code: &str,
) -> Result<String> {
    if !(200..300).contains(&status) {
        return Err(anyhow::anyhow!(
            "ann detail {art_code} HTTP status {status}"
        ));
    }
    let body = body.map_err(|error| anyhow::anyhow!("ann detail {art_code} read: {error}"))?;
    if body.trim().is_empty() {
        return Err(anyhow::anyhow!("ann detail {art_code} empty body"));
    }
    let response: DetailResponse = serde_json::from_str(&body)
        .map_err(|error| anyhow::anyhow!("ann detail {art_code} json: {error}"))?;
    response
        .data
        .and_then(|data| data.content)
        .filter(|content| !content.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("ann detail {art_code} missing content"))
}

fn detail_art_codes(list: &[AnnItem]) -> Vec<String> {
    list.iter()
        .filter_map(|item| {
            let (level, _) = classify_title(&item.title, "", "");
            matches!(level, AnnLevel::Emergency | AnnLevel::Important)
                .then(|| item.art_code.clone())
        })
        .collect()
}

fn assemble_announcements(
    list: Vec<AnnItem>,
    detail_map: &std::collections::HashMap<String, String>,
) -> Result<Vec<Announcement>> {
    let mut results = Vec::new();
    for item in list {
        let stock = item
            .codes
            .first()
            .expect("announcement stock evidence was validated");
        let (level, reason) = classify_title(&item.title, &stock.stock_code, &stock.short_name);
        if matches!(level, AnnLevel::Skip) {
            continue;
        }

        let summary = item
            .columns
            .as_ref()
            .and_then(|columns| columns.first())
            .and_then(|column| column.column_name.as_deref())
            .unwrap_or("")
            .to_string();
        let art_code = item.art_code.clone();
        let content = if matches!(level, AnnLevel::Emergency | AnnLevel::Important) {
            detail_map
                .get(&art_code)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("公告 {art_code} 正文批次缺失"))?
        } else {
            String::new()
        };

        results.push(Announcement {
            code: stock.stock_code.clone(),
            name: stock.short_name.clone(),
            title: item.title,
            date: item.notice_date,
            summary,
            content,
            level,
            reason,
            external_id: Some(art_code.clone()),
            url: Some(format!(
                "https://data.eastmoney.com/notices/detail/{}.html",
                art_code
            )),
        });
    }
    Ok(results)
}

// ── API 拉取 ──

/// review #15: 改 async + FuturesUnordered 并发 fetch_ann_detail.
pub async fn fetch_announcements(date: Option<&str>) -> Result<Vec<Announcement>> {
    let client = crate::http_client::SHARED_HTTP_CLIENT.clone();
    fetch_announcements_with_client(&client, date).await
}

async fn fetch_announcements_with_client(
    client: &reqwest::Client,
    date: Option<&str>,
) -> Result<Vec<Announcement>> {
    fetch_announcements_from_url(client, date, ANNOUNCE_URL).await
}

async fn fetch_announcements_from_url(
    client: &reqwest::Client,
    date: Option<&str>,
    announcement_url: &str,
) -> Result<Vec<Announcement>> {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let date_str = date.unwrap_or(&today);
    chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
        .map_err(|error| anyhow::anyhow!("公告查询日期非法 {date_str:?}: {error}"))?;

    let url = format!(
        "{}?page_size={}&page_index=1&ann_type=SHA,SZA&start_date={}&end_date={}",
        announcement_url, MAX_PER_FETCH, date_str, date_str
    );

    let response = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .header("Referer", "https://data.eastmoney.com/")
        .send()
        .await?;
    let status = response.status().as_u16();
    let body = response.text().await.map_err(|error| error.to_string());

    let list = parse_announcement_http_response(status, body)?;
    info!("[公告] {} 获取 {} 条", date_str, list.len());

    // 熔断：超 200 条仅标题扫描
    if list.len() >= MAX_PER_FETCH {
        warn!("[公告] 单日 {} 条，触发熔断，仅标题扫描", list.len());
    }

    // review #15: 高危公告 body 拉取改成 FuturesUnordered 并发, 不再串行 N × 10s.
    let client_arc = Arc::new(client.clone());
    let detail_futures = detail_art_codes(&list);
    let detail_results = futures::future::join_all(detail_futures.iter().map(|art_code| {
        let c = Arc::clone(&client_arc);
        let ac = art_code.clone();
        let detail_base = announcement_url.to_string();
        async move {
            let content = fetch_ann_detail_from_url(&c, &ac, &detail_base).await?;
            Ok::<_, anyhow::Error>((ac, content))
        }
    }))
    .await
    .into_iter()
    .collect::<Result<Vec<_>>>()?;
    let detail_map: std::collections::HashMap<String, String> =
        detail_results.into_iter().collect();

    let results = assemble_announcements(list, &detail_map)?;

    info!("[公告] 过滤后 {} 条需告警", results.len());
    crate::monitor::data_mode::mark_capability_success(crate::monitor::data_mode::Capability::News)
        .map_err(anyhow::Error::msg)?;
    Ok(results)
}

async fn fetch_ann_detail_from_url(
    client: &reqwest::Client,
    art_code: &str,
    announcement_url: &str,
) -> Result<String> {
    let url = format!("{announcement_url}/detail?art_code={art_code}");
    let response = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .header("Referer", "https://data.eastmoney.com/")
        .send()
        .await?;
    let status = response.status().as_u16();
    let body = response.text().await.map_err(|error| error.to_string());
    parse_announcement_detail_http_response(status, body, art_code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validated(raw: &str) -> Result<Vec<AnnItem>> {
        validate_announcement_response(serde_json::from_str(raw)?)
    }

    fn protocol_fixture() -> Vec<AnnItem> {
        validated(
            r#"{
                "data": {
                    "list": [
                        {
                            "art_code": "AN-EMERGENCY",
                            "title": "关于收到立案调查通知书的公告",
                            "notice_date": "2026-07-18 09:30:00",
                            "codes": [{"stock_code": "000001", "short_name": "协议样本甲"}],
                            "columns": [{"column_name": "风险提示"}]
                        },
                        {
                            "art_code": "AN-IMPORTANT",
                            "title": "关于收到监管函的公告",
                            "notice_date": "2026-07-18",
                            "codes": [{"stock_code": "000002", "short_name": "协议样本乙"}],
                            "columns": null
                        },
                        {
                            "art_code": "AN-INFO",
                            "title": "关于回购公司股份方案的公告",
                            "notice_date": "2026-07-18",
                            "codes": [{"stock_code": "000003", "short_name": "协议样本丙"}],
                            "columns": [{"column_name": null}]
                        },
                        {
                            "art_code": "AN-SKIP",
                            "title": "股东大会决议公告",
                            "notice_date": "2026-07-18",
                            "codes": [{"stock_code": "000004", "short_name": "协议样本丁"}]
                        }
                    ]
                }
            }"#,
        )
        .expect("local provider protocol fixture must be valid")
    }

    #[test]
    fn br105_announcement_protocol_requires_list_and_row_fields() {
        assert!(serde_json::from_str::<AnnResponse>(r#"{"data":{}}"#).is_err());
        assert!(serde_json::from_str::<AnnResponse>(
            r#"{"data":{"list":[{"art_code":"A1","title":"测试","notice_date":"2026-07-18","codes":[{}]}]}}"#
        )
        .is_err());
    }

    #[test]
    fn br105_announcement_response_rejects_missing_or_bad_values() {
        for raw in [
            r#"{"data":null}"#,
            r#"{"data":{"list":[{"art_code":"","title":"公告","notice_date":"2026-07-18","codes":[{"stock_code":"000001","short_name":"样本"}]}]}}"#,
            r#"{"data":{"list":[{"art_code":"A1","title":" ","notice_date":"2026-07-18","codes":[{"stock_code":"000001","short_name":"样本"}]}]}}"#,
            r#"{"data":{"list":[{"art_code":"A1","title":"公告","notice_date":"2026","codes":[{"stock_code":"000001","short_name":"样本"}]}]}}"#,
            r#"{"data":{"list":[{"art_code":"A1","title":"公告","notice_date":"2026-02-30","codes":[{"stock_code":"000001","short_name":"样本"}]}]}}"#,
            r#"{"data":{"list":[{"art_code":"A1","title":"公告","notice_date":"2026-07-18","codes":[]}]}}"#,
            r#"{"data":{"list":[{"art_code":"A1","title":"公告","notice_date":"2026-07-18","codes":[{"stock_code":"00001","short_name":"样本"}]}]}}"#,
            r#"{"data":{"list":[{"art_code":"A1","title":"公告","notice_date":"2026-07-18","codes":[{"stock_code":"00000A","short_name":"样本"}]}]}}"#,
            r#"{"data":{"list":[{"art_code":"A1","title":"公告","notice_date":"2026-07-18","codes":[{"stock_code":"000001","short_name":" "}]}]}}"#,
        ] {
            assert!(validated(raw).is_err(), "unexpectedly accepted {raw}");
        }
    }

    #[test]
    fn announcement_http_response_requires_complete_success_body() {
        let complete = r#"{"data":{"list":[{"art_code":"AN-HTTP","title":"关于回购股份的公告","notice_date":"2026-07-18","codes":[{"stock_code":"000001","short_name":"协议样本"}],"columns":null}]}}"#;
        let rows = parse_announcement_http_response(200, Ok(complete.to_string())).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].art_code, "AN-HTTP");

        for result in [
            parse_announcement_http_response(503, Ok(complete.to_string())),
            parse_announcement_http_response(200, Err("断流".to_string())),
            parse_announcement_http_response(200, Ok(String::new())),
            parse_announcement_http_response(200, Ok("<html>限流</html>".to_string())),
            parse_announcement_http_response(200, Ok(r#"{"data":null}"#.to_string())),
        ] {
            assert!(result.is_err());
        }
    }

    #[test]
    fn announcement_detail_http_response_requires_non_empty_content() {
        assert_eq!(
            parse_announcement_detail_http_response(
                200,
                Ok(r#"{"data":{"content":"完整公告正文"}}"#.to_string()),
                "AN-DETAIL",
            )
            .unwrap(),
            "完整公告正文"
        );

        for result in [
            parse_announcement_detail_http_response(
                404,
                Ok(r#"{"data":{"content":"正文"}}"#.to_string()),
                "AN-DETAIL",
            ),
            parse_announcement_detail_http_response(200, Err("断流".to_string()), "AN-DETAIL"),
            parse_announcement_detail_http_response(200, Ok(String::new()), "AN-DETAIL"),
            parse_announcement_detail_http_response(
                200,
                Ok("<html>错误</html>".to_string()),
                "AN-DETAIL",
            ),
            parse_announcement_detail_http_response(
                200,
                Ok(r#"{"data":null}"#.to_string()),
                "AN-DETAIL",
            ),
            parse_announcement_detail_http_response(
                200,
                Ok(r#"{"data":{"content":" "}}"#.to_string()),
                "AN-DETAIL",
            ),
        ] {
            assert!(result.is_err());
        }
    }

    #[test]
    fn br059_detail_selection_only_fetches_risk_announcements() {
        let list = protocol_fixture();
        assert_eq!(
            detail_art_codes(&list),
            vec!["AN-EMERGENCY".to_string(), "AN-IMPORTANT".to_string()]
        );
    }

    #[test]
    fn br059_announcement_assembly_requires_risk_details() {
        let list = protocol_fixture();
        let mut details = std::collections::HashMap::new();
        details.insert("AN-EMERGENCY".to_string(), "立案正文".to_string());
        assert!(assemble_announcements(list.clone(), &details).is_err());

        details.insert("AN-IMPORTANT".to_string(), "监管正文".to_string());
        let announcements = assemble_announcements(list, &details).unwrap();
        assert_eq!(announcements.len(), 3);

        let emergency = &announcements[0];
        assert_eq!(emergency.code, "000001");
        assert_eq!(emergency.name, "协议样本甲");
        assert_eq!(emergency.title, "关于收到立案调查通知书的公告");
        assert_eq!(emergency.date, "2026-07-18 09:30:00");
        assert_eq!(emergency.summary, "风险提示");
        assert_eq!(emergency.content, "立案正文");
        assert_eq!(emergency.level, AnnLevel::Emergency);
        assert!(emergency.reason.contains("立案调查"));
        assert_eq!(emergency.external_id.as_deref(), Some("AN-EMERGENCY"));
        assert_eq!(
            emergency.url.as_deref(),
            Some("https://data.eastmoney.com/notices/detail/AN-EMERGENCY.html")
        );

        let info = &announcements[2];
        assert_eq!(info.level, AnnLevel::Info);
        assert!(info.summary.is_empty());
        assert!(info.content.is_empty());
        assert_eq!(info.external_id.as_deref(), Some("AN-INFO"));
    }

    #[test]
    fn test_classify_emergency() {
        let (lvl, reason) = classify_title(
            "关于收到中国证监会立案调查通知书的公告",
            "TEST_CODE_000001",
            "测试",
        );
        assert_eq!(lvl, AnnLevel::Emergency);
        assert!(reason.contains("立案调查"));
    }

    #[test]
    fn test_classify_important() {
        let (lvl, _) = classify_title(
            "关于持股5%以上股东减持股份超过1%的公告",
            "TEST_CODE_000002",
            "测试",
        );
        assert_eq!(lvl, AnnLevel::Important);
    }

    #[test]
    fn test_classify_positive() {
        let (lvl, _) = classify_title("关于回购公司股份方案的公告", "TEST_CODE_000003", "测试");
        assert_eq!(lvl, AnnLevel::Info);
    }

    #[test]
    fn test_classify_normal_skip() {
        let (lvl, _) = classify_title(
            "2025年第三次临时股东大会决议公告",
            "TEST_CODE_000004",
            "测试",
        );
        assert_eq!(lvl, AnnLevel::Skip);
    }

    #[test]
    fn test_extract_reduction_pct() {
        assert!((extract_reduction_pct("减持不超过3%").unwrap() - 3.0).abs() < 0.01);
        assert!((extract_reduction_pct("减持比例不超过0.5%").unwrap() - 0.5).abs() < 0.01);
        assert!(extract_reduction_pct("无减持").is_none());
    }

    #[test]
    fn test_small_reduction_downgraded() {
        let (lvl, _) = classify_title(
            "关于股东减持股份不超过0.5%的提示性公告",
            "TEST_CODE_000005",
            "测试",
        );
        assert_eq!(lvl, AnnLevel::Skip); // <1% 不告警
    }

    #[test]
    fn test_financial_fraud_emergency() {
        let (lvl, _) = classify_title(
            "关于收到证监会涉嫌财务造假立案调查通知书的公告",
            "TEST_CODE_000006",
            "测试",
        );
        assert_eq!(lvl, AnnLevel::Emergency);
    }

    #[test]
    fn test_audit_denial_emergency() {
        let (lvl, _) = classify_title(
            "公司2025年度审计报告被出具否定意见",
            "TEST_CODE_000007",
            "测试",
        );
        assert_eq!(lvl, AnnLevel::Emergency);
    }

    #[test]
    fn test_restructure_failure_important() {
        // "重组终止" 必须在 Important 中被捕获，不能被 Positive 的 "重组" 误命中
        let (lvl, reason) =
            classify_title("关于终止重大资产重组事项的公告", "TEST_CODE_000008", "测试");
        assert_eq!(lvl, AnnLevel::Important);
        assert!(reason.contains("重组"));
    }

    #[test]
    fn test_restructure_plan_positive() {
        // 真正的重组利好仍应命中 Positive
        let (lvl, _) = classify_title(
            "关于重大资产重组预案暨关联交易的公告",
            "TEST_CODE_000009",
            "测试",
        );
        assert_eq!(lvl, AnnLevel::Info);
    }

    #[test]
    fn test_merger_failure_important() {
        let (lvl, _) = classify_title(
            "关于终止发行股份购买资产暨并购重组事项的公告",
            "TEST_CODE_000010",
            "测试",
        );
        assert_eq!(lvl, AnnLevel::Important);
    }

    #[test]
    fn test_equity_incentive_positive() {
        let (lvl, _) = classify_title(
            "关于向激励对象授予限制性股票的公告",
            "TEST_CODE_000011",
            "测试",
        );
        assert_eq!(lvl, AnnLevel::Info);
    }

    #[test]
    fn test_dividend_positive() {
        let (lvl, _) = classify_title(
            "2025年度利润分配及高比例现金分红方案公告",
            "TEST_CODE_000012",
            "测试",
        );
        assert_eq!(lvl, AnnLevel::Info);
    }

    #[test]
    fn test_debt_default_emergency() {
        let (lvl, _) = classify_title(
            "关于公司债券发生实质性违约的公告",
            "TEST_CODE_000013",
            "测试",
        );
        assert_eq!(lvl, AnnLevel::Emergency);
    }

    #[test]
    fn test_stake_acquisition_positive() {
        let (lvl, _) = classify_title(
            "关于股东权益变动暨举牌的提示性公告",
            "TEST_CODE_000014",
            "测试",
        );
        assert_eq!(lvl, AnnLevel::Info);
    }

    #[test]
    fn test_executive_resign_important() {
        let (lvl, _) = classify_title(
            "关于公司总经理及财务负责人辞职的公告",
            "TEST_CODE_000015",
            "测试",
        );
        assert_eq!(lvl, AnnLevel::Important);
    }

    #[test]
    fn test_production_halt_important() {
        let (lvl, _) = classify_title(
            "关于子公司发生安全事故暂停生产的公告",
            "TEST_CODE_000016",
            "测试",
        );
        assert_eq!(lvl, AnnLevel::Important);
    }

    #[tokio::test]
    async fn announcement_transport_and_query_date_fail_closed() {
        let client = super::super::unreachable_http_client();
        assert!(fetch_announcements_with_client(&client, Some("bad-date"))
            .await
            .unwrap_err()
            .to_string()
            .contains("日期非法"));
        assert!(fetch_announcements_with_client(&client, Some("2026-07-18"))
            .await
            .is_err());
        assert!(
            fetch_ann_detail_from_url(&client, "TEST_CODE_ARTICLE", ANNOUNCE_URL)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn loopback_transport_executes_list_detail_and_final_assembly() {
        let list = r#"{"data":{"list":[{"art_code":"TEST_CODE_AN_EMERGENCY","title":"关于收到立案调查通知书的公告","notice_date":"2026-07-18","codes":[{"stock_code":"000001","short_name":"TEST_CODE_协议甲"}],"columns":[{"column_name":"风险提示"}]},{"art_code":"TEST_CODE_AN_IMPORTANT","title":"关于收到监管函的公告","notice_date":"2026-07-18","codes":[{"stock_code":"000002","short_name":"TEST_CODE_协议乙"}],"columns":null},{"art_code":"TEST_CODE_AN_INFO","title":"关于回购股份的公告","notice_date":"2026-07-18","codes":[{"stock_code":"000003","short_name":"TEST_CODE_协议丙"}],"columns":null}]}}"#;
        let detail = r#"{"data":{"content":"TEST_CODE_完整公告正文"}}"#;
        let server = super::super::TestHttpServer::new(vec![
            super::super::TestHttpResponse::json(list),
            super::super::TestHttpResponse::json(detail),
            super::super::TestHttpResponse::json(detail),
        ]);
        let base = format!("{}/api/security/ann", server.base_url());
        let announcements = fetch_announcements_from_url(
            &super::super::loopback_http_client(),
            Some("2026-07-18"),
            &base,
        )
        .await
        .expect("complete list and detail transport");
        assert_eq!(announcements.len(), 3);
        assert_eq!(announcements[0].level, AnnLevel::Emergency);
        assert_eq!(announcements[0].content, "TEST_CODE_完整公告正文");
        assert_eq!(announcements[2].level, AnnLevel::Info);
        assert!(announcements[2].content.is_empty());

        let requests = server.finish();
        assert_eq!(requests.len(), 3);
        assert!(requests[0].starts_with("/api/security/ann?page_size=200"));
        assert!(requests[1..]
            .iter()
            .all(|path| path.starts_with("/api/security/ann/detail?art_code=")));
    }
}
