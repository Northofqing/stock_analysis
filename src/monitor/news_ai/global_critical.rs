//! Empty-instrument news purpose; no market/stock placeholder and no delivery mint.
use super::*;
const VERSION_V1: &str = "global_critical_v1";
pub(super) const VERSION: &str = "global_critical_v2";
const SYSTEM_V1: &str = "仅评价原新闻对当前交易日A股市场整体制度、流动性或宏观风险的重要程度。importance不是confidence、来源真实性、行情事实或交易授权。不得补造市场状态或关联证券。只返回恰好importance、uncertainty、core_logic三个字段的JSON对象；importance是0到100整数，其余两项非空；不得Markdown或额外字段。";
pub(super) const SYSTEM: &str = "仅评价原新闻对当前交易日A股市场整体制度、流动性或宏观风险的重要程度。importance不是confidence、来源真实性、行情事实或交易授权。不得补造市场状态或关联证券。只返回恰好importance、uncertainty、core_logic三个字段的JSON对象。importance必须是0到100的JSON整数，不能是字符串或小数；uncertainty和core_logic必须各为非空解释字符串，不能是数字、布尔值、数组、对象或null。类型示例：{\"importance\":80,\"uncertainty\":\"尚待原始信息进一步确认\",\"core_logic\":\"说明原新闻影响整体市场的机制\"}。不得Markdown或额外字段。";


#[derive(Debug, Clone)]
pub struct GlobalCriticalFact { record: GlobalNewsRecord, batch: BatchEvidence, legacy_v1: bool }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot { schema_version: u8, record: RecoveryNewsRecord, batch: RecoveryBatchEvidence }
impl GlobalCriticalFact {
    pub fn from_admitted(batch: &AdmittedGlobalNewsBatch, index: usize) -> Result<Self, NewsAiError> {
        let record = batch.records().get(index).ok_or_else(||NewsAiError::NewsEvidenceMismatch("global record index outside admitted batch".into()))?;
        Self::from_parts(record,batch.evidence())
    }
    fn from_parts(record: &GlobalNewsRecord,batch: &BatchEvidence) -> Result<Self,NewsAiError> {
        if !record.instruments.is_empty() {
            return Err(NewsAiError::NewsEvidenceMismatch("GlobalCritical requires genuine empty instruments and admitted provider/source".into()));
        }
        validate_global_record_evidence(record,batch)?;
        validate_news_fields(&record.item_id,&record.title,record.summary.as_deref(),record.content.as_deref(),&record.canonical_url)?;
        Ok(Self { record:record.clone(),batch:batch.clone(),legacy_v1:false })
    }
    // Historical v1 snapshots retain the exact previously issued time contract.
    fn from_legacy_v1_parts(record: &GlobalNewsRecord,batch: &BatchEvidence) -> Result<Self,NewsAiError> {
        validate_batch_evidence(batch)?;
        if !record.instruments.is_empty() || expected_global_source(batch.provider)!=Some(batch.source.as_str()) {
            return Err(NewsAiError::NewsEvidenceMismatch("GlobalCritical requires genuine empty instruments and admitted provider/source".into()));
        }
        validate_source_evidence(&record.evidence,batch)?;
        let observed=parse_observed_at(&batch.observed_at)?;
        let source=record.evidence.source_at().ok_or_else(||NewsAiError::NewsEvidenceMismatch("global publication evidence missing".into()))?;
        if record.observed_at!=observed || parse_source_at(batch.provider,source)?!=record.published_at || record.published_at>observed {
            return Err(NewsAiError::NewsEvidenceMismatch("global publication/observation evidence differs".into()));
        }
        validate_news_fields(&record.item_id,&record.title,record.summary.as_deref(),record.content.as_deref(),&record.canonical_url)?;
        Ok(Self { record:record.clone(),batch:batch.clone(),legacy_v1:true })
    }
    pub(crate) fn canonical(&self) -> Result<Vec<u8>,NewsAiError> {
        if self.legacy_v1 { Self::from_legacy_v1_parts(&self.record,&self.batch)?; }
        else { Self::from_parts(&self.record,&self.batch)?; }
        let r=&self.record;
        let value=Snapshot { schema_version:1, record:RecoveryNewsRecord::Global {
            item_id:r.item_id.clone(),title:r.title.clone(),summary:r.summary.clone(),content:r.content.clone(),publisher:r.publisher.clone(),canonical_url:r.canonical_url.clone(),published_at:r.published_at,observed_at:r.observed_at,instruments:r.instruments.clone(),topics:r.topics.clone(),language:r.language.clone(),evidence:RecoverySourceEvidence::from_source(&r.evidence) },batch:RecoveryBatchEvidence::from_batch(&self.batch) };
        let bytes=serde_json::to_vec(&value).map_err(|e|NewsAiError::AnalysisAuditFailed(e.to_string()))?;
        if bytes.len()>NEWS_FACT_RECOVERY_SNAPSHOT_MAX_BYTES { return Err(NewsAiError::AnalysisAuditFailed("global snapshot extent exceeded".into())); }
        Ok(bytes)
    }
    fn from_snapshot(bytes:&[u8],legacy_v1:bool) -> Result<Self,NewsAiError> {
        if bytes.is_empty() || bytes.len()>NEWS_FACT_RECOVERY_SNAPSHOT_MAX_BYTES { return Err(NewsAiError::AnalysisAuditFailed("global snapshot extent invalid".into())); }
        let value:Snapshot=serde_json::from_slice(bytes).map_err(|e|NewsAiError::AnalysisAuditFailed(e.to_string()))?;
        if value.schema_version!=1 || serde_json::to_vec(&value).map_err(|e|NewsAiError::AnalysisAuditFailed(e.to_string()))?!=bytes { return Err(NewsAiError::AnalysisAuditFailed("global snapshot is not canonical v1".into())); }
        let RecoveryNewsRecord::Global { item_id,title,summary,content,publisher,canonical_url,published_at,observed_at,instruments,topics,language,evidence }=value.record else { return Err(NewsAiError::NewsEvidenceMismatch("global snapshot has non-global source".into())); };
        let record=GlobalNewsRecord { item_id,title,summary,content,publisher,canonical_url,published_at,observed_at,instruments,topics,language,evidence:evidence.into_source()? };
        let batch=value.batch.into_batch();
        if legacy_v1 { Self::from_legacy_v1_parts(&record,&batch) } else { Self::from_parts(&record,&batch) }
    }
    pub fn provider(&self)->ProviderId { self.batch.provider }
    pub fn source(&self)->&str { &self.batch.source }
    pub fn item_id(&self)->&str { &self.record.item_id }
    pub fn title(&self)->&str { &self.record.title }
    pub fn published_at(&self)->DateTime<Utc> { self.record.published_at }
    pub fn observed_at(&self)->DateTime<Utc> { self.record.observed_at }
    pub fn batch_id(&self)->&str { &self.batch.batch_id }
    pub(crate) fn news_base(&self)->NewsBaseIdentity {
        NewsBaseIdentity::from_exact_parts(provider_tag(self.provider()),self.item_id(),self.title(),self.record.summary.as_deref(),self.record.content.as_deref())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalCriticalIdentity { purpose:String, profile:NewsAiAnalysisProfile, news_base:NewsBaseIdentity }
impl GlobalCriticalIdentity {
    pub fn digest(&self)->String { let mut h=Sha256::new();h.update(b"BR244_GLOBAL_CRITICAL_IDENTITY_V1\0");h.update(serde_json::to_vec(self).expect("fixed identity DTO"));format!("{:x}",h.finalize()) }
}
pub struct GlobalCriticalRequest { fact:GlobalCriticalFact,identity:GlobalCriticalIdentity,prompt:String }
impl GlobalCriticalRequest {
    pub fn fact(&self)->&GlobalCriticalFact { &self.fact }
    fn build(fact:GlobalCriticalFact,profile:NewsAiAnalysisProfile)->Result<Self,NewsAiError> {
        profile.validate()?;
        if !profile.is_global_critical() { return Err(NewsAiError::AnalysisAuditFailed("global profile is not configured purpose".into())); }
        // A historical fact cannot lend its old parser contract to a new v2 request.
        let fact=if profile.analysis_version==VERSION_V1 { fact } else { GlobalCriticalFact::from_parts(&fact.record,&fact.batch)? };
        fact.canonical()?;
        let version=profile.analysis_version.clone();
        let identity=GlobalCriticalIdentity { purpose:version.clone(),profile,news_base:fact.news_base() };
        let r=&fact.record;
        let prompt=serde_json::to_string(&serde_json::json!({"analysis_version":version,"purpose":"whole_a_share_market_importance","news":{"provider":provider_tag(fact.provider()),"source":fact.source(),"batch_id":fact.batch_id(),"item_id":fact.item_id(),"published_at":r.published_at.to_rfc3339(),"title":r.title,"summary":r.summary,"content":r.content}})).map_err(|e|NewsAiError::AnalysisAuditFailed(e.to_string()))?;
        if prompt.len()>1024*1024 { return Err(NewsAiError::AnalysisAuditFailed("global prompt extent exceeded".into())); }
        Ok(Self { fact,identity,prompt })
    }
}
#[derive(Debug,Deserialize)]
#[serde(deny_unknown_fields)]
struct Output { importance:u8,uncertainty:String,core_logic:String }
fn output(raw:&str)->Result<Output,NewsAiError> {
    if raw.len()>128*1024 { return Err(NewsAiError::InvalidModelSchema("global response extent exceeded".into())); }
    let v:Output=serde_json::from_str(raw).map_err(|e|NewsAiError::InvalidModelSchema(e.to_string()))?;
    if v.importance>100 || v.uncertainty.trim().is_empty() || v.core_logic.trim().is_empty() { return Err(NewsAiError::InvalidModelSchema("global output bounds".into())); }
    Ok(v)
}
pub struct GlobalCriticalModelResult { pub(crate) evidence:GlobalCriticalEvidence }
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalCriticalEvidence { identity:GlobalCriticalIdentity,fact_snapshot:String,normalized_prompt:String,response:String,receipt:ModelCallReceipt,importance:u8,
    // Only Global v2 records the model sent by the actual HTTP adapter.
    #[serde(default,skip_serializing_if="Option::is_none")]
    requested_model:Option<String>,
}
impl GlobalCriticalEvidence {
    pub fn validate(&self)->Result<(),NewsAiError> {
        let fact=GlobalCriticalFact::from_snapshot(self.fact_snapshot.as_bytes(),self.identity.profile.analysis_version==VERSION_V1)?;
        let request=GlobalCriticalRequest::build(fact,self.identity.profile.clone())?;
        let parsed=output(&self.response)?;
        let system=self.identity.profile.global_system().ok_or_else(||NewsAiError::AnalysisAuditFailed("global_profile_unsupported".into()))?;
        let fail=|reason:&str|NewsAiError::AnalysisAuditFailed(reason.into());
        if request.identity!=self.identity { return Err(fail("global_identity_mismatch")); }
        if request.prompt!=self.normalized_prompt { return Err(fail("global_prompt_mismatch")); }
        if parsed.importance!=self.importance { return Err(fail("global_importance_mismatch")); }
        if self.receipt.system_sha256!=sha256_hex(system.as_bytes()) { return Err(fail("global_system_hash_mismatch")); }
        if self.receipt.user_sha256!=sha256_hex(self.normalized_prompt.as_bytes()) { return Err(fail("global_user_hash_mismatch")); }
        if self.receipt.response_sha256!=sha256_hex(self.response.as_bytes()) { return Err(fail("global_response_hash_mismatch")); }
        if self.receipt.provider!=self.identity.profile.model_provider { return Err(fail("global_provider_mismatch")); }
        if self.identity.profile.analysis_version==VERSION_V1 {
            if self.receipt.model!=self.identity.profile.configured_model { return Err(fail("global_upstream_model_mismatch")); }
            if self.requested_model.is_some() { return Err(fail("global_v1_requested_model_unexpected")); }
        } else {
            if self.receipt.model.trim().is_empty() { return Err(fail("global_upstream_model_missing")); }
            if self.requested_model.as_deref()!=Some(self.identity.profile.configured_model.as_str()) {
                return Err(fail("global_requested_model_mismatch"));
            }
        }
        if self.receipt.upstream_response_id.trim().is_empty() { return Err(fail("global_response_id_missing")); }
        if self.receipt.completed_at<self.receipt.started_at { return Err(fail("global_completion_before_start")); }
        if self.receipt.started_at<request.fact.observed_at() { return Err(fail("global_start_before_observation")); }
        Ok(())
    }
    pub fn canonical(&self)->Result<Vec<u8>,NewsAiError> { self.validate()?;serde_json::to_vec(self).map_err(|e|NewsAiError::AnalysisAuditFailed(e.to_string())) }
    pub fn digest(&self)->Result<String,NewsAiError> { let mut h=Sha256::new();h.update(b"BR244_GLOBAL_CRITICAL_AUDIT_V1\0");h.update(self.canonical()?);Ok(format!("{:x}",h.finalize())) }
    pub(crate) fn fact(&self)->Result<GlobalCriticalFact,NewsAiError> { self.validate()?;GlobalCriticalFact::from_snapshot(self.fact_snapshot.as_bytes(),self.identity.profile.analysis_version==VERSION_V1) }
    pub fn assessment_id(&self)->String { self.identity.digest() }
    pub fn importance(&self)->u8 { self.importance }
    pub fn event_id(&self)->Result<String,NewsAiError> { let f=self.fact()?;let mut h=Sha256::new();h.update(b"BR166_GLOBAL_NEWS_EVENT_V1\0");h.update(f.source().as_bytes());h.update([0]);h.update(f.item_id().as_bytes());Ok(format!("{:x}",h.finalize())) }
    pub fn source(&self)->Result<crate::event::NewsFlashAuditSource,NewsAiError> {
        let f=self.fact()?;let provider=match f.provider() { ProviderId::Eastmoney=>"Eastmoney",ProviderId::Cailianpress=>"Cailianpress",ProviderId::Jin10=>"Jin10",ProviderId::ThePaper=>"ThePaper",_=>return Err(NewsAiError::NewsEvidenceMismatch("unsupported global source".into())) };
        Ok(crate::event::NewsFlashAuditSource { event_id:self.event_id()?,provider:provider.into(),source:f.source().into(),published_at:f.published_at().fixed_offset(),observed_at:f.observed_at().fixed_offset(),batch_id:f.batch_id().into() })
    }
    pub(crate) fn news_base(&self)->Result<NewsBaseIdentity,NewsAiError> { Ok(self.fact()?.news_base()) }
    pub(crate) fn validate_delivery(&self,day:NaiveDate,at:DateTime<FixedOffset>)->Result<(),NewsAiError> {
        let f=self.fact()?;
        if f.published_at().with_timezone(&chrono::Local).date_naive()!=day || f.observed_at().with_timezone(&chrono::Local).date_naive()!=day || at.with_timezone(&chrono::Local).date_naive()!=day || at.with_timezone(&Utc)<self.receipt.completed_at {
            return Err(NewsAiError::AnalysisAuditFailed("global score/source/attempt day or time differs".into()));
        }Ok(())
    }
}
/// No constructor/deserializer: only successful transaction plus full immutable readback mints.
#[derive(Debug,Clone)]
pub struct AuditedGlobalCriticalNews { evidence:GlobalCriticalEvidence,audit_sha256:String }
impl AuditedGlobalCriticalNews {
    pub fn evidence(&self)->&GlobalCriticalEvidence { &self.evidence }
    pub fn evidence_sha256(&self)->&str { &self.audit_sha256 }
    pub fn fact(&self)->Result<GlobalCriticalFact,NewsAiError> { self.evidence.fact() }
    pub fn completed_at(&self)->DateTime<Utc> { self.evidence.receipt.completed_at }
    pub(crate) fn mint(evidence:GlobalCriticalEvidence,sha:&str)->Result<Self,NewsAiError> {
        if evidence.digest()?!=sha { return Err(NewsAiError::AnalysisAuditFailed("global immutable readback differs".into())); }Ok(Self { evidence,audit_sha256:sha.into() })
    }
}
impl NewsAiAnalysisProfile {
    fn global_system(&self)->Option<&'static str> {
        if self.data_contract_version!="br166_empty_global_news_v1" { return None; }
        let system=if self.analysis_version==VERSION_V1 && self.prompt_contract=="global_critical_strict_v1" && self.model_profile_revision=="receipt_bearing_json_v1" { SYSTEM_V1 }
            else if self.analysis_version==VERSION && self.prompt_contract=="global_critical_strict_v2" && self.model_profile_revision=="receipt_bearing_json_requested_model_v2" { SYSTEM }
            else { return None; };
        (self.system_sha256==sha256_hex(system.as_bytes())).then_some(system)
    }
    pub(super) fn is_global_critical(&self)->bool { self.global_system().is_some() }
}
impl NewsAIAnalyzer {
    pub fn global_critical_identity(&self,fact:&GlobalCriticalFact)->Result<GlobalCriticalIdentity,NewsAiError> { Ok(GlobalCriticalRequest::build(fact.clone(),self.global_critical_profile()?)?.identity) }
    fn global_critical_profile(&self)->Result<NewsAiAnalysisProfile,NewsAiError> {
        let mut p=self.identity_profile()?;p.analysis_version=VERSION.into();p.prompt_contract="global_critical_strict_v2".into();p.model_profile_revision="receipt_bearing_json_requested_model_v2".into();p.system_sha256=sha256_hex(SYSTEM.as_bytes());p.data_contract_version="br166_empty_global_news_v1".into();p.validate()?;Ok(p)
    }
    pub async fn assess_global_critical_if_absent<L,LF,P,PF>(&self,fact:GlobalCriticalFact,lookup:L,prepare:P)->Result<Option<GlobalCriticalModelResult>,String>
    where L:FnOnce(GlobalCriticalFact)->LF,LF:std::future::Future<Output=Result<bool,String>>,P:FnOnce(GlobalCriticalRequest)->PF,PF:std::future::Future<Output=Result<GlobalCriticalRequest,String>> {
        let request=GlobalCriticalRequest::build(fact.clone(),self.global_critical_profile().map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
        if lookup(fact).await? { return Ok(None); }
        let expected=request.identity.clone();let snapshot=request.fact.canonical().map_err(|e|e.to_string())?;let prompt=request.prompt.clone();let request=prepare(request).await?;
        if request.identity!=expected || request.fact.canonical().map_err(|e|e.to_string())?!=snapshot || request.prompt!=prompt { return Err("global prepared source/request changed".into()); }
        let completed=tokio::time::timeout(std::time::Duration::from_secs(MODEL_CALL_TIMEOUT_SECONDS),self.provider.chat_json_with_receipt(SYSTEM,&request.prompt)).await.map_err(|_|"global model timeout".to_owned())?.map_err(|e|model_call_error(e).to_string())?;
        completed_result(request,completed).map(Some).map_err(|e|e.to_string())
    }
}
fn completed_result(request:GlobalCriticalRequest,completed:ReceiptBearingJson)->Result<GlobalCriticalModelResult,NewsAiError> {
    let (_,response,actual)=completed.into_parts();
    let system=request.identity.profile.global_system().ok_or_else(||NewsAiError::AnalysisAuditFailed("global_profile_unsupported".into()))?;
    let requested_model=if request.identity.profile.analysis_version==VERSION_V1 { None } else {
        Some(actual.requested_model().filter(|value| !value.is_empty() && value.trim()==*value && !value.contains('\0'))
            .ok_or_else(||NewsAiError::ModelUnavailable("global_requested_model_missing_or_invalid".into()))?.to_owned())
    };
    let receipt=ModelCallReceipt::try_from_provider(actual,system,&request.prompt,&response)?;
    let parsed=output(&response)?;
    let evidence=GlobalCriticalEvidence { identity:request.identity,fact_snapshot:String::from_utf8(request.fact.canonical()?).map_err(|e|NewsAiError::AnalysisAuditFailed(e.to_string()))?,normalized_prompt:request.prompt,response,receipt,importance:parsed.importance,requested_model };evidence.validate()?;Ok(GlobalCriticalModelResult { evidence })
}

#[cfg(test)]
pub(crate) fn test_fact(item:&str,title:&str)->GlobalCriticalFact {
    let published=Utc.with_ymd_and_hms(2026,7,27,1,0,0).single().unwrap();let observed=published+chrono::Duration::seconds(3);
    let batch=BatchEvidence { provider:ProviderId::Cailianpress,source:"cls-v1".into(),source_at:Some(published.to_rfc3339()),observed_at:observed.to_rfc3339(),batch_id:"TEST_CODE_GLOBAL_BATCH".into() };
    let record=GlobalNewsRecord { item_id:item.into(),title:title.into(),summary:Some("TEST_CODE disclosed contract evidence".into()),content:None,publisher:"TEST_CODE publisher".into(),canonical_url:format!("https://example.com/{item}"),published_at:published,observed_at:observed,instruments:Vec::new(),topics:vec!["TEST_CODE contract".into()],language:"zh-CN".into(),evidence:SourceEvidence::new(ProviderId::Cailianpress,observed.to_rfc3339(),"TEST_CODE_GLOBAL_BATCH").unwrap().with_source_at(published.to_rfc3339()).unwrap() };
    GlobalCriticalFact::from_parts(&record,&batch).unwrap()
}
#[cfg(test)]
pub(crate) fn test_result(fact:GlobalCriticalFact,raw:&str)->Result<GlobalCriticalModelResult,NewsAiError> {
    let mut profile=NewsAiAnalysisProfile::for_configured_model("TEST_CODE_MODEL_PROVIDER","TEST_CODE_MODEL_V1")?;
    profile.analysis_version=VERSION.into();profile.prompt_contract="global_critical_strict_v2".into();profile.model_profile_revision="receipt_bearing_json_requested_model_v2".into();profile.system_sha256=sha256_hex(SYSTEM.as_bytes());profile.data_contract_version="br166_empty_global_news_v1".into();
    let request=GlobalCriticalRequest::build(fact,profile)?;let start=request.fact.observed_at();
    let completed=ReceiptBearingJson::test_fixture_requested_model("TEST_CODE_MODEL_PROVIDER","TEST_CODE_MODEL_V1","TEST_CODE_MODEL_V1",Some("TEST_CODE_GLOBAL_REQUEST"),"TEST_CODE_GLOBAL_RESPONSE",SYSTEM,&request.prompt,raw,start,start+chrono::Duration::seconds(1));
    completed_result(request,completed)
}
#[cfg(test)]
mod tests {
    use super::*;
    const GOOD:&str=r#"{"importance":91,"uncertainty":"TEST_CODE uncertainty","core_logic":"TEST_CODE macro source evidence"}"#;
    struct Provider(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    #[async_trait]
    impl LlmProvider for Provider {
        fn name(&self)->&'static str { "TEST_CODE_MODEL_PROVIDER" }
        fn model(&self)->&str { "TEST_CODE_MODEL_V1" }
        async fn chat_json(&self,_:&str,_:&str)->Result<serde_json::Value,LlmError> { Err(LlmError::ReceiptUnavailable{provider:self.name().into(),model:self.model().into()}) }
        async fn chat_json_with_receipt(&self,system:&str,user:&str)->Result<ReceiptBearingJson,LlmError> {
            self.0.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
            let at=Utc.with_ymd_and_hms(2026,7,27,1,0,4).single().unwrap();
            Ok(ReceiptBearingJson::test_fixture_requested_model(self.name(),self.model(),self.model(),Some("TEST_CODE_REAL_CALL"),"TEST_CODE_RESPONSE",system,user,GOOD,at,at+chrono::Duration::seconds(1)))
        }
    }
    #[tokio::test]
    async fn news_global_n01_empty_source_strict_receipt_and_rejection() {
        let fact=test_fact("TEST_CODE_GLOBAL_STRICT","TEST_CODE macro importance");let calls=Arc::new(std::sync::atomic::AtomicUsize::new(0));let analyzer=NewsAIAnalyzer::new(Arc::new(Provider(calls.clone())));
        for found in [Ok(true),Err("TEST_CODE legacy Unknown".to_owned())] {
            let result=analyzer.assess_global_critical_if_absent(fact.clone(),move |_|async move{found},|_|async{panic!("no preflight absence, no reserve/model")}).await;
            assert!(result.is_err() || result.unwrap().is_none());
        }
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst),0);
        let result=analyzer.assess_global_critical_if_absent(fact.clone(),|_|async{Ok(false)},|r|async{Ok(r)}).await.unwrap().unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst),1);assert_eq!(result.evidence.importance(),91);assert_eq!(result.evidence.fact().unwrap().canonical().unwrap(),fact.canonical().unwrap());
        assert_eq!(result.evidence.source().unwrap().event_id,result.evidence.event_id().unwrap());
        for raw in [r#"{"importance":101,"uncertainty":"x","core_logic":"x"}"#,r#"{"importance":90.5,"uncertainty":"x","core_logic":"x"}"#,r#"{"importance":90,"uncertainty":"","core_logic":"x"}"#,r#"{"importance":90,"uncertainty":"x","core_logic":"x","confidence":90}"#] { assert!(test_result(fact.clone(),raw).is_err()); }
        for field in ["importance","receipt","fact_snapshot","identity"] {
            let mut altered=serde_json::to_value(&result.evidence).unwrap();
            match field {
                "importance"=>altered[field]=serde_json::json!(89),
                "receipt"=>altered["requested_model"]=serde_json::json!("TEST_CODE_DIFFERENT_REQUESTED_MODEL"),
                "fact_snapshot"=>{let mut snapshot:serde_json::Value=serde_json::from_str(altered[field].as_str().unwrap()).unwrap();snapshot["record"]["instruments"]=serde_json::json!(["600519"]);altered[field]=serde_json::json!(serde_json::to_string(&snapshot).unwrap());},
                _=>altered[field]["purpose"]=serde_json::json!("equity"),
            }
            let changed:GlobalCriticalEvidence=serde_json::from_value(altered).unwrap();assert!(changed.validate().is_err());
        }
        let mut nonempty=fact.record.clone();nonempty.instruments.push("not-a-stock".into());assert!(GlobalCriticalFact::from_parts(&nonempty,&fact.batch).is_err());
        let mut mismatch=fact.record.clone();mismatch.observed_at+=chrono::Duration::seconds(1);assert!(GlobalCriticalFact::from_parts(&mismatch,&fact.batch).is_err());
        let mut other=fact.clone();other.record.content=Some(String::new());
        assert!(!other.news_base().same_text_revision(&fact.news_base())); // None and present-empty are distinct actual bytes.
    }


    #[test]
    fn news_global_input_contract_provider_times_cold_recovery_and_tamper() {
        let base=test_fact("TEST_CODE_WIRE_TIME","TEST_CODE original provider publication");
        let pub_ms=format!("unix-ms:{}",base.published_at().timestamp_millis());
        let obs_ms=format!("unix-ms:{}",base.observed_at().timestamp_millis());
        let pub_seconds=base.published_at().timestamp().to_string();
        let obs_rfc=base.observed_at().to_rfc3339();
        for (provider,source,published,observed) in [
            (ProviderId::Eastmoney,"eastmoney-web","2026-07-27 09:00",obs_ms.as_str()),
            (ProviderId::Jin10,"jin10-flash-v1","2026-07-27 09:00:00",obs_rfc.as_str()),
            (ProviderId::Cailianpress,"cls-v1",pub_seconds.as_str(),obs_rfc.as_str()),
            (ProviderId::ThePaper,"thepaper-finance-v1",pub_ms.as_str(),obs_ms.as_str()),
        ] {
            let mut record=base.record.clone();
            let batch=BatchEvidence {provider,source:source.into(),source_at:Some(published.into()),
                observed_at:observed.into(),batch_id:"TEST_CODE_PROVIDER_BATCH".into()};
            record.evidence=SourceEvidence::new(provider,observed,"TEST_CODE_PROVIDER_BATCH").unwrap()
                .with_source_at(published).unwrap();
            let fact=GlobalCriticalFact::from_parts(&record,&batch).unwrap();
            assert_eq!(fact.record.evidence.source_at(),Some(published));
            assert_eq!(fact.batch.observed_at,observed);
            let bytes=fact.canonical().unwrap();
            let cold=GlobalCriticalFact::from_snapshot(&bytes,false).unwrap();
            assert_eq!(cold.canonical().unwrap(),bytes);
            assert_eq!(cold.published_at(),base.published_at());
            assert_eq!(cold.observed_at(),base.observed_at());
            record.instruments.push("TEST_CODE_600519".into());
            let equity=AdmittedNewsFact::from_global_parts(&record,&batch,"TEST_CODE_600519").unwrap();
            let equity_bytes=equity.recovery_snapshot_canonical().unwrap();
            assert_eq!(AdmittedNewsFact::from_recovery_snapshot(&equity_bytes).unwrap()
                .recovery_snapshot_canonical().unwrap(),equity_bytes);
            let mut drift=fact.record.clone();drift.published_at+=chrono::Duration::seconds(1);
            assert!(matches!(GlobalCriticalFact::from_parts(&drift,&batch),
                Err(NewsAiError::NewsEvidenceMismatch(reason)) if reason=="global_publication_mismatch"));
            let mut drift=batch.clone();drift.source.push_str("_other");
            assert!(GlobalCriticalFact::from_parts(&fact.record,&drift).is_err());
            let mut tampered:Snapshot=serde_json::from_slice(&bytes).unwrap();
            tampered.batch.observed_at=base.published_at().to_rfc3339();
            assert!(GlobalCriticalFact::from_snapshot(&serde_json::to_vec(&tampered).unwrap(),false).is_err());
        }
        assert!(GlobalCriticalFact::from_parts(&base.record,&BatchEvidence {
            provider:ProviderId::Sina,source:"sina".into(),..base.batch.clone()
        }).is_err());
    }

    #[test]
    fn news_global_input_contract_v2_prompt_and_legacy_v1_receipts() {
        let fact=test_fact("TEST_CODE_PROMPT_VERSION","TEST_CODE unchanged old content");
        let mut legacy_profile=NewsAiAnalysisProfile::for_configured_model("TEST_CODE_MODEL_PROVIDER","TEST_CODE_MODEL_V1").unwrap();
        legacy_profile.analysis_version=VERSION_V1.into();legacy_profile.prompt_contract="global_critical_strict_v1".into();
        legacy_profile.system_sha256=sha256_hex(SYSTEM_V1.as_bytes());legacy_profile.data_contract_version="br166_empty_global_news_v1".into();
        let old_fact=GlobalCriticalFact::from_legacy_v1_parts(&fact.record,&fact.batch).unwrap();
        let request=GlobalCriticalRequest::build(old_fact,legacy_profile).unwrap();
        assert_eq!(request.identity.purpose,"global_critical_v1");
        assert_eq!(serde_json::from_str::<serde_json::Value>(&request.prompt).unwrap()["analysis_version"],"global_critical_v1");
        let start=fact.observed_at();
        let legacy_prompt=request.prompt.clone();
        let old=completed_result(request,ReceiptBearingJson::test_fixture("TEST_CODE_MODEL_PROVIDER","TEST_CODE_MODEL_V1",
            Some("TEST_CODE_OLD_REQUEST"),"TEST_CODE_OLD_RESPONSE",SYSTEM_V1,
            &legacy_prompt,GOOD,start,start+chrono::Duration::seconds(1))).unwrap();
        let old_bytes=old.evidence.canonical().unwrap();let old_digest=old.evidence.digest().unwrap();
        assert!(serde_json::from_slice::<serde_json::Value>(&old_bytes).unwrap().get("requested_model").is_none());
        let cold:GlobalCriticalEvidence=serde_json::from_slice(&old_bytes).unwrap();
        cold.validate().unwrap();assert_eq!(cold.canonical().unwrap(),old_bytes);assert_eq!(cold.digest().unwrap(),old_digest);
        assert_eq!(cold.receipt.system_sha256,sha256_hex(SYSTEM_V1.as_bytes()));
        assert_eq!(sha256_hex(SYSTEM_V1.as_bytes()),"52294c4580c1b5c927be373ac85b664a70a571336fe0a032a9b316977dc24fc9");
        let current=test_result(fact.clone(),GOOD).unwrap();
        assert_eq!(current.evidence.identity.purpose,VERSION);
        assert_ne!(current.evidence.assessment_id(),old.evidence.assessment_id());
        assert_ne!(sha256_hex(SYSTEM.as_bytes()),sha256_hex(SYSTEM_V1.as_bytes()));
        assert!(SYSTEM.contains(r#"{"importance":80,"uncertainty":"尚待原始信息进一步确认","core_logic":"说明原新闻影响整体市场的机制"}"#));
        for raw in [r#"{"importance":91,"uncertainty":80,"core_logic":"x"}"#,
            r#"{"importance":91,"uncertainty":"x","core_logic":80}"#,
            r#"{"importance":"91","uncertainty":"x","core_logic":"x"}"#] {
            assert!(matches!(test_result(fact.clone(),raw),Err(NewsAiError::InvalidModelSchema(_))));
        }
        let profile=current.evidence.identity.profile.clone();
        let request=GlobalCriticalRequest::build(fact.clone(),profile.clone()).unwrap();
        let completed=ReceiptBearingJson::test_fixture_requested_model("TEST_CODE_MODEL_PROVIDER","TEST_CODE_MODEL_V1","TEST_CODE_UPSTREAM_ACTUAL",
            None,"TEST_CODE_DISTINCT_RESPONSE",SYSTEM,&request.prompt,GOOD,start,start+chrono::Duration::seconds(1));
        let distinct=completed_result(request,completed).unwrap();
        assert_eq!(distinct.evidence.receipt.model,"TEST_CODE_UPSTREAM_ACTUAL");
        assert_eq!(distinct.evidence.requested_model.as_deref(),Some("TEST_CODE_MODEL_V1"));
        let bytes=distinct.evidence.canonical().unwrap();
        let cold:GlobalCriticalEvidence=serde_json::from_slice(&bytes).unwrap();assert_eq!(cold.canonical().unwrap(),bytes);
        for requested in [None,Some("TEST_CODE_WRONG_REQUESTED"),Some("")] {
            let request=GlobalCriticalRequest::build(fact.clone(),profile.clone()).unwrap();
            let completed=match requested {
                Some(model)=>ReceiptBearingJson::test_fixture_requested_model("TEST_CODE_MODEL_PROVIDER",model,"TEST_CODE_UPSTREAM_ACTUAL",
                    None,"TEST_CODE_BAD_REQUEST",SYSTEM,&request.prompt,GOOD,start,start+chrono::Duration::seconds(1)),
                None=>ReceiptBearingJson::test_fixture("TEST_CODE_MODEL_PROVIDER","TEST_CODE_MODEL_V1",
                    None,"TEST_CODE_MISSING_REQUEST",SYSTEM,&request.prompt,GOOD,start,start+chrono::Duration::seconds(1)),
            };
            assert!(completed_result(request,completed).is_err());
        }
        let mut old_changed=cold;old_changed.identity.profile=old.evidence.identity.profile.clone();
        assert!(old_changed.validate().is_err());
        let mut old_injected=old.evidence;old_injected.requested_model=Some("TEST_CODE_MODEL_V1".into());
        assert!(matches!(old_injected.validate(),Err(NewsAiError::AnalysisAuditFailed(reason)) if reason=="global_v1_requested_model_unexpected"));
    }

    #[test]
    fn news_global_input_contract_static_first_fault_reasons() {
        let fact=test_fact("TEST_CODE_REASON","TEST_CODE immutable input");
        let baseline=test_result(fact.clone(),GOOD).unwrap().evidence;
        for (field,reason) in [
            ("identity","global_identity_mismatch"),("prompt","global_prompt_mismatch"),
            ("importance","global_importance_mismatch"),("system","global_system_hash_mismatch"),
            ("user","global_user_hash_mismatch"),("response","global_response_hash_mismatch"),
            ("provider","global_provider_mismatch"),("requested","global_requested_model_mismatch"),
            ("upstream","global_upstream_model_missing"),("response_id","global_response_id_missing"),
            ("completed","global_completion_before_start"),("started","global_start_before_observation"),
        ] {
            let mut changed=baseline.clone();
            match field {
                "identity"=>changed.identity.purpose="TEST_CODE_OTHER_PURPOSE".into(),
                "prompt"=>changed.normalized_prompt.push(' '),
                "importance"=>changed.importance=90,
                "system"=>changed.receipt.system_sha256="0".repeat(64),
                "user"=>changed.receipt.user_sha256="0".repeat(64),
                "response"=>changed.receipt.response_sha256="0".repeat(64),
                "provider"=>changed.receipt.provider="TEST_CODE_OTHER_PROVIDER".into(),
                "requested"=>changed.requested_model=Some("TEST_CODE_OTHER_MODEL".into()),
                "upstream"=>changed.receipt.model.clear(),
                "response_id"=>changed.receipt.upstream_response_id.clear(),
                "completed"=>changed.receipt.completed_at=changed.receipt.started_at-chrono::Duration::seconds(1),
                "started"=>changed.receipt.started_at=fact.observed_at()-chrono::Duration::seconds(1),
                _=>unreachable!(),
            }
            assert!(matches!(changed.validate(),Err(NewsAiError::AnalysisAuditFailed(actual)) if actual==reason),"cut={field}");
            let bytes=serde_json::to_vec(&changed).unwrap();
            let cold:GlobalCriticalEvidence=serde_json::from_slice(&bytes).unwrap();assert!(cold.validate().is_err());
        }
        let mut two=baseline;two.importance=1;two.receipt.model.clear();
        assert!(matches!(two.validate(),Err(NewsAiError::AnalysisAuditFailed(reason)) if reason=="global_importance_mismatch"));
    }
}
