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
    fn name(&self) -> &str {
        "GrossMarginValidator"
    }
    fn validate(&self, context: &ContextManager) -> Result<(), ValidationError> {
        if let Some(financials) = context.get_fact("fetch_financials") {
            let gm = financials
                .get("gross_margin")
                .and_then(|value| value.as_f64())
                .ok_or_else(|| ValidationError::MissingField("gross_margin".to_string()))?;
            if gm < -500.0 {
                return Err(ValidationError::Inconsistency(format!("发现异常毛利率 ({})，可能为数据源错乱或报表未更新。请寻找其他数据源代替或在报告中警示。", gm)));
            }
        }
        Ok(())
    }
}

pub struct ConsensusDeviationValidator;
impl Validator for ConsensusDeviationValidator {
    fn name(&self) -> &str {
        "ConsensusDeviationValidator"
    }
    fn validate(&self, context: &ContextManager) -> Result<(), ValidationError> {
        if let Some(research) = context.get_fact("fetch_research") {
            let reports = research
                .as_array()
                .or_else(|| research.get("reports").and_then(|value| value.as_array()))
                .ok_or_else(|| {
                    ValidationError::MissingField("fetch_research.reports".to_string())
                })?;
            if reports.is_empty() {
                return Err(ValidationError::MissingField("未能获取到任何机构研报视图。若代码无误可能为冷门股，请在报告特别标明由于缺乏机构覆盖流动性风险可能偏高。".to_string()));
            }
        }
        Ok(())
    }
}

pub struct ValidationEngine {
    validators: Vec<Box<dyn Validator>>,
}

impl Default for ValidationEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ValidationEngine {
    pub fn new() -> Self {
        Self {
            validators: Vec::new(),
        }
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

    pub fn validator_names(&self) -> Vec<&str> {
        self.validators
            .iter()
            .map(|validator| validator.name())
            .collect()
    }

    pub fn run_all(&self, context: &ContextManager) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        for validator in &self.validators {
            if let Err(e) = validator.validate(context) {
                errors.push(e);
            }
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
        assert_eq!(
            engine.validator_names(),
            vec!["GrossMarginValidator", "ConsensusDeviationValidator"]
        );
        let ctx = crate::agent::context::ContextManager::new();
        let errors = engine.run_all(&ctx);
        assert!(errors.is_empty(), "未调用的工具不应被伪判为已失败");
    }

    #[test]
    fn test_financial_fact_requires_gross_margin() {
        let mut ctx = ContextManager::new();
        ctx.insert_fact("fetch_financials", serde_json::json!({"eps": 1.0}));
        let errors = ValidationEngine::new_with_defaults().run_all(&ctx);
        assert!(matches!(
            errors.as_slice(),
            [ValidationError::MissingField(field)] if field == "gross_margin"
        ));
    }

    #[test]
    fn test_research_object_requires_nonempty_reports() {
        let mut ctx = ContextManager::new();
        ctx.insert_fact("fetch_research", serde_json::json!({"reports": []}));
        let errors = ValidationEngine::new_with_defaults().run_all(&ctx);
        assert!(matches!(
            errors.as_slice(),
            [ValidationError::MissingField(_)]
        ));
    }
}
