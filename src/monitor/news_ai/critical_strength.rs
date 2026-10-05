//! One designated existing NewsAI call; declared model importance is not source certainty.
use super::*;

pub(super) const VERSION: &str = "news_ai_n01_strength_v1";
pub(super) const SYSTEM: &str = r#"你是A股新闻影响分析助手。仅对输入中预先指定的target_code分析，不得更换证券。
只返回一个JSON对象，且恰好五个字段：impact、confidence、uncertainty、core_logic、strength。
impact仅可为major_negative/negative/neutral/positive/major_positive。
confidence和strength均为0到100的整数。confidence表示判断信心；strength表示该新闻对指定证券的重要程度，两者独立，均不是来源真实性或交易授权。
uncertainty和core_logic必须为非空字符串。只使用输入事实；没有证据不能补造。不得输出Markdown或额外字段。"#;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewsBaseIdentity {
    provider: String,
    item_id: String,
    text_revision: String,
}
impl NewsBaseIdentity {
    pub fn from_fact(fact: &AdmittedNewsFact) -> Result<Self, NewsAiError> {
        // Reuse the exact V3 text/Option encoding, while excluding target/profile/batch.
        let profile = NewsAiAnalysisProfile::for_configured_model("base-only", "base-only")?;
        let identity = NewsAiIdentityV3::from_fact(fact, &profile)?;
        Ok(Self { provider: identity.provider, item_id: identity.item_id,
            text_revision: identity.content_revision })
    }
    pub fn digest(&self) -> String {
        let mut hash = Sha256::new();
        hash.update(b"BR172_NEWS_BASE_V1\0");
        hash_field(&mut hash, &self.provider);
        hash_field(&mut hash, &self.item_id);
        hash_field(&mut hash, &self.text_revision);
        format!("{:x}", hash.finalize())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Output {
    impact: NewsImpact,
    confidence: u8,
    uncertainty: String,
    core_logic: String,
    strength: u8,
}
fn parse_output(raw: &str) -> Result<Output, NewsAiError> {
    if raw.len() > 128 * 1024 { return Err(NewsAiError::InvalidModelSchema("N01 response exceeds 128KiB".into())); }
    let value: Output = serde_json::from_str(raw)
        .map_err(|e| NewsAiError::InvalidModelSchema(e.to_string()))?;
    if value.confidence > 100 || value.strength > 100
        || value.uncertainty.trim().is_empty() || value.core_logic.trim().is_empty() {
        return Err(NewsAiError::InvalidModelSchema("invalid N01 output bounds".into()));
    }
    Ok(value)
}

// No public constructor or deserializer: only the real receipt-bearing call creates this.
pub struct CriticalModelResult {
    pub(crate) request: NewsAiRequest,
    pub(crate) assessment: NewsAiAssessment,
    pub(crate) response: String,
    pub(crate) strength: u8,
}

/// Persistable declaration; this alone is never a delivery capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CriticalNewsEvidence {
    identity: NewsAiIdentityV3,
    news_base: NewsBaseIdentity,
    fact_snapshot: String,
    normalized_prompt: String,
    response: String,
    receipt: ModelCallReceipt,
    input_evidence_sha256: String,
    assessment_audit_sha256: String,
    strength: u8,
}
impl CriticalNewsEvidence {
    pub fn validate(&self) -> Result<(), NewsAiError> {
        if self.normalized_prompt.len() > 1024 * 1024 || self.fact_snapshot.len() > NEWS_FACT_RECOVERY_SNAPSHOT_MAX_BYTES {
            return Err(NewsAiError::AnalysisAuditFailed("N01 input evidence extent exceeded".into()));
        }
        let fact = AdmittedNewsFact::from_recovery_snapshot(self.fact_snapshot.as_bytes())?;
        self.identity.validate_fact(&fact)?;
        if !self.identity.profile.is_critical() || self.news_base != NewsBaseIdentity::from_fact(&fact)? {
            return Err(NewsAiError::AnalysisAuditFailed("N01 identity/base changed".into()));
        }
        validate_sha256("N01 assessment audit", &self.assessment_audit_sha256)?;
        validate_sha256("N01 input evidence", &self.input_evidence_sha256)?;
        let output = parse_output(&self.response)?;
        let prompt: serde_json::Value = serde_json::from_str(&self.normalized_prompt)
            .map_err(|e| NewsAiError::AnalysisAuditFailed(e.to_string()))?;
        if output.strength != self.strength
            || self.receipt.system_sha256 != sha256_hex(SYSTEM.as_bytes())
            || self.receipt.user_sha256 != sha256_hex(self.normalized_prompt.as_bytes())
            || self.receipt.response_sha256 != sha256_hex(self.response.as_bytes())
            || self.receipt.provider != self.identity.profile.model_provider
            || self.receipt.model != self.identity.profile.configured_model
            || self.receipt.upstream_response_id.trim().is_empty()
            || self.receipt.completed_at < self.receipt.started_at
            || self.receipt.started_at < fact.observed_at()
            || prompt.get("target_code").and_then(|v| v.as_str()) != Some(fact.target_code())
            || prompt.get("analysis_version").and_then(|v| v.as_str()) != Some(VERSION)
            || prompt.get("news") != Some(&serde_json::json!({
                "provider": provider_tag(fact.provider()), "source": fact.source(),
                "batch_id": fact.source_batch_id(), "item_id": fact.item_id(),
                "published_at": fact.published_at().to_rfc3339(), "title": fact.title(),
                "summary": fact.summary(), "content": fact.content(),
            })) {
            return Err(NewsAiError::AnalysisAuditFailed("N01 call/content/target binding changed".into()));
        }
        // GlobalNews only. Sina/macros never acquire this purpose.
        if !matches!(&fact.record, AdmittedNewsSourceRecord::Global(_)) {
            return Err(NewsAiError::NewsEvidenceMismatch("N01 requires GlobalNews".into()));
        }
        if canonical_critical_target(&fact).as_deref() != Some(fact.target_code()) {
            return Err(NewsAiError::NewsEvidenceMismatch("N01 target is not the source-fixed canonical target".into()));
        }
        Ok(())
    }
    pub fn canonical(&self) -> Result<Vec<u8>, NewsAiError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|e| NewsAiError::AnalysisAuditFailed(e.to_string()))
    }
    pub fn digest(&self) -> Result<String, NewsAiError> {
        let mut h = Sha256::new(); h.update(b"BR244_N01_SCORE_AUDIT_V1\0");
        h.update(self.canonical()?); Ok(format!("{:x}", h.finalize()))
    }
    pub(crate) fn fact(&self) -> Result<AdmittedNewsFact, NewsAiError> {
        self.validate()?; AdmittedNewsFact::from_recovery_snapshot(self.fact_snapshot.as_bytes())
    }
    pub fn assessment_id(&self) -> String { self.identity.digest() }
    pub fn strength(&self) -> u8 { self.strength }
    pub fn event_id(&self) -> Result<String, NewsAiError> {
        let fact = self.fact()?;
        // Exact old BR166 source-domain key, independent of profile/revision/target.
        let mut h = Sha256::new(); h.update(b"BR166_GLOBAL_NEWS_EVENT_V1\0");
        h.update(fact.source().as_bytes()); h.update([0]); h.update(fact.item_id().as_bytes());
        Ok(format!("{:x}", h.finalize()))
    }
    pub fn source(&self) -> Result<crate::event::NewsFlashAuditSource, NewsAiError> {
        let fact = self.fact()?;
        let provider = match fact.provider() {
            ProviderId::Eastmoney => "Eastmoney", ProviderId::Cailianpress => "Cailianpress",
            ProviderId::Jin10 => "Jin10", ProviderId::ThePaper => "ThePaper",
            _ => return Err(NewsAiError::NewsEvidenceMismatch("unsupported N01 source".into())),
        };
        Ok(crate::event::NewsFlashAuditSource { event_id: self.event_id()?,
            provider: provider.into(), source: fact.source().into(),
            published_at: fact.published_at().fixed_offset(),
            observed_at: fact.observed_at().fixed_offset(), batch_id: fact.source_batch_id().into() })
    }
}

/// Minted only after transactional immutable score audit AND exact readback.
#[derive(Debug, Clone)]
pub struct AuditedCriticalNews {
    evidence: CriticalNewsEvidence,
    score_audit_sha256: String,
}
impl AuditedCriticalNews {
    pub fn evidence(&self) -> &CriticalNewsEvidence { &self.evidence }
    pub fn fact(&self) -> Result<AdmittedNewsFact,NewsAiError> { self.evidence.fact() }
    pub fn completed_at(&self) -> DateTime<Utc> { self.evidence.receipt.completed_at }
    pub fn evidence_sha256(&self) -> &str { &self.score_audit_sha256 }
    pub(crate) fn mint(evidence: CriticalNewsEvidence, readback_sha: &str) -> Result<Self, NewsAiError> {
        if evidence.digest()? != readback_sha {
            return Err(NewsAiError::AnalysisAuditFailed("N01 immutable score readback mismatch".into()));
        }
        Ok(Self { evidence, score_audit_sha256: readback_sha.into() })
    }
}
impl CriticalModelResult {
    pub(crate) fn evidence(&self, audit_sha: &str) -> Result<CriticalNewsEvidence, NewsAiError> {
        let value = CriticalNewsEvidence {
            identity: self.request.business_identity.clone().ok_or_else(|| NewsAiError::AnalysisAuditFailed("N01 V3 missing".into()))?,
            news_base: NewsBaseIdentity::from_fact(self.request.fact())?,
            fact_snapshot: String::from_utf8(self.request.fact().recovery_snapshot_canonical()?)
                .map_err(|e| NewsAiError::AnalysisAuditFailed(e.to_string()))?,
            normalized_prompt: self.request.normalized_prompt.clone(), response: self.response.clone(),
            receipt: self.assessment.receipt.clone(), input_evidence_sha256: self.assessment.input_evidence_sha256.clone(),
            assessment_audit_sha256: audit_sha.into(), strength: self.strength,
        };
        value.validate()?; Ok(value)
    }
}
impl NewsAiAnalysisProfile {
    pub(super) fn is_critical(&self) -> bool {
        self.analysis_version == VERSION && self.prompt_contract == "n01_designated_strength_v1"
            && self.system_sha256 == sha256_hex(SYSTEM.as_bytes())
            && self.model_profile_revision == "receipt_bearing_json_v1"
            && self.data_contract_version == "br172_admitted_news_market_v1"
    }
}
impl NewsAIAnalyzer {
    pub fn critical_identity_profile(&self) -> Result<NewsAiAnalysisProfile, NewsAiError> {
        let mut profile = self.identity_profile()?;
        profile.analysis_version = VERSION.into(); profile.prompt_contract = "n01_designated_strength_v1".into();
        profile.system_sha256 = sha256_hex(SYSTEM.as_bytes()); profile.validate()?; Ok(profile)
    }
    pub async fn assess_critical_if_absent<L, LF, P, PF>(&self, identity: NewsAiIdentityV3, lookup: L, prepare: P)
        -> Result<Option<CriticalModelResult>, String>
    where L: FnOnce(NewsAiIdentityV3) -> LF, LF: std::future::Future<Output=Result<bool,String>>,
        P: FnOnce(NewsAiIdentityV3) -> PF, PF: std::future::Future<Output=Result<NewsAiRequest,String>> {
        if identity.profile != self.critical_identity_profile().map_err(|e|e.to_string())? {
            return Err("N01 configured profile changed".into());
        }
        if lookup(identity.clone()).await? { return Ok(None); }
        let request = prepare(identity.clone()).await?;
        if request.business_identity() != Some(&identity) { return Err("N01 prepared identity changed".into()); }
        let completed = tokio::time::timeout(std::time::Duration::from_secs(MODEL_CALL_TIMEOUT_SECONDS),
            self.provider.chat_json_with_receipt(SYSTEM, request.normalized_prompt()))
            .await.map_err(|_|"N01 model call timeout".to_owned())?
            .map_err(|e|model_call_error(e).to_string())?;
        parse_completed(request,completed).map(Some).map_err(|e|e.to_string())
    }
}

pub(super) fn restore_assessment(
        input: PersistedNewsAiAssessment,
    ) -> Result<NewsAiAssessment, NewsAiError> {
        validate_sha256("persisted assessment identity", &input.assessment_id)?;
        validate_sha256(
            "persisted assessment input evidence",
            &input.input_evidence_sha256,
        )?;
        validate_sha256(
            "persisted assessment prompt",
            &input.normalized_prompt_sha256,
        )?;
        if input.confidence > 100 {
            return Err(NewsAiError::AnalysisAuditFailed(
                "persisted assessment confidence exceeds 100".to_owned(),
            ));
        }
        if input.uncertainty.trim().is_empty() || input.core_logic.trim().is_empty() {
            return Err(NewsAiError::AnalysisAuditFailed(
                "persisted assessment reasoning is empty".to_owned(),
            ));
        }
        let receipt = ModelCallReceipt::try_from_persisted(input.receipt)?;
        if receipt.system_sha256() != sha256_hex(SYSTEM.as_bytes())
            || receipt.user_sha256() != input.normalized_prompt_sha256
        {
            return Err(NewsAiError::AnalysisAuditFailed(
                "persisted model receipt differs from NewsAI prompt evidence".to_owned(),
            ));
        }
        Ok(NewsAiAssessment {
            assessment_id: input.assessment_id,
            impact: input.impact,
            confidence: input.confidence,
            uncertainty: input.uncertainty,
            core_logic: input.core_logic,
            input_evidence_sha256: input.input_evidence_sha256,
            normalized_prompt_sha256: input.normalized_prompt_sha256,
            receipt,
        })
    }

impl CriticalNewsEvidence {
    pub(crate) fn validate_assessment(&self, value: &PersistedNewsAiAssessment, link: &str) -> Result<(),NewsAiError> {
        self.validate()?;
        let output = parse_output(&self.response)?;
        let r = &value.receipt;
        if value.assessment_id != self.assessment_id() || value.impact != output.impact
            || value.confidence != output.confidence || value.uncertainty != output.uncertainty
            || value.core_logic != output.core_logic || value.input_evidence_sha256 != self.input_evidence_sha256
            || value.normalized_prompt_sha256 != self.receipt.user_sha256
            || link != self.assessment_audit_sha256 || r.provider != self.receipt.provider
            || r.model != self.receipt.model || r.upstream_request_id != self.receipt.upstream_request_id
            || r.upstream_response_id != self.receipt.upstream_response_id || r.system_sha256 != self.receipt.system_sha256
            || r.user_sha256 != self.receipt.user_sha256 || r.response_sha256 != self.receipt.response_sha256
            || r.started_at != self.receipt.started_at || r.completed_at != self.receipt.completed_at {
            return Err(NewsAiError::AnalysisAuditFailed("N01 assessment/audit differs from complete call".into()));
        }
        Ok(())
    }
}

/// Freeze designation from admitted source instruments, never from model outputs or cursor order.
#[cfg(not(test))]
pub fn canonical_critical_target(fact: &AdmittedNewsFact) -> Option<String> {
    let AdmittedNewsSourceRecord::Global(record) = &fact.record else { return None; };
    record.instruments.iter().filter_map(|code| {
        let identity = crate::data_gateway::instrument_identity::resolve_production_equity(code,None).ok()?;
        identity.require_a_share().ok()?;
        Some(identity.storage_code().to_owned())
    }).min()
}

#[cfg(test)]
pub fn canonical_critical_target(fact: &AdmittedNewsFact) -> Option<String> {
    let AdmittedNewsSourceRecord::Global(record) = &fact.record else { return None; };
    // The same admitted production-equity set determines the minimum first.
    let canonical = record.instruments.iter().filter_map(|code| {
        let identity = crate::data_gateway::instrument_identity::resolve_production_equity(code,None).ok()?;
        identity.require_a_share().ok()?;
        Some(identity.storage_code().to_owned())
    }).min()?;
    if !fact.target_code().starts_with("TEST_CODE_") { return Some(canonical); }
    // Test presentation is allowed only for the existing, validated Test identity.
    let target = crate::data_gateway::instrument_identity::resolve_test_equity(fact.target_code(),None).ok()?;
    target.require_a_share().ok()?;
    let presented = crate::data_gateway::instrument_identity::resolve_test_equity(&format!("TEST_CODE_{canonical}"),None).ok()?;
    presented.require_a_share().ok()?;
    Some(presented.storage_code().to_owned())
}

fn parse_completed(request: NewsAiRequest,completed: ReceiptBearingJson) -> Result<CriticalModelResult,NewsAiError> {
    let (_, response, provider_receipt) = completed.into_parts();
        let receipt = ModelCallReceipt::try_from_provider(provider_receipt, SYSTEM, request.normalized_prompt(), &response)
            ?;
        let output = parse_output(&response)?;
        let assessment = NewsAiAssessment { assessment_id: request.business_identity.as_ref().ok_or_else(||NewsAiError::AnalysisAuditFailed("N01 identity missing".into()))?.digest(), impact: output.impact,
            confidence: output.confidence, uncertainty: output.uncertainty, core_logic: output.core_logic,
            input_evidence_sha256: request.evidence_hash.clone(),
            normalized_prompt_sha256: sha256_hex(request.normalized_prompt.as_bytes()), receipt };
        Ok(CriticalModelResult { request, assessment, response, strength: output.strength })
}

#[cfg(test)]
pub(crate) fn test_result(request: NewsAiRequest,response: &str) -> Result<CriticalModelResult,NewsAiError> {
    let mut profile = NewsAiAnalysisProfile::for_configured_model("TEST_CODE_MODEL_PROVIDER","TEST_CODE_MODEL_V1")?;
    profile.analysis_version = VERSION.into(); profile.prompt_contract = "n01_designated_strength_v1".into(); profile.system_sha256 = sha256_hex(SYSTEM.as_bytes());
    let identity = NewsAiIdentityV3::from_fact(&request.fact,&profile)?;
    let request = NewsAiRequest::try_new_v3(request.fact,request.market,request.optional_metrics,identity,request.chain)?;
    let start = request.fact.observed_at();
    let completed = ReceiptBearingJson::test_fixture("TEST_CODE_MODEL_PROVIDER","TEST_CODE_MODEL_V1",Some("TEST_CODE_N01_REQUEST"),"TEST_CODE_N01_RESPONSE",
        SYSTEM,request.normalized_prompt(),response,start,start+chrono::Duration::seconds(1));
    parse_completed(request,completed)
}

impl CriticalNewsEvidence {
    pub(crate) fn validate_delivery(&self,day:chrono::NaiveDate,attempt_at:DateTime<chrono::FixedOffset>) -> Result<(),NewsAiError> {
        let fact=self.fact()?;
        if fact.published_at().with_timezone(&chrono::Local).date_naive()!=day
            || fact.observed_at().with_timezone(&chrono::Local).date_naive()!=day
            || attempt_at.with_timezone(&chrono::Local).date_naive()!=day
            || attempt_at.with_timezone(&Utc)<self.receipt.completed_at {
            return Err(NewsAiError::AnalysisAuditFailed("N01 score/source/attempt day or time differs".into()));
        }
        Ok(())
    }
}
