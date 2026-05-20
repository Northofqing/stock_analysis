use crate::agent::context::ContextManager;
use serde_json::Value;

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

pub struct ConceptLinkageValidator;
impl Validator for ConceptLinkageValidator {
    fn name(&self) -> &str { "ConceptLinkageValidator" }
    fn validate(&self, context: &ContextManager) -> Result<(), ValidationError> { Ok(()) }
}

pub struct DividendTaxValidator;
impl Validator for DividendTaxValidator {
    fn name(&self) -> &str { "DividendTaxValidator" }
    fn validate(&self, context: &ContextManager) -> Result<(), ValidationError> { Ok(()) }
}

pub struct ValidationEngine {
    validators: Vec<Box<dyn Validator>>,
}

impl ValidationEngine {
    pub fn new() -> Self {
        Self { validators: Vec::new() }
    }

    pub fn new_with_defaults() -> Self {
        let mut engine = Self::new();
        engine.add_validator(GrossMarginValidator);
        engine.add_validator(ConsensusDeviationValidator);
        engine.add_validator(ConceptLinkageValidator);
        engine.add_validator(DividendTaxValidator);
        engine
    }

    pub fn add_validator(&mut self, validator: impl Validator + 'static) {
        self.validators.push(Box::new(validator));
    }

    pub fn run_all(&self, context: &ContextManager) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        for validator in &self.validators {
            if let Err(e) = validator.validate(context) { errors.push(e); }
        }
        errors
    }
}
