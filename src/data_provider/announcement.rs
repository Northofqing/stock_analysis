//! A股公告抓取（东方财富公告API）。
//!
//! 策略：标题即风控——含关键词直接告警，不等正文。
//! 非高危公告用东方财富编辑摘要，不做 PDF 解析。
//! 单次超200条自动熔断，仅扫描标题。

use anyhow::Result;
use log::{info, warn};
use serde::Deserialize;

const ANNOUNCE_URL: &str = "https://np-anotice-stock.eastmoney.com/api/security/ann";
const MAX_PER_FETCH: usize = 200;

#[derive(Debug, Deserialize)]
struct AnnResponse {
    data: Option<AnnData>,
}

#[derive(Debug, Deserialize)]
struct AnnData {
    list: Option<Vec<AnnItem>>,
}

#[derive(Debug, Deserialize, Clone)]
struct AnnItem {
    art_code: Option<String>,
    title: Option<String>,
    notice_date: Option<String>,
    /// 关联股票列表（codes[0] 通常是主股票）
    codes: Option<Vec<AnnCode>>,
    /// 公告分类（columns[0].column_name 如"召开股东大会通知"）
    columns: Option<Vec<AnnColumn>>,
}

#[derive(Debug, Deserialize, Clone)]
struct AnnCode {
    stock_code: Option<String>,
    short_name: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct AnnColumn {
    column_name: Option<String>,
}

/// 告警级别
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AnnLevel { Emergency, Important, Info, Skip }

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
}

// ── 标题关键词哨兵 ──

const EMERGENCY_KEYWORDS: &[&str] = &[
    "立案调查", "终止上市", "ST风险", "无法表示意见", "强制退市",
    "暂停上市", "破产", "清算", "实际控制人失联",
];

const IMPORTANT_KEYWORDS: &[&str] = &[
    "减持", "问询函", "业绩预亏", "商誉减值", "重大诉讼",
    "质押", "冻结", "监管函", "警示函", "责令改正",
];

const POSITIVE_KEYWORDS: &[&str] = &[
    "回购", "增持", "中标", "重组", "业绩预增", "高送转",
    "重大合同", "战略合作", "获得批文",
];

fn classify_title(title: &str, _code: &str, _name: &str) -> (AnnLevel, String) {
    for kw in EMERGENCY_KEYWORDS {
        if title.contains(kw) {
            return (AnnLevel::Emergency, format!("标题含'{}'，直接告警", kw));
        }
    }
    for kw in IMPORTANT_KEYWORDS {
        if title.contains(kw) {
            // 减持需要判断比例
            if kw == &"减持" {
                if let Some(pct) = extract_reduction_pct(title) {
                    if pct < 1.0 {
                        continue; // <1% 不算重要
                    }
                }
            }
            return (AnnLevel::Important, format!("标题含'{}'", kw));
        }
    }
    for kw in POSITIVE_KEYWORDS {
        if title.contains(kw) {
            return (AnnLevel::Info, format!("利好: '{}'", kw));
        }
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

// ── API 拉取 ──

pub fn fetch_announcements(date: Option<&str>) -> Result<Vec<Announcement>> {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let date_str = date.unwrap_or(&today);

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let url = format!(
        "{}?page_size={}&page_index=1&ann_type=SHA,SZA&start_date={}&end_date={}",
        ANNOUNCE_URL, MAX_PER_FETCH, date_str, date_str
    );

    let resp: AnnResponse = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .header("Referer", "https://data.eastmoney.com/")
        .send()?
        .json()?;

    let list = resp.data.and_then(|d| d.list).unwrap_or_default();
    info!("[公告] {} 获取 {} 条", date_str, list.len());

    // 熔断：超 200 条仅标题扫描
    if list.len() >= MAX_PER_FETCH {
        warn!("[公告] 单日 {} 条，触发熔断，仅标题扫描", list.len());
    }

    let mut results = Vec::new();
    for item in list {
        let title = item.title.as_deref().unwrap_or("");

        // 从 codes[0] 提取股票信息
        let code = item.codes.as_ref()
            .and_then(|c| c.first())
            .and_then(|c| c.stock_code.as_deref())
            .unwrap_or("");
        let name = item.codes.as_ref()
            .and_then(|c| c.first())
            .and_then(|c| c.short_name.as_deref())
            .unwrap_or("");

        let (level, reason) = classify_title(title, code, name);
        if matches!(level, AnnLevel::Skip) {
            continue; // 非高危非利好，跳过
        }

        // 公告分类描述（如"召开股东大会通知"），用作摘要回退
        let column_desc = item.columns.as_ref()
            .and_then(|c| c.first())
            .and_then(|c| c.column_name.as_deref())
            .unwrap_or("");

        // 高危公告尝试拉取正文
        let content = if matches!(level, AnnLevel::Emergency | AnnLevel::Important) {
            let art_code = item.art_code.as_deref().unwrap_or("");
            if !art_code.is_empty() {
                fetch_ann_detail(art_code).unwrap_or_default()
            } else { String::new() }
        } else { String::new() };

        results.push(Announcement {
            code: code.to_string(),
            name: name.to_string(),
            title: title.to_string(),
            date: item.notice_date.as_deref().unwrap_or(date_str).to_string(),
            summary: column_desc.to_string(),
            content,
            level,
            reason,
        });
    }

    info!("[公告] 过滤后 {} 条需告警", results.len());
    Ok(results)
}

/// 获取公告正文（东方财富公告详情API）
fn fetch_ann_detail(art_code: &str) -> Result<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let url = format!(
        "https://np-anotice-stock.eastmoney.com/api/security/ann/detail?art_code={}",
        art_code
    );
    #[derive(Deserialize)]
    struct DetailResp { data: Option<DetailData> }
    #[derive(Deserialize)]
    struct DetailData { content: Option<String> }

    let resp: DetailResp = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .header("Referer", "https://data.eastmoney.com/")
        .send()?
        .json()?;

    Ok(resp.data.and_then(|d| d.content).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_emergency() {
        let (lvl, reason) = classify_title("关于收到中国证监会立案调查通知书的公告", "000001", "测试");
        assert_eq!(lvl, AnnLevel::Emergency);
        assert!(reason.contains("立案调查"));
    }

    #[test]
    fn test_classify_important() {
        let (lvl, _) = classify_title("关于持股5%以上股东减持股份超过1%的公告", "000002", "测试");
        assert_eq!(lvl, AnnLevel::Important);
    }

    #[test]
    fn test_classify_positive() {
        let (lvl, _) = classify_title("关于回购公司股份方案的公告", "000003", "测试");
        assert_eq!(lvl, AnnLevel::Info);
    }

    #[test]
    fn test_classify_normal_skip() {
        let (lvl, _) = classify_title("2025年第三次临时股东大会决议公告", "000004", "测试");
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
        let (lvl, _) = classify_title("关于股东减持股份不超过0.5%的提示性公告", "000005", "测试");
        assert_eq!(lvl, AnnLevel::Skip); // <1% 不告警
    }
}
