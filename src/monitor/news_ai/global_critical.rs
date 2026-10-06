//! Empty-instrument news purpose; no market/stock placeholder and no delivery mint.
use super::*;
pub(super) const VERSION: &str = "global_critical_v1";
pub(super) const SYSTEM: &str = "仅评价原新闻对当前交易日A股市场整体制度、流动性或宏观风险的重要程度。importance不是confidence、来源真实性、行情事实或交易授权。不得补造市场状态或关联证券。只返回恰好importance、uncertainty、core_logic三个字段的JSON对象；importance是0到100整数，其余两项非空；不得Markdown或额外字段。";

#[derive(Debug, Clone)]
pub struct GlobalCriticalFact { record: GlobalNewsRecord, batch: BatchEvidence }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot { schema_version: u8, record: RecoveryNewsRecord, batch: RecoveryBatchEvidence }
impl GlobalCriticalFact {
    pub fn from_admitted(batch: &AdmittedGlobalNewsBatch, index: usize) -> Result<Self, NewsAiError> {
        let record = batch.records().get(index).ok_or_else(||NewsAiError::NewsEvidenceMismatch("global record index outside admitted batch".into()))?;
        Self::from_parts(record,batch.evidence())
    }
    fn from_parts(record: &GlobalNewsRecord,batch: &BatchEvidence) -> Result<Self,NewsAiError> {
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
        Ok(Self { record:record.clone(),batch:batch.clone() })
    }
    pub(crate) fn canonical(&self) -> Result<Vec<u8>,NewsAiError> {
        Self::from_parts(&self.record,&self.batch)?;
        let r=&self.record;
        let value=Snapshot { schema_version:1, record:RecoveryNewsRecord::Global {
            item_id:r.item_id.clone(),title:r.title.clone(),summary:r.summary.clone(),content:r.content.clone(),publisher:r.publisher.clone(),canonical_url:r.canonical_url.clone(),published_at:r.published_at,observed_at:r.observed_at,instruments:r.instruments.clone(),topics:r.topics.clone(),language:r.language.clone(),evidence:RecoverySourceEvidence::from_source(&r.evidence) },batch:RecoveryBatchEvidence::from_batch(&self.batch) };
        let bytes=serde_json::to_vec(&value).map_err(|e|NewsAiError::AnalysisAuditFailed(e.to_string()))?;
        if bytes.len()>NEWS_FACT_RECOVERY_SNAPSHOT_MAX_BYTES { return Err(NewsAiError::AnalysisAuditFailed("global snapshot extent exceeded".into())); }
        Ok(bytes)
    }
    pub(crate) fn from_snapshot(bytes:&[u8]) -> Result<Self,NewsAiError> {
        if bytes.is_empty() || bytes.len()>NEWS_FACT_RECOVERY_SNAPSHOT_MAX_BYTES { return Err(NewsAiError::AnalysisAuditFailed("global snapshot extent invalid".into())); }
        let value:Snapshot=serde_json::from_slice(bytes).map_err(|e|NewsAiError::AnalysisAuditFailed(e.to_string()))?;
        if value.schema_version!=1 || serde_json::to_vec(&value).map_err(|e|NewsAiError::AnalysisAuditFailed(e.to_string()))?!=bytes { return Err(NewsAiError::AnalysisAuditFailed("global snapshot is not canonical v1".into())); }
        let RecoveryNewsRecord::Global { item_id,title,summary,content,publisher,canonical_url,published_at,observed_at,instruments,topics,language,evidence }=value.record else { return Err(NewsAiError::NewsEvidenceMismatch("global snapshot has non-global source".into())); };
        Self::from_parts(&GlobalNewsRecord { item_id,title,summary,content,publisher,canonical_url,published_at,observed_at,instruments,topics,language,evidence:evidence.into_source()? },&value.batch.into_batch())
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
        fact.canonical()?;
        let identity=GlobalCriticalIdentity { purpose:VERSION.into(),profile,news_base:fact.news_base() };
        let r=&fact.record;
        let prompt=serde_json::to_string(&serde_json::json!({"analysis_version":VERSION,"purpose":"whole_a_share_market_importance","news":{"provider":provider_tag(fact.provider()),"source":fact.source(),"batch_id":fact.batch_id(),"item_id":fact.item_id(),"published_at":r.published_at.to_rfc3339(),"title":r.title,"summary":r.summary,"content":r.content}})).map_err(|e|NewsAiError::AnalysisAuditFailed(e.to_string()))?;
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
pub struct GlobalCriticalEvidence { identity:GlobalCriticalIdentity,fact_snapshot:String,normalized_prompt:String,response:String,receipt:ModelCallReceipt,importance:u8 }
impl GlobalCriticalEvidence {
    pub fn validate(&self)->Result<(),NewsAiError> {
        let fact=GlobalCriticalFact::from_snapshot(self.fact_snapshot.as_bytes())?;
        let request=GlobalCriticalRequest::build(fact,self.identity.profile.clone())?;
        let parsed=output(&self.response)?;
        if request.identity!=self.identity || request.prompt!=self.normalized_prompt || parsed.importance!=self.importance
            || self.receipt.system_sha256!=sha256_hex(SYSTEM.as_bytes()) || self.receipt.user_sha256!=sha256_hex(self.normalized_prompt.as_bytes()) || self.receipt.response_sha256!=sha256_hex(self.response.as_bytes())
            || self.receipt.provider!=self.identity.profile.model_provider || self.receipt.model!=self.identity.profile.configured_model || self.receipt.upstream_response_id.trim().is_empty()
            || self.receipt.completed_at<self.receipt.started_at || self.receipt.started_at<request.fact.observed_at() {
            return Err(NewsAiError::AnalysisAuditFailed("global actual source/purpose/model/call/output changed".into()));
        }
        Ok(())
    }
    pub fn canonical(&self)->Result<Vec<u8>,NewsAiError> { self.validate()?;serde_json::to_vec(self).map_err(|e|NewsAiError::AnalysisAuditFailed(e.to_string())) }
    pub fn digest(&self)->Result<String,NewsAiError> { let mut h=Sha256::new();h.update(b"BR244_GLOBAL_CRITICAL_AUDIT_V1\0");h.update(self.canonical()?);Ok(format!("{:x}",h.finalize())) }
    pub(crate) fn fact(&self)->Result<GlobalCriticalFact,NewsAiError> { self.validate()?;GlobalCriticalFact::from_snapshot(self.fact_snapshot.as_bytes()) }
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
    pub(super) fn is_global_critical(&self)->bool { self.analysis_version==VERSION && self.prompt_contract=="global_critical_strict_v1" && self.system_sha256==sha256_hex(SYSTEM.as_bytes()) && self.model_profile_revision=="receipt_bearing_json_v1" && self.data_contract_version=="br166_empty_global_news_v1" }
}
impl NewsAIAnalyzer {
    pub fn global_critical_identity(&self,fact:&GlobalCriticalFact)->Result<GlobalCriticalIdentity,NewsAiError> { Ok(GlobalCriticalRequest::build(fact.clone(),self.global_critical_profile()?)?.identity) }
    fn global_critical_profile(&self)->Result<NewsAiAnalysisProfile,NewsAiError> {
        let mut p=self.identity_profile()?;p.analysis_version=VERSION.into();p.prompt_contract="global_critical_strict_v1".into();p.system_sha256=sha256_hex(SYSTEM.as_bytes());p.data_contract_version="br166_empty_global_news_v1".into();p.validate()?;Ok(p)
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
    let (_,response,actual)=completed.into_parts();let receipt=ModelCallReceipt::try_from_provider(actual,SYSTEM,&request.prompt,&response)?;let parsed=output(&response)?;
    let evidence=GlobalCriticalEvidence { identity:request.identity,fact_snapshot:String::from_utf8(request.fact.canonical()?).map_err(|e|NewsAiError::AnalysisAuditFailed(e.to_string()))?,normalized_prompt:request.prompt,response,receipt,importance:parsed.importance };evidence.validate()?;Ok(GlobalCriticalModelResult { evidence })
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
    profile.analysis_version=VERSION.into();profile.prompt_contract="global_critical_strict_v1".into();profile.system_sha256=sha256_hex(SYSTEM.as_bytes());profile.data_contract_version="br166_empty_global_news_v1".into();
    let request=GlobalCriticalRequest::build(fact,profile)?;let start=request.fact.observed_at();
    let completed=ReceiptBearingJson::test_fixture("TEST_CODE_MODEL_PROVIDER","TEST_CODE_MODEL_V1",Some("TEST_CODE_GLOBAL_REQUEST"),"TEST_CODE_GLOBAL_RESPONSE",SYSTEM,&request.prompt,raw,start,start+chrono::Duration::seconds(1));
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
            Ok(ReceiptBearingJson::test_fixture(self.name(),self.model(),Some("TEST_CODE_REAL_CALL"),"TEST_CODE_RESPONSE",system,user,GOOD,at,at+chrono::Duration::seconds(1)))
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
                "receipt"=>altered[field]["model"]=serde_json::json!("TEST_CODE_DIFFERENT_MODEL"),
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
}
