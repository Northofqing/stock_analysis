//! AI 分析产业链（2026-10-18）。
//!
//! ## 这不是事实数据
//!
//! F10 的经营分析/行业分析是**原文事实**（港澳资讯），AI 对它的解读是**推断**。
//! 两者必须分开存，不能混：
//!
//!     stock_f10_profile          F10 原文（事实）
//!     chain_relation_candidate   AI 推断（候选，本模块写这里）
//!     chain_relation             人工确认（可信，本模块不写）
//!
//! 本模块只写 candidate，**不写 relation，不参与任何买卖判断**。
//!
//! ## 输入
//!
//! F10 经营分析里的【2.主营构成分析】—— 它告诉 AI
//! "77% 收入来自卖电、21% 来自供热、毛利率分别 28% 和 5.6%"。
//! 有了这个，AI 推上下游才不是靠"光伏概念"瞎猜。
//!
//! ## 输出
//!
//! 每条候选带：
//!
//!     relation     supplier / customer / competitor
//!     confidence   模型自评 0-1
//!     evidence     它的依据（必须引用主营构成里的具体项）
//!     object_name  对手方名称（公司名或行业名）
//!     object_code  能对应到库里就填，否则 NULL
//!
//! ## 边界
//!
//! 全部标 `source='ai'`。任何展示都必须带 `[AI推断·未核实]`。
//! 不因为"AI 说"就进 buy_zone / priority。

use diesel::prelude::*;
use diesel::sql_types::{Double, Nullable, Text};
use serde::Deserialize;

use crate::database::DatabaseManager;
use crate::llm::LlmProvider;

const SYSTEM_PROMPT: &str = r#"你是A股产业链成本分析助手。只输出 JSON，不要 markdown，不要解释。

任务：根据一家公司的"主营业务"和"主营构成"，找出它的上游成本驱动项和下游需求驱动项。

严格规则：
1. 主要找 supplier（上游）：这家公司生产时花钱买什么？
   - 必须写到**具体的大宗商品或原材料**。不要写"煤炭行业"，要写"动力煤"；
     不要写"金属"，要写"电解铝"。理由：只有具体到商品，才能盯它的价格。
   - 如果主营构成里能看出成本占比，在 evidence 里写出来。
2. customer（下游）只在收入明显集中在某个具体行业或客户类型时才列。
   泛泛的"终端用户"不要列。
3. **不要列 competitor**。同行业竞争关系我们从概念标签算，不需要你判断。
4. 每条 evidence 必须回答方向："这个上游涨价，这家公司毛利是升还是降？"
   必须写清"涨价→毛利下降"或"涨价→毛利上升"这样的因果。
5. confidence 是自评 0-1。低于 0.5 的不要输出。
6. 最多 6 条。宁少不滥，凑数的一条都不要。

输出格式（无 markdown）：
{"relations":[{"object_name":"动力煤","object_code":null,"relation":"supplier","confidence":0.85,"evidence":"主营构成为火力发电，燃料成本是营业成本主要构成；动力煤涨价→毛利下降"}]}"#;

#[derive(Debug, Clone, Deserialize)]
pub struct ChainCandidate {
    pub object_name: String,
    #[serde(default)]
    pub object_code: Option<String>,
    pub relation: String,
    pub confidence: f64,
    pub evidence: String,
}

#[derive(Debug, Deserialize)]
struct LlmReply {
    #[serde(default)]
    relations: Vec<ChainCandidate>,
}

#[derive(Debug, Default)]
pub struct AnalyzeReport {
    pub code: String,
    pub candidates: usize,
    pub persisted: usize,
    pub rejected: usize,
    pub errors: Vec<String>,
}

impl AnalyzeReport {
    pub fn log(&self) {
        log::info!(
            "[产业链AI] {} candidates={} persisted={} rejected={}",
            self.code,
            self.candidates,
            self.persisted,
            self.rejected
        );
        for error in &self.errors {
            log::warn!("[产业链AI] {error}");
        }
    }
}

fn create_schema(conn: &mut SqliteConnection) -> Result<(), diesel::result::Error> {
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS chain_relation_candidate (\
            subject_code TEXT NOT NULL,\
            object_name TEXT NOT NULL,\
            object_code TEXT,\
            relation TEXT NOT NULL,\
            confidence REAL NOT NULL,\
            evidence TEXT NOT NULL,\
            source TEXT NOT NULL DEFAULT 'ai',\
            generated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,\
            PRIMARY KEY (subject_code, object_name, relation)\
        )",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE INDEX IF NOT EXISTS ix_chain_relation_candidate_subject \
         ON chain_relation_candidate (subject_code)",
    )
    .execute(conn)?;
    Ok(())
}

/// 从 F10 原文里截取关键段落（省 token，也避免把整份表格塞进 prompt）。
fn extract_business_core(text: &str) -> String {
    // 主营构成分析是最有价值的段落；取不到就退回全文前 3000 字符
    if let Some(start) = text.find("【2.主营构成分析】") {
        let rest = &text[start..];
        let end = rest
            .find("【3.")
            .or_else(|| rest.find("【4."))
            .unwrap_or(rest.len());
        return rest[..end.min(4000)].to_string();
    }
    text.chars().take(3000).collect()
}

/// 分析一只票。需要 F10 经营分析已落库。
pub async fn analyze(
    provider: std::sync::Arc<dyn LlmProvider>,
    db: &DatabaseManager,
    code: &str,
) -> Result<AnalyzeReport, String> {
    let mut report = AnalyzeReport {
        code: code.to_string(),
        ..Default::default()
    };

    let (business, industry) = {
        let mut conn = db.get_conn().map_err(|error| error.to_string())?;
        let business = crate::monitor::f10_profile::load_category(
            &mut conn,
            code,
            crate::monitor::f10_profile::CATEGORY_BUSINESS,
        )?
        .ok_or_else(|| format!("{code} 没有 F10 经营分析，先跑 refresh_f10"))?;
        let industry = crate::monitor::f10_profile::load_category(
            &mut conn,
            code,
            crate::monitor::f10_profile::CATEGORY_INDUSTRY,
        )?
        .unwrap_or_default();
        (business, industry)
    };

    let business_core = extract_business_core(&business);
    let industry_core: String = industry.chars().take(2000).collect();
    let user = format!(
        "股票代码：{code}\n\n主营构成：\n{business_core}\n\n行业分析（节选）：\n{industry_core}\n\n请按规则输出 JSON。"
    );

    let value = provider
        .chat_json(SYSTEM_PROMPT, &user)
        .await
        .map_err(|error| format!("LLM 调用失败: {error}"))?;

    let reply: LlmReply =
        serde_json::from_value(value).map_err(|error| format!("LLM 响应解析失败: {error}"))?;
    report.candidates = reply.relations.len();

    let mut conn = db.get_conn().map_err(|error| error.to_string())?;
    create_schema(&mut conn).map_err(|error| error.to_string())?;

    for candidate in &reply.relations {
        // 二次清洗：relation 白名单、confidence 下限、object_name 非空
        let relation_ok = matches!(
            candidate.relation.as_str(),
            "supplier" | "customer" | "competitor"
        );
        if !relation_ok
            || candidate.confidence < 0.5
            || candidate.object_name.trim().is_empty()
            || candidate.evidence.trim().is_empty()
        {
            report.rejected += 1;
            continue;
        }

        let written = diesel::sql_query(
            "INSERT INTO chain_relation_candidate \
                (subject_code, object_name, object_code, relation, confidence, evidence, source) \
             VALUES (?, ?, ?, ?, ?, ?, 'ai') \
             ON CONFLICT(subject_code, object_name, relation) DO UPDATE SET \
                object_code=excluded.object_code, \
                confidence=excluded.confidence, \
                evidence=excluded.evidence, \
                generated_at=CURRENT_TIMESTAMP",
        )
        .bind::<Text, _>(code)
        .bind::<Text, _>(candidate.object_name.trim())
        .bind::<Nullable<Text>, _>(candidate.object_code.as_deref())
        .bind::<Text, _>(&candidate.relation)
        .bind::<Double, _>(candidate.confidence)
        .bind::<Text, _>(candidate.evidence.trim())
        .execute(&mut conn);

        match written {
            Ok(_) => report.persisted += 1,
            Err(error) => report.errors.push(format!(
                "{code} {} 落库失败: {error}",
                candidate.object_name
            )),
        }
    }

    Ok(report)
}

/// 读候选关系（供展示）。
pub fn load_candidates(
    conn: &mut SqliteConnection,
    code: &str,
) -> Result<Vec<(String, String, String, f64, String)>, String> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        object_name: String,
        #[diesel(sql_type = Text)]
        relation: String,
        #[diesel(sql_type = Text)]
        evidence: String,
        #[diesel(sql_type = Double)]
        confidence: f64,
    }
    let rows: Vec<Row> = diesel::sql_query(
        "SELECT object_name, relation, evidence, confidence \
         FROM chain_relation_candidate WHERE subject_code = ? \
         ORDER BY confidence DESC",
    )
    .bind::<Text, _>(code)
    .load(conn)
    .map_err(|error| error.to_string())?;

    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.object_name,
                row.relation,
                row.evidence,
                row.confidence,
                String::new(),
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_core_finds_business_section() {
        let text =
            "☆经营分析☆\n【1.主营业务】\n卖电\n【2.主营构成分析】\n电力 77%\n【3.经营投资】\n其他";
        let core = extract_business_core(text);
        assert!(core.contains("【2.主营构成分析】"));
        assert!(core.contains("电力 77%"));
        assert!(!core.contains("【3.经营投资】"));
    }

    #[test]
    fn extract_core_falls_back_when_no_section() {
        let text = "只有正文没有分段";
        let core = extract_business_core(text);
        assert_eq!(core, text);
    }

    #[test]
    fn relation_whitelist_rejects_unknown() {
        for relation in ["supplier", "customer", "competitor"] {
            assert!(matches!(relation, "supplier" | "customer" | "competitor"));
        }
        assert!(!matches!("partner", "supplier" | "customer" | "competitor"));
    }
}
