//! 消息→股票 实体关联器（规则消解，不依赖 NLP 模型）。
//!
//! 核心：快讯文本 → 匹配持仓/自选 → 排除噪声关联 → 输出命中列表+置信度。

use std::collections::{HashMap, HashSet};

/// 命中结果
#[derive(Debug, Clone)]
pub struct EntityHit {
    pub code: String,
    pub name: String,
    /// 置信度 0-1
    pub confidence: f64,
    /// 命中原因
    pub reason: String,
}

/// 实体关联器
pub struct EntityLinker {
    /// 代码 → 名称映射
    code_to_name: HashMap<String, String>,
    /// 名称关键词 → 代码列表（简称可能对应多只）
    name_index: HashMap<String, Vec<String>>,
    /// 概念 → 相关代码
    concept_index: HashMap<String, Vec<String>>,
    /// 噪声短语：匹配到但不应作为主体的上下文
    noise_patterns: Vec<String>,
    /// 全名 → 代码 反向索引（自学习缓存，公告返回过的 name+code 自动注册）
    name_to_code: HashMap<String, String>,
}

impl EntityLinker {
    pub fn new() -> Self {
        let mut linker = EntityLinker {
            code_to_name: HashMap::new(),
            name_index: HashMap::new(),
            concept_index: HashMap::new(),
            noise_patterns: vec![
                "供应商".into(),
                "客户".into(),
                "竞争对手".into(),
                "合作方".into(),
                "参股".into(),
                "关联方".into(),
                "行业".into(),
                "板块".into(),
                "概念股".into(),
            ],
            name_to_code: HashMap::new(),
        };
        linker.load_from_env();
        linker
    }

    /// 从 portfolio 加载自选股
    fn load_from_env(&mut self) {
        if let Ok(codes) = crate::portfolio::get_all_codes() {
            for code in codes {
                self.code_to_name
                    .insert(code.clone(), format!("股票{}", code));
            }
        }
    }

    /// 注册持仓股
    pub fn register_position(&mut self, code: &str, name: &str) {
        self.code_to_name.insert(code.to_string(), name.to_string());
        // 全名→代码反向索引
        self.name_to_code.insert(name.to_string(), code.to_string());
        // 全名索引
        self.name_index
            .entry(name.to_string())
            .or_default()
            .push(code.to_string());
        // 简称（去后缀/前缀）
        let short = shorten_name(name);
        if short != name && short.chars().count() >= 2 {
            self.name_index
                .entry(short)
                .or_default()
                .push(code.to_string());
        }
        // 前2字片段（最简匹配，仅短名生效）
        if name.chars().count() >= 3 {
            let prefix: String = name.chars().take(3).collect();
            if !self.name_index.contains_key(&prefix) {
                self.name_index
                    .entry(prefix)
                    .or_default()
                    .push(code.to_string());
            }
        }
    }

    /// 注册名称→代码映射（自学习：公告返回过完整name+code就缓存）
    pub fn register_name_code(&mut self, name: &str, code: &str) {
        if !name.is_empty() && !code.is_empty() && code.len() == 6 {
            self.name_to_code.insert(name.to_string(), code.to_string());
        }
    }

    /// 通过公司全名查找代码（反向索引）
    pub fn lookup_code_by_name(&self, name: &str) -> Option<&str> {
        self.name_to_code.get(name).map(|s| s.as_str())
    }

    /// 替换整个概念索引（L2 实时刷新）
    pub fn replace_concept_index(&mut self, index: HashMap<String, Vec<String>>) {
        self.concept_index = index;
    }

    /// 概念索引的板块数量
    pub fn concept_count(&self) -> usize {
        self.concept_index.len()
    }

    /// 所有已注册的股票代码
    pub fn registered_codes(&self) -> Vec<&str> {
        self.code_to_name.keys().map(|s| s.as_str()).collect()
    }

    /// 读取概念索引（L2 匹配用）
    pub fn concept_index(&self) -> &HashMap<String, Vec<String>> {
        &self.concept_index
    }

    /// 注册概念关联
    pub fn register_concept(&mut self, concept: &str, codes: &[String]) {
        self.concept_index
            .insert(concept.to_string(), codes.to_vec());
    }

    /// 检查股票是否在持仓/自选池中（L1 硬匹配）
    pub fn is_registered(&self, code: &str, _name: &str) -> bool {
        if code.len() == 6 && self.code_to_name.contains_key(code) {
            return true;
        }
        // 备用：通过名称检查（name_index 中的简称可能匹配）
        if !_name.is_empty() {
            let short = shorten_name(_name);
            if self.name_index.contains_key(_name) || self.name_index.contains_key(&short) {
                return true;
            }
            // 前3字片段
            if _name.chars().count() >= 3 {
                let prefix: String = _name.chars().take(3).collect();
                if self.name_index.contains_key(&prefix) {
                    return true;
                }
            }
        }
        false
    }

    /// 核心方法：从快讯文本中提取命中的股票
    pub fn link(&self, text: &str) -> Vec<EntityHit> {
        let mut hits: Vec<EntityHit> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        // 1. 全名精确匹配（高置信度）
        for (code, name) in &self.code_to_name {
            if text.contains(name.as_str()) && seen.insert(code.clone()) {
                let conf = self.check_noise(text, name);
                if conf > 0.0 {
                    hits.push(EntityHit {
                        code: code.clone(),
                        name: name.clone(),
                        confidence: conf,
                        reason: "全名匹配".into(),
                    });
                }
            }
        }

        // 2. 简称/片段匹配（中置信度），按长度降序避免短串误匹配
        let mut keys: Vec<&String> = self.name_index.keys().collect();
        keys.sort_by(|a, b| b.chars().count().cmp(&a.chars().count()));
        for short in keys {
            if short.chars().count() < 2 {
                continue;
            }
            if text.contains(short.as_str()) {
                for code in &self.name_index[short] {
                    if seen.insert(code.clone()) {
                        let name = self.code_to_name.get(code).cloned().unwrap_or_default();
                        let conf = self.check_noise(text, &name) * 0.85;
                        if conf > 0.0 {
                            hits.push(EntityHit {
                                code: code.clone(),
                                name,
                                confidence: conf,
                                reason: format!("片段'{}'匹配", short),
                            });
                        }
                        break; // 一个片段只匹配一次
                    }
                }
            }
        }

        // 3. 概念关联（低置信度，仅做参考）
        for (concept, codes) in &self.concept_index {
            if text.contains(concept.as_str()) {
                for code in codes {
                    if seen.insert(code.clone()) {
                        let name = self.code_to_name.get(code).cloned().unwrap_or_default();
                        hits.push(EntityHit {
                            code: code.clone(),
                            name,
                            confidence: 0.4,
                            reason: format!("概念'{}'关联", concept),
                        });
                    }
                }
            }
        }

        hits
    }

    /// 检查噪声：如果股票名出现在噪声短语的语境中，降低置信度
    fn check_noise(&self, text: &str, name: &str) -> f64 {
        // 找名字在文本中的位置
        let pos = match text.find(name) {
            Some(p) => p,
            None => return 1.0,
        };

        // 检查名字前后是否有噪声短语
        let raw_start = pos.saturating_sub(10);
        let raw_end = (pos + name.len() + 15).min(text.len());
        // 对齐到 UTF-8 字符边界（中文等多字节字符会跨字节）
        let context_start = text.floor_char_boundary(raw_start);
        let context_end = text.ceil_char_boundary(raw_end);
        let context = &text[context_start..context_end];

        for noise in &self.noise_patterns {
            if context.contains(noise.as_str()) {
                // 名字出现在"供应商"等词之前 → 这不是主体
                let noise_pos = context.find(noise.as_str()).unwrap();
                if pos - context_start > noise_pos {
                    return 0.0; // 名字在噪声词之后 → 排除
                }
                return 0.5; // 名字在噪声词之前 → 降级
            }
        }
        1.0
    }

    /// 已注册股票数
    pub fn stock_count(&self) -> usize {
        self.code_to_name.len()
    }
}

impl Default for EntityLinker {
    fn default() -> Self {
        Self::new()
    }
}

/// 提取简称（去后缀/前缀）
fn shorten_name(name: &str) -> String {
    let suffixes = [
        "科技", "集团", "股份", "控股", "实业", "产业", "电子", "医药", "电气", "汽车", "通信",
        "传媒",
    ];
    let prefixes = [
        "贵州", "云南", "四川", "山东", "江苏", "浙江", "广东", "福建", "深圳", "上海", "北京",
    ];
    let mut s = name.to_string();
    for sfx in &suffixes {
        if s.ends_with(sfx) && s.len() > sfx.len() + 2 {
            s = s[..s.len() - sfx.len()].into();
            break;
        }
    }
    for pfx in &prefixes {
        if s.starts_with(pfx) && s.len() > pfx.len() + 1 {
            s = s[pfx.len()..].into();
            break;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        let mut linker = EntityLinker::new();
        linker.register_position("000547", "航天发展");
        let hits = linker.link("航天发展今日获大额订单");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].code, "000547");
        assert!(hits[0].confidence > 0.9);
    }

    #[test]
    fn test_supplier_noise_excluded() {
        let mut linker = EntityLinker::new();
        linker.register_position("300750", "宁德时代");
        // "宁德时代供应商X发生火灾" → 主体是X，不是宁德时代
        let hits = linker.link("宁德时代供应商某公司发生火灾");
        // 宁德时代在噪声短语"供应商"之前被匹配 → 降级但可能仍有 0.5
        assert!(hits.is_empty() || hits[0].confidence < 0.6);
    }

    #[test]
    fn test_short_name_match() {
        let mut linker = EntityLinker::new();
        linker.register_position("000547", "航天发展工业集团");
        // 简称"航天发展"应在文本中匹配
        let hits = linker.link("航天发展发布公告");
        assert!(!hits.is_empty());
    }

    #[test]
    fn test_concept_match_low_confidence() {
        let mut linker = EntityLinker::new();
        linker.register_concept("低空经济", &["000099".into(), "002085".into()]);
        linker.register_position("000099", "中信海直");
        linker.register_position("002085", "万丰奥威");
        let hits = linker.link("低空经济新政即将发布");
        assert!(hits.len() >= 2);
        assert!(hits.iter().all(|h| h.confidence <= 0.5));
    }

    #[test]
    fn test_no_match() {
        let linker = EntityLinker::new();
        let hits = linker.link("今日大盘走势平稳");
        assert!(hits.is_empty());
    }

    #[test]
    fn test_shorten_name() {
        assert_eq!(shorten_name("航天发展工业集团"), "航天发展工业");
        assert_eq!(shorten_name("贵州茅台"), "茅台");
    }
}
