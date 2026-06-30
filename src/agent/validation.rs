use crate::agent::context::ContextManager;

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("数据不一致 (Data inconsistency): {0}")]
    Inconsistency(String),
    #[error("缺失必填字段 (Missing field): {0}")]
    MissingField(String),
}

pub trait Validator: Send + Sync {
    fn name(&self) -> &str;
    fn validate(&self, context: &ContextManager) -> Result<(), ValidationError>;
}

pub struct GrossMarginValidator;
impl Validator for GrossMarginValidator {
    fn name(&self) -> &str { "GrossMarginValidator" }
    fn validate(&self, context: &ContextManager) -> Result<(), ValidationError> { 
        if let Some(financials) = context.get_fact("fetch_financials") {
            if let Some(gm) = financials.get("gross_margin").and_then(|v| v.as_f64()) {
                if gm < -500.0 {
                    return Err(ValidationError::Inconsistency(format!("发现异常毛利率 ({})，可能为数据源错乱或报表未更新。请寻找其他数据源代替或在报告中警示。", gm)));
                }
            }
        }
        Ok(()) 
    }
}

pub struct ConsensusDeviationValidator;
impl Validator for ConsensusDeviationValidator {
    fn name(&self) -> &str { "ConsensusDeviationValidator" }
    fn validate(&self, context: &ContextManager) -> Result<(), ValidationError> { 
        if let Some(research) = context.get_fact("fetch_research") {
            if let Some(arr) = research.as_array() {
                if arr.is_empty() {
                    return Err(ValidationError::MissingField("未能获取到任何机构研报视图。若代码无误可能为冷门股，请在报告特别标明由于缺乏机构覆盖流动性风险可能偏高。".to_string()));
                }
            }
        }
        Ok(()) 
    }
}

/// Stub: no-op. Awaiting fact-source wiring (concept data + dividend schedule).
/// 修复 (2026-06-30 codex review): 不再加入 new_with_defaults(), 避免 AGENTS §2.8
/// 假实现禁令风险. 外部可通过 add_validator(ConceptLinkageValidator) opt-in.
pub struct ConceptLinkageValidator;
impl Validator for ConceptLinkageValidator {
    fn name(&self) -> &str { "ConceptLinkageValidator" }
    fn validate(&self, _context: &ContextManager) -> Result<(), ValidationError> { Ok(()) }
}

/// Stub: no-op. Awaiting fact-source wiring (dividend yield/schedule).
/// 修复 (2026-06-30 codex review): 不再加入 new_with_defaults(), 避免 AGENTS §2.8
/// 假实现禁令风险. 外部可通过 add_validator(DividendTaxValidator) opt-in.
pub struct DividendTaxValidator;
impl Validator for DividendTaxValidator {
    fn name(&self) -> &str { "DividendTaxValidator" }
    fn validate(&self, _context: &ContextManager) -> Result<(), ValidationError> { Ok(()) }
}

pub struct ValidationEngine {
    validators: Vec<Box<dyn Validator>>,
}

impl ValidationEngine {
    pub fn new() -> Self {
        Self { validators: Vec::new() }
    }

    /// 默认只装两个真实 validator. 修复 (2026-06-30 codex review): 之前装的
    /// ConceptLinkageValidator + DividendTaxValidator 都是 Ok(()) 假实现,
    /// 占用 validation slot 制造 "已校验" 假象, 违反 AGENTS §2.8.
    pub fn new_with_defaults() -> Self {
        let mut engine = Self::new();
        engine.add_validator(GrossMarginValidator);
        engine.add_validator(ConsensusDeviationValidator);
        engine
    }

    pub fn add_validator(&mut self, validator: impl Validator + 'static) {
        self.validators.push(Box::new(validator));
    }

    pub fn validator_count(&self) -> usize {
        self.validators.len()
    }

    pub fn run_all(&self, context: &ContextManager) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        for validator in &self.validators {
            if let Err(e) = validator.validate(context) { errors.push(e); }
        }
        errors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_with_defaults_contains_only_real_validators() {
        // 修复 (2026-06-30 codex review): new_with_defaults 必须只含真实 validator.
        // ConceptLinkage / DividendTax 是 no-op stub, 只能通过 add_validator opt-in.
        let engine = ValidationEngine::new_with_defaults();
        assert_eq!(
            engine.validator_count(), 2,
            "new_with_defaults must only register the 2 real validators (GrossMargin + ConsensusDeviation), not the 2 no-op stubs"
        );
    }

    #[test]
    fn test_new_with_defaults_validator_names() {
        let engine = ValidationEngine::new_with_defaults();
        // 通过 run_all 行为反推: 空 context 下, 2 个真实 validator 都应返回 Ok
        // (因为 financials/research 都不在 context 里, 走 default Ok 路径)
        let ctx = crate::agent::context::ContextManager::new();
        let errors = engine.run_all(&ctx);
        assert!(errors.is_empty(), "default validators must pass on empty context");
    }
}
