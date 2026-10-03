//! Closed paid financial operations. No approval, budget factory or deserialize authority.
#![allow(dead_code)]
use super::paper_ledger::{
    LedgerError,
    Lot,
    Projection
};
use super::paper_book_v2_execution::{
    ExecutionManifest,
    LotClaim,
    FillRecord
};
use super::paper_book_v2_budget_v1::{
    WorkingReservation,
    MarkedAllocation
};
use super::paper_book_v2_fill_model::WindowRecord;
use super::paper_replay_codec_v1::{
    self as codec,
    Value,
    MapEntry,
    Root
};
use crate::database::global_schema_v1::replay_work::{
    ReplayMemory,
    CodecMechanics,
    TransitionOps,
    ReplayTerminalFailure
};
use crate::performance::fee_policy::{
    AShareFeeV2Error,
    AShareFeePolicyV2,
    StampTaxBracketV2
};
use std::collections::{
    BTreeMap,
    BTreeSet
};
use sha2::{
    Digest,
    Sha256
};
pub(crate) type Result<T> = std::result::Result<T, FinancialFailure>;
#[derive(Debug)]
pub(crate) enum FinancialFailure {
    Financial(LedgerError),
    Fee(AShareFeeV2Error),
    Terminal(ReplayTerminalFailure)
}
impl From<LedgerError> for FinancialFailure {
    fn from(e: LedgerError)->Self {
        Self::Financial(e)
    }
}
impl From<AShareFeeV2Error> for FinancialFailure {
    fn from(e: AShareFeeV2Error)->Self{
        Self::Fee(e)
    }
}
impl From<ReplayTerminalFailure> for FinancialFailure {
    fn from(e: ReplayTerminalFailure)->Self{
        Self::Terminal(e)
    }
}
pub(crate) fn historical<T>(r: Result<T>)->std::result::Result<T, LedgerError>{
    match r{
        Ok(v)=>Ok(v),
        Err(FinancialFailure::Financial(e))=>Err(e),
        Err(_)=>unreachable!("Historical financial dispatch cannot produce a terminal or unmapped fee error")
    }
}
pub(crate) fn historical_fee<T>(r: Result<T>)->std::result::Result<T, AShareFeeV2Error>{
    match r{
        Ok(v)=>Ok(v),
        Err(FinancialFailure::Fee(e))=>Err(e),
        Err(_)=>unreachable!("Historical fee dispatch cannot produce replay failure")
    }
}
pub(crate) fn fee_evidence<T>(r:Result<T>, w:&mut FinancialWork<'_, '_>)->Result<T>{
    match r{
        Err(FinancialFailure::Fee(e))=>Err(LedgerError::EvidenceUnavailable(w.text(ClosedFinancialText::FeeError(&e))?).into()),
        other=>other
    }
}
pub(crate) enum FinancialWork<'loan, 'pool>{
    Historical,
    Bounded(ReplayMemory<'loan, 'pool>),
    #[cfg(test)]
    Fixture(crate::database::global_schema_v1::replay_work::FinancialFixtureLoan<'loan, 'pool>),
}
impl<'loan, 'pool> FinancialWork<'loan, 'pool>{
    fn is_historical(&self)->bool{
        matches!(self, Self::Historical)
    }
    pub(crate) fn finish(&self)->Result<()>{
        match self{
            Self::Historical=>Ok(()),
            Self::Bounded(m)=>Ok(m.finish()?),
            #[cfg(test)]
            Self::Fixture(m)=>Ok(m.finish()?)
        }
    }
    fn codec(&mut self)->std::result::Result<CodecMechanics<'_,
    'pool>,
    ReplayTerminalFailure>{
        match self{
            Self::Historical=>unreachable!("Historical has no paid mechanics"),
            Self::Bounded(m)=>m.mechanics(),
            #[cfg(test)]
            Self::Fixture(m)=>m.mechanics()
        }
    }
    fn ops(&mut self)->std::result::Result<TransitionOps<'_,
    'pool>,
    ReplayTerminalFailure>{
        match self{
            Self::Historical=>unreachable!("Historical has no transition mechanics"),
            Self::Bounded(m)=>m.transition_ops(),
            #[cfg(test)]
            Self::Fixture(m)=>m.transition_ops()
        }
    }
    pub(crate) fn fee_source_revision(&mut self, value:&str)->Result<String>{
        if self.is_historical(){
            Ok(value.to_owned())
        } else{
            Ok(self.codec()?.string(value)?)
        }
    }
    pub(crate) fn option<T>(&mut self, value:Option<T>, reason:ClosedFinancialText<'_>)->Result<T>{
        match value{
            Some(v)=>Ok(v),
            None=>Err(self.error(reason)?)
        }
    }
    pub(crate) fn copy<T:Value+Clone>(&mut self, v:&T)->Result<T>{
        if self.is_historical(){
            Ok(v.clone())
        } else{
            Ok(v.paid_copy(&mut self.codec()?)?)
        }
    }
    pub(crate) fn text(&mut self, v:ClosedFinancialText<'_>)->Result<String>{
        let bytes=if self.is_historical(){
            let mut sink=FinancialSink::Owned(Vec::new());
            v.write(&mut sink).expect("Historical fixed writer");
            sink.into_owned()
        } else{
            self.ops()?.financial_output(FinancialOutput::Text(v))?
        };
        Ok(String::from_utf8(bytes).expect("fixed tokens and borrowed UTF8"))
    }
    pub(crate) fn error(&mut self, v:ClosedFinancialText<'_>)->Result<FinancialFailure>{
        let category=v.category();
        let text=self.text(v)?;
        Ok(match category{
            ErrorCategory::Invalid=>LedgerError::InvalidInput(text),
            ErrorCategory::Integrity=>LedgerError::IntegrityFailure(text),
            ErrorCategory::Evidence=>LedgerError::EvidenceUnavailable(text)
        } .into())
    }
    pub(crate) fn require(&mut self, ok:bool, v:ClosedFinancialText<'_>)->Result<()>{
        self.finish()?;
        if ok{
            Ok(())
        } else{
            Err(self.error(v)?)
        }
    }
    pub(crate) fn insert<K:codec::Value+Ord,
    V:codec::Value>(&mut self, map:&mut BTreeMap<K, V>, key:K, value:V)->Result<()> where(K, V):MapEntry {
        if self.is_historical(){
            map.insert(key, value);
            Ok(())
        } else{
            Ok(self.codec()?.insert(map, key, value)?)
        }
    }
    pub(crate) fn push<T:TransitionElement>(&mut self, vec:&mut Vec<T>, value:T)->Result<()>{
        if self.is_historical(){
            vec.push(value);
            Ok(())
        } else{
            Ok(self.ops()?.push_transition(vec, value)?)
        }
    }
    pub(crate) fn set<'a>(&mut self, set:&mut BTreeSet<&'a str>, value:&'a str)->Result<bool>{
        if self.is_historical(){
            Ok(set.insert(value))
        } else{
            Ok(self.ops()?.str_set_insert(set, value)?)
        }
    }
    pub(crate) fn lot_ref<'a>(&mut self, map:&mut BTreeMap<&'a str, &'a Lot>, key:&'a str, value:&'a Lot)->Result<()>{
        if self.is_historical(){
            map.insert(key, value);
        } else{
            self.ops()?.lot_ref_insert(map, key, value)?;
        }
        Ok(())
    }
    pub(crate) fn descriptor<'a>(&mut self, map:&mut BTreeMap<&'a str, &'a str>, key:&'a str, value:&'a str)->Result<Option<&'a str>>{
        if self.is_historical(){
            Ok(map.insert(key, value))
        } else{
            Ok(self.ops()?.descriptor_field_insert(map, key, value)?)
        }
    }
    pub(crate) fn claim<'m,
    'a>(&mut self, map:&'m mut BTreeMap<&'a str, u32>, key:&'a str)->Result<&'m mut u32>{
        if self.is_historical(){
            Ok(map.entry(key).or_default())
        } else{
            Ok(self.ops()?.claim_slot(map, key)?)
        }
    }
    pub(crate) fn exposure<'m,
    'a>(&mut self, map:&'m mut BTreeMap<&'a str, i128>, key:&'a str)->Result<&'m mut i128>{
        if self.is_historical(){
            Ok(map.entry(key).or_default())
        } else{
            Ok(self.ops()?.exposure_slot(map, key)?)
        }
    }
    pub(crate) fn sort_fifo(&mut self, lots:&mut Vec<&Lot>)->Result<()>{
        if self.is_historical(){
            super::paper_book_v2_execution::sort_fifo_lots_owner(lots);
            Ok(())
        } else{
            Ok(self.ops()?.sort_fifo_lots(lots)?)
        }
    }
    pub(crate) fn fee_descriptor(&mut self, policy:&AShareFeePolicyV2)->Result<Vec<u8>>{
        if self.is_historical(){
            let mut out=FinancialSink::Owned(Vec::new());
            policy.write_replay_descriptor(&mut out).expect("Historical descriptor writer");
            Ok(out.into_owned())
        } else{
            Ok(self.ops()?.financial_output(FinancialOutput::FeeDescriptor(policy))?)
        }
    }
    pub(crate) fn fee_instance(&mut self, policy:&AShareFeePolicyV2)->Result<String>{
        let bytes=self.fee_descriptor(policy)?;
        let mut h=Sha256::new();
        h.update(b"a-share-fee-policy-descriptor/v1\n");
        h.update(bytes);
        let digest=FeeDescriptorDigest(self.hex(h.finalize().into())?);
        self.text(ClosedFinancialText::FeeInstance(&digest))
    }
    pub(crate) fn hex(&mut self, digest:[u8; 32])->Result<String>{
        #[cfg(test)]
        self.note_hash(HashAction::Hex);
        if self.is_historical(){
            Ok(hex::encode(digest))
        } else{
            Ok(self.codec()?.hex_digest(&digest)?)
        }
    }
    pub(crate) fn fixed_hash(&mut self, input:ClosedFinancialHash<'_>)->Result<String>{
        let bytes=if self.is_historical(){
            input.historical_json()?
        } else{
            self.ops()?.finish()?;
            let mut m=self.codec()?;
            m.serializer_escrow()?;
            let mut count=JsonSink::Count(0);
            input.write_json(&mut count).map_err(|_|m.refuse(codec::K::RecordExtent, None))?;
            let n=count.count();
            let mut bytes=m.output(n)?;
            input.write_json(&mut JsonSink::Output{
                bytes:&mut bytes,
                limit:n
            }).map_err(|_|m.refuse(codec::K::PlanMismatch, None))?;
            if bytes.len()!=n{
                return Err(m.refuse(codec::K::PlanMismatch, None).into());
            }
            bytes
        };
        #[cfg(test)]
        self.note_hash(HashAction::Output);
        if input.execution(){
            #[cfg(test)]
            self.note_hash(HashAction::Guard);
            super::paper_book_v2_execution::require_execution_record_extent_with_work(&bytes, self)?;
        }
        #[cfg(test)]
        self.note_hash(HashAction::PayloadHash);
        let mut h=Sha256::new();
        if let Some(domain)=input.domain(){
            h.update(domain);
            h.update(b"\n");
        }
        h.update(bytes);
        self.hex(h.finalize().into())
    }
    pub(crate) fn decode<T:Root+serde::de::DeserializeOwned>(&mut self, bytes:&[u8])->Result<T>{
        if self.is_historical(){
            Ok(serde_json::from_slice(bytes).map_err(|e|LedgerError::IntegrityFailure(e.to_string()))?)
        } else{
            match self{
                Self::Bounded(m)=>Ok(codec::decode_record(bytes, m)?),
                #[cfg(test)]
                Self::Fixture(m)=>Ok(m.decode::<T>(bytes)?),
                _=>unreachable!()
            }
        }
    }
    pub(crate) fn canonical_equal<T:Root>(&mut self, value:&T, bytes:&[u8])->Result<bool>{
        if self.is_historical(){
            Ok(serde_json::to_vec(value).map_err(|e|LedgerError::IntegrityFailure(e.to_string()))?==bytes)
        } else{
            Ok(codec::encode_core(value, &mut self.codec()?)?==bytes)
        }
    }
    pub(crate) fn raw_hash(&mut self, bytes:&[u8])->Result<String>{
        self.hex(Sha256::digest(bytes).into())
    }
    pub(crate) fn cutover_hash(&mut self, bytes:&[u8])->Result<String>{
        let mut h=Sha256::new();
        h.update(b"paper-book-v2-cutover-manifest/v1\n");
        h.update(bytes);
        self.hex(h.finalize().into())
    }
}
impl FinancialWork<'_, '_>{
    pub(crate) fn calendar_day(&mut self, day:chrono::NaiveDate)->Result<bool>{
        use crate::database::global_schema_v1::replay_work::ReplayCalendarCallFailure as E;
        let value=match self{
            Self::Historical=>return crate::calendar::verified_a_share_trading_day(day).map_err(|e|LedgerError::EvidenceUnavailable(e).into()),
            Self::Bounded(m)=>m.calendar_day(day),
            #[cfg(test)]
            Self::Fixture(m)=>m.calendar_day(day)
        };
        match value{
            Ok(v)=>Ok(v),
            Err(E::Historical(e))=>Err(LedgerError::EvidenceUnavailable(e).into()),
            Err(E::Terminal(e))=>Err(e.into())
        }
    }
}
impl FinancialWork<'_, '_>{
    pub(crate) fn calendar_prev(&mut self, day:chrono::NaiveDate)->Result<chrono::NaiveDate>{
        use crate::database::global_schema_v1::replay_work::ReplayCalendarCallFailure as E;
        let value=match self{
            Self::Historical=>return crate::calendar::verified_prev_a_share_trading_day(day).map_err(|e|LedgerError::EvidenceUnavailable(e).into()),
            Self::Bounded(m)=>m.calendar_prev(day),
            #[cfg(test)]
            Self::Fixture(m)=>m.calendar_prev(day)
        };
        match value{
            Ok(v)=>Ok(v),
            Err(E::Historical(e))=>Err(LedgerError::EvidenceUnavailable(e).into()),
            Err(E::Terminal(e))=>Err(e.into())
        }
    }
}
impl FinancialWork<'_, '_>{
    pub(crate) fn calendar_next(&mut self, day:chrono::NaiveDate)->Result<chrono::NaiveDate>{
        use crate::database::global_schema_v1::replay_work::ReplayCalendarCallFailure as E;
        let value=match self{
            Self::Historical=>return crate::calendar::verified_next_a_share_trading_day(day).map_err(|e|LedgerError::EvidenceUnavailable(e).into()),
            Self::Bounded(m)=>m.calendar_next(day),
            #[cfg(test)]
            Self::Fixture(m)=>m.calendar_next(day)
        };
        match value{
            Ok(v)=>Ok(v),
            Err(E::Historical(e))=>Err(LedgerError::EvidenceUnavailable(e).into()),
            Err(E::Terminal(e))=>Err(e.into())
        }
    }
}
pub(crate) mod sealed {
    pub(crate) trait Element{
    }
}
pub(crate) trait TransitionElement:sealed::Element{
}
impl sealed::Element for Lot{
}
impl TransitionElement for Lot{
}
impl sealed::Element for LotClaim{
}
impl TransitionElement for LotClaim{
}
impl sealed::Element for FillRecord{
}
impl TransitionElement for FillRecord{
}
impl sealed::Element for WorkingReservation{
}
impl TransitionElement for WorkingReservation{
}
impl sealed::Element for MarkedAllocation{
}
impl TransitionElement for MarkedAllocation{
}
impl sealed::Element for String{
}
impl TransitionElement for String{
}
impl<'a> sealed::Element for &'a Lot{
}
impl<'a> TransitionElement for &'a Lot{
}
#[derive(Clone, Copy)]
pub(crate) enum SeedText{
    InvalidSeedIdentityEffectiveTimeOrPolicy,
    InvalidSeedMark,
    InvalidSeedLot,
    SeedLotMarkMissing,
    ExplicitSellabilityLacksEvidence,
    SeedMarkCoverageMismatch,
    UnapprovedSeedResidualEmptyEquity
}
impl SeedText{
    fn payload(self)->(&'static str, ErrorCategory){
        match self{
            Self::InvalidSeedIdentityEffectiveTimeOrPolicy=>("invalid seed identity, effective time or policy", ErrorCategory::Invalid),
            Self::InvalidSeedMark=>("invalid seed mark", ErrorCategory::Invalid),
            Self::InvalidSeedLot=>("invalid seed lot", ErrorCategory::Invalid),
            Self::SeedLotMarkMissing=>("seed lot mark missing", ErrorCategory::Evidence),
            Self::ExplicitSellabilityLacksEvidence=>("explicit sellability lacks evidence", ErrorCategory::Invalid),
            Self::SeedMarkCoverageMismatch=>("seed mark coverage mismatch", ErrorCategory::Invalid),
            Self::UnapprovedSeedResidualEmptyEquity=>("unapproved seed residual/empty equity", ErrorCategory::Invalid),
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) enum V1Text{
    InvalidDuplicateExtraneousValuationMark,
    IncompleteWholeAccountValuation,
    SecondGenesis,
    MarkInventoryMismatch,
    FIFOBeforeLotMismatch,
    DuplicateLotIdentity,
    NegativePaperCash,
    NonfillHasFinancialEffects
}
impl V1Text{
    fn payload(self)->(&'static str, ErrorCategory){
        match self{
            Self::InvalidDuplicateExtraneousValuationMark=>("invalid/duplicate/extraneous valuation mark", ErrorCategory::Evidence),
            Self::IncompleteWholeAccountValuation=>("incomplete whole-account valuation", ErrorCategory::Evidence),
            Self::SecondGenesis=>("second genesis", ErrorCategory::Integrity),
            Self::MarkInventoryMismatch=>("mark inventory mismatch", ErrorCategory::Integrity),
            Self::FIFOBeforeLotMismatch=>("FIFO before-lot mismatch", ErrorCategory::Integrity),
            Self::DuplicateLotIdentity=>("duplicate lot identity", ErrorCategory::Integrity),
            Self::NegativePaperCash=>("negative paper cash", ErrorCategory::Integrity),
            Self::NonfillHasFinancialEffects=>("nonfill has financial effects", ErrorCategory::Integrity),
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) enum BookText{
    GenesisAccountMismatch,
    V1SourceAnchorMismatch,
    FeeInstanceMismatch,
    ManifestFieldsMismatch,
    ManifestHashMismatch,
    GenesisEventMismatch,
    GenesisHashMismatch,
    V2ProjectionMismatch
}
impl BookText{
    fn payload(self)->(&'static str, ErrorCategory){
        match self{
            Self::GenesisAccountMismatch=>("V2 book genesis account mismatch", ErrorCategory::Integrity),
            Self::V1SourceAnchorMismatch=>("V2 book V1 source anchor mismatch", ErrorCategory::Integrity),
            Self::FeeInstanceMismatch=>("V2 book fee instance mismatch", ErrorCategory::Integrity),
            Self::ManifestFieldsMismatch=>("V2 book manifest fields mismatch", ErrorCategory::Integrity),
            Self::ManifestHashMismatch=>("V2 book manifest hash mismatch", ErrorCategory::Integrity),
            Self::GenesisEventMismatch=>("V2 book genesis event mismatch", ErrorCategory::Integrity),
            Self::GenesisHashMismatch=>("V2 book genesis hash mismatch", ErrorCategory::Integrity),
            Self::V2ProjectionMismatch=>("V2 book V2 projection mismatch", ErrorCategory::Integrity),
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) enum ExecutionText{
    PaperExecutionShanghaiClockExceedsSupportedRange,
    ExecutionManifestInvalid,
    FeeDescriptorUTF8,
    FeeDescriptorField,
    DuplicateFeeDescriptorField,
    FeeDescriptorMissingField,
    FeeDescriptorInteger,
    FeeSegmentUnavailable,
    FeeCoverageField,
    FeeDescriptorIsNotCanonicalReviewedPolicy,
    AccountCashDiffersFromExecutionPartition,
    RecordedValuationWindowDiffersFromOriginalMark,
    FullLotDispositionsDiffer,
    ParentQuantityDiffers,
    WorkingParentRemainderInvalid,
    ReservationOwnerDiffers,
    ReservationComponentsDiffer,
    TerminalParentRetainsReservation,
    WorkingReservationExceedsStrategyCash,
    ClaimReferencesAbsentLot,
    SellClaimsOverbookLot,
    AllocatedHoldingMarkAbsent,
    AllocatedHoldingMarkIsNotCurrentSession,
    AllocatedHoldingQualifiedValuationWindowAbsent,
    AllocatedHoldingQualifiedValuationWindowExpiredOrDiffers,
    RecordedAdmittedBoardDiffersFromFeeScope,
    SubmitManifestOwnerDiffers,
    BudgetPolicySessionNotEffective,
    ParentObservationPrecedesPriorFinancialFact,
    StrategyCashCannotReserveSellFees,
    AllocatedFIFOSellableSharesUnavailableOrAlreadyReserved,
    FillObservationPrecedesPriorFinancialFact,
    WindowDoesNotMatchWorkingDayParent,
    DuplicateFillLot,
    ReservedSellLotDisappeared,
    SellLotIsNotAssignedSellable,
    FillExceedsReservedFIFOShares,
    CancelExpireNotCurrentWorkingOrder,
    DayOrderNotYetExpiredOnVerifiedSession,
    QualifiedMarkSetEmpty,
    MarkPrecedesPriorFinancialFact,
    MarkSetDateCodeDuplicate,
    MarksOmitFullAccountHolding,
    ExecutionRecordExceedsByteLimit
}
impl ExecutionText{
    fn payload(self)->(&'static str, ErrorCategory){
        match self{
            Self::PaperExecutionShanghaiClockExceedsSupportedRange=>("paper execution Shanghai clock exceeds supported range", ErrorCategory::Evidence),
            Self::ExecutionManifestInvalid=>("execution manifest invalid", ErrorCategory::Integrity),
            Self::FeeDescriptorUTF8=>("fee descriptor UTF8", ErrorCategory::Integrity),
            Self::FeeDescriptorField=>("fee descriptor field", ErrorCategory::Integrity),
            Self::DuplicateFeeDescriptorField=>("duplicate fee descriptor field", ErrorCategory::Integrity),
            Self::FeeDescriptorMissingField=>("fee descriptor missing field", ErrorCategory::Integrity),
            Self::FeeDescriptorInteger=>("fee descriptor integer", ErrorCategory::Integrity),
            Self::FeeSegmentUnavailable=>("fee segment unavailable", ErrorCategory::Evidence),
            Self::FeeCoverageField=>("fee coverage field", ErrorCategory::Integrity),
            Self::FeeDescriptorIsNotCanonicalReviewedPolicy=>("fee descriptor is not canonical reviewed policy", ErrorCategory::Integrity),
            Self::AccountCashDiffersFromExecutionPartition=>("account cash differs from execution partition", ErrorCategory::Integrity),
            Self::RecordedValuationWindowDiffersFromOriginalMark=>("recorded valuation window differs from original mark", ErrorCategory::Integrity),
            Self::FullLotDispositionsDiffer=>("full lot dispositions differ", ErrorCategory::Integrity),
            Self::ParentQuantityDiffers=>("parent quantity differs", ErrorCategory::Integrity),
            Self::WorkingParentRemainderInvalid=>("working parent remainder invalid", ErrorCategory::Integrity),
            Self::ReservationOwnerDiffers=>("reservation owner differs", ErrorCategory::Integrity),
            Self::ReservationComponentsDiffer=>("reservation components differ", ErrorCategory::Integrity),
            Self::TerminalParentRetainsReservation=>("terminal parent retains reservation", ErrorCategory::Integrity),
            Self::WorkingReservationExceedsStrategyCash=>("working reservation exceeds strategy cash", ErrorCategory::Integrity),
            Self::ClaimReferencesAbsentLot=>("claim references absent lot", ErrorCategory::Integrity),
            Self::SellClaimsOverbookLot=>("sell claims overbook lot", ErrorCategory::Integrity),
            Self::AllocatedHoldingMarkAbsent=>("allocated holding mark absent", ErrorCategory::Evidence),
            Self::AllocatedHoldingMarkIsNotCurrentSession=>("allocated holding mark is not current session", ErrorCategory::Integrity),
            Self::AllocatedHoldingQualifiedValuationWindowAbsent=>("allocated holding qualified valuation window absent", ErrorCategory::Evidence),
            Self::AllocatedHoldingQualifiedValuationWindowExpiredOrDiffers=>("allocated holding qualified valuation window expired or differs", ErrorCategory::Integrity),
            Self::RecordedAdmittedBoardDiffersFromFeeScope=>("recorded admitted board differs from fee scope", ErrorCategory::Integrity),
            Self::SubmitManifestOwnerDiffers=>("submit manifest owner differs", ErrorCategory::Integrity),
            Self::BudgetPolicySessionNotEffective=>("budget policy session not effective", ErrorCategory::Integrity),
            Self::ParentObservationPrecedesPriorFinancialFact=>("parent observation precedes prior financial fact", ErrorCategory::Integrity),
            Self::StrategyCashCannotReserveSellFees=>("strategy cash cannot reserve sell fees", ErrorCategory::Integrity),
            Self::AllocatedFIFOSellableSharesUnavailableOrAlreadyReserved=>("allocated FIFO sellable shares unavailable or already reserved", ErrorCategory::Integrity),
            Self::FillObservationPrecedesPriorFinancialFact=>("fill observation precedes prior financial fact", ErrorCategory::Integrity),
            Self::WindowDoesNotMatchWorkingDayParent=>("window does not match working day parent", ErrorCategory::Integrity),
            Self::DuplicateFillLot=>("duplicate fill lot", ErrorCategory::Integrity),
            Self::ReservedSellLotDisappeared=>("reserved sell lot disappeared", ErrorCategory::Integrity),
            Self::SellLotIsNotAssignedSellable=>("sell lot is not assigned/sellable", ErrorCategory::Integrity),
            Self::FillExceedsReservedFIFOShares=>("fill exceeds reserved FIFO shares", ErrorCategory::Integrity),
            Self::CancelExpireNotCurrentWorkingOrder=>("cancel/expire not current working order", ErrorCategory::Integrity),
            Self::DayOrderNotYetExpiredOnVerifiedSession=>("day order not yet expired on verified session", ErrorCategory::Integrity),
            Self::QualifiedMarkSetEmpty=>("qualified mark set empty", ErrorCategory::Integrity),
            Self::MarkPrecedesPriorFinancialFact=>("mark precedes prior financial fact", ErrorCategory::Integrity),
            Self::MarkSetDateCodeDuplicate=>("mark set date/code duplicate", ErrorCategory::Integrity),
            Self::MarksOmitFullAccountHolding=>("marks omit full account holding", ErrorCategory::Integrity),
            Self::ExecutionRecordExceedsByteLimit=>("execution record exceeds byte limit", ErrorCategory::Integrity),
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) enum BudgetText{
    NonpositivePriceOrQuantity,
    ExecutionCashPartitionsDiffer,
    DescriptorIsInvalid,
    CompleteOrderedLotAllocationIsInvalid,
    AllocationDoesNotMatchGenesis,
    DuplicateGenesisLot,
    UnknownGenesisLot,
    GenesisQuantityChanged,
    GenesisMarkAbsent,
    InitialAllocatedCapitalExceedsFixedBudget,
    MarkedAllocationIsInvalid,
    WorkingReservationIsInvalid,
    NewBuyExceedsFixedAllInOrCashLimits
}
impl BudgetText{
    fn payload(self)->(&'static str, ErrorCategory){
        match self{
            Self::NonpositivePriceOrQuantity=>("parent budget nonpositive price or quantity", ErrorCategory::Invalid),
            Self::ExecutionCashPartitionsDiffer=>("execution cash partitions differ", ErrorCategory::Integrity),
            Self::DescriptorIsInvalid=>("parent budget descriptor is invalid", ErrorCategory::Invalid),
            Self::CompleteOrderedLotAllocationIsInvalid=>("parent budget complete ordered lot allocation is invalid", ErrorCategory::Invalid),
            Self::AllocationDoesNotMatchGenesis=>("parent budget allocation does not match genesis", ErrorCategory::Invalid),
            Self::DuplicateGenesisLot=>("parent budget duplicate genesis lot", ErrorCategory::Invalid),
            Self::UnknownGenesisLot=>("parent budget unknown genesis lot", ErrorCategory::Invalid),
            Self::GenesisQuantityChanged=>("parent budget genesis quantity changed", ErrorCategory::Invalid),
            Self::GenesisMarkAbsent=>("parent budget genesis mark absent", ErrorCategory::Invalid),
            Self::InitialAllocatedCapitalExceedsFixedBudget=>("parent budget initial allocated capital exceeds fixed budget", ErrorCategory::Invalid),
            Self::MarkedAllocationIsInvalid=>("parent budget marked allocation is invalid", ErrorCategory::Invalid),
            Self::WorkingReservationIsInvalid=>("parent budget working reservation is invalid", ErrorCategory::Invalid),
            Self::NewBuyExceedsFixedAllInOrCashLimits=>("parent budget new buy exceeds fixed all-in or cash limits", ErrorCategory::Invalid),
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) enum FillText{
    ExecutionWindowShanghaiClockExceedsSupportedRange,
    ExecutionWindowIsNotAdmissible,
    ParentWholeLotBoundsInvalid,
    ExecutionPriceExceedsFrozenFeeCap,
    OddLotModelUnavailable,
    FeeDescriptorUTF8Unavailable,
    FeeDescriptorRateUnavailable,
    FeeDescriptorRateDuplicated,
    FeeDescriptorRateInvalid
}
impl FillText{
    fn payload(self)->(&'static str, ErrorCategory){
        match self{
            Self::ExecutionWindowShanghaiClockExceedsSupportedRange=>("execution window Shanghai clock exceeds supported range", ErrorCategory::Evidence),
            Self::ExecutionWindowIsNotAdmissible=>("execution window is not admissible", ErrorCategory::Evidence),
            Self::ParentWholeLotBoundsInvalid=>("parent whole-lot bounds invalid", ErrorCategory::Invalid),
            Self::ExecutionPriceExceedsFrozenFeeCap=>("execution price exceeds frozen fee cap", ErrorCategory::Evidence),
            Self::OddLotModelUnavailable=>("odd lot model unavailable", ErrorCategory::Evidence),
            Self::FeeDescriptorUTF8Unavailable=>("fee descriptor UTF8 unavailable", ErrorCategory::Evidence),
            Self::FeeDescriptorRateUnavailable=>("fee descriptor rate unavailable", ErrorCategory::Evidence),
            Self::FeeDescriptorRateDuplicated=>("fee descriptor rate duplicated", ErrorCategory::Evidence),
            Self::FeeDescriptorRateInvalid=>("fee descriptor rate invalid", ErrorCategory::Evidence),
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) enum IntentText{
    ClosedParentIntentBindingInvalid
}
impl IntentText{
    fn payload(self)->(&'static str, ErrorCategory){
        match self{
            Self::ClosedParentIntentBindingInvalid=>("closed parent intent binding invalid", ErrorCategory::Invalid),
        }
    }
}
#[derive(Clone, Copy)]
enum ErrorCategory{
    Invalid,
    Integrity,
    Evidence
}
pub(crate) struct FeeDescriptorDigest(String);
pub(crate) enum ClosedFinancialText<'a>{
    Seed(SeedText),
    V1(V1Text),
    Book(BookText),
    Execution(ExecutionText),
    Budget(BudgetText),
    Fill(FillText),
    Intent(IntentText),
    FeeError(&'a AShareFeeV2Error),
    MissingMark(&'a Lot),
    SeedLotOrdinal(usize),
    EconomicUnavailable(&'a Projection),
    ProjectionVersion,
    CutoverSchema,
    GenesisSchema,
    FeeInstance(&'a FeeDescriptorDigest),
    StampBracket(StampTaxBracketV2)
}
impl ClosedFinancialText<'_>{
    fn category(&self)->ErrorCategory{
        match self{
            Self::Seed(r)=>r.payload().1,
            Self::V1(r)=>r.payload().1,
            Self::Book(r)=>r.payload().1,
            Self::Execution(r)=>r.payload().1,
            Self::Budget(r)=>r.payload().1,
            Self::Fill(r)=>r.payload().1,
            Self::Intent(r)=>r.payload().1,
            _=>ErrorCategory::Evidence
        }
    }
    pub(crate) fn write(&self, out:&mut FinancialSink<'_>)->std::result::Result<(),
    ()>{
        match self{
            Self::Seed(r)=>out.bytes(r.payload().0.as_bytes()),
            Self::V1(r)=>out.bytes(r.payload().0.as_bytes()),
            Self::Book(r)=>out.bytes(r.payload().0.as_bytes()),
            Self::Execution(r)=>out.bytes(r.payload().0.as_bytes()),
            Self::Budget(r)=>out.bytes(r.payload().0.as_bytes()),
            Self::Fill(r)=>out.bytes(r.payload().0.as_bytes()),
            Self::Intent(r)=>out.bytes(r.payload().0.as_bytes()),
            Self::FeeError(e)=>out.bytes(fee_error_text(e).as_bytes()),
            Self::MissingMark(lot)=>{
                out.bytes(b"missing mark ")?;
                out.bytes(lot.code.as_bytes())
            },
            Self::SeedLotOrdinal(i)=>{
                out.bytes(b"seed:")?;
                out.unsigned(*i as u64)
            },
            Self::EconomicUnavailable(p)=>out.bytes(p.replay_unavailable_reason().unwrap_or("").as_bytes()),
            Self::ProjectionVersion=>out.bytes(b"paper-parent-projection/v1"),
            Self::CutoverSchema=>out.bytes(b"paper-book-v2-cutover-manifest/v1"),
            Self::GenesisSchema=>out.bytes(b"paper-book-v2-genesis/v1"),
            Self::FeeInstance(d)=>{
                out.bytes(crate::performance::fee_evidence::A_SHARE_FEE_SCHEDULE_V2.as_bytes())?;
                out.bytes(b":sha256:")?;
                out.bytes(d.0.as_bytes())
            },
            Self::StampBracket(b)=>out.bytes(b.id().as_bytes())
        }
    }
}
fn fee_error_text(e:&AShareFeeV2Error)->&'static str{
    match e{
        AShareFeeV2Error::UnsupportedInstrument=>"Shanghai A-share stock fee policy does not support this instrument",
        AShareFeeV2Error::ScopeMismatch=>"fee instrument scope does not match policy scope",
        AShareFeeV2Error::UnsupportedCoverage=>"complete trading cost is unavailable: transfer and other charges are excluded",
        AShareFeeV2Error::InvalidNotional=>"A-share fee schedule v2 requires positive micro-CNY notional",
        AShareFeeV2Error::UnsupportedTradeDate=>"A-share fee schedule v2 has no authority before 2008-09-19",
        AShareFeeV2Error::InvalidCommissionRate=>"commission rate must be nonnegative with a positive denominator",
        AShareFeeV2Error::InvalidCommissionMinimum=>"commission minimum must be nonnegative",
        AShareFeeV2Error::InvalidSourceRevision=>"source revision must be 1..128 ASCII token characters",
        AShareFeeV2Error::Overflow=>"A-share fee schedule v2 amount overflow",
    }
}
pub(crate) enum FinancialOutput<'a>{
    Text(ClosedFinancialText<'a>),
    FeeDescriptor(&'a AShareFeePolicyV2)
}
impl FinancialOutput<'_>{
    pub(crate) fn write(&self, out:&mut FinancialSink<'_>)->std::result::Result<(),
    ()>{
        match self{
            Self::Text(t)=>t.write(out),
            Self::FeeDescriptor(p)=>p.write_replay_descriptor(out)
        }
    }
}
// Concrete sinks are used only by the closed writers above and the fee owner.
pub(crate) enum FinancialSink<'a>{
    Count(usize),
    Output{
        bytes:&'a mut Vec<u8>,
        limit:usize
    },
    Owned(Vec<u8>)
}
impl FinancialSink<'_>{
    pub(crate) fn bytes(&mut self, b:&[u8])->std::result::Result<(),
    ()>{
        match self{
            Self::Count(n)=>{
                *n=n.checked_add(b.len()).ok_or(())?;
            },
            Self::Output{
                bytes,
                limit
            }
            =>{
                if b.len()>limit.saturating_sub(bytes.len()){
                    return Err(());
                }
                bytes.extend_from_slice(b);
            },
            Self::Owned(v)=>v.extend_from_slice(b)
        }
        Ok(())
    }
    pub(crate) fn unsigned(&mut self, mut n:u64)->std::result::Result<(),
    ()>{
        let mut b=[0u8; 20];
        let mut i=20;
        loop{
            i-=1;
            b[i]=b'0'+(n%10)as u8;
            n/=10;
            if n==0{
                break;
            }
        }
        self.bytes(&b[i..])
    }
    pub(crate) fn signed(&mut self, n:i64)->std::result::Result<(),
    ()>{
        if n<0{
            self.bytes(b"-")?;
        }
        self.unsigned(n.unsigned_abs())
    }
    pub(crate) fn count(&self)->usize{
        match self{
            Self::Count(n)=>*n,
            _=>unreachable!()
        }
    }
    fn into_owned(self)->Vec<u8>{
        match self{
            Self::Owned(v)=>v,
            _=>unreachable!()
        }
    }
}
pub(crate) enum ClosedFinancialHash<'a>{
    Inventory(&'a [Lot]),
    ExecutionManifest(&'a ExecutionManifest),
    ExecutionWindow(&'a WindowRecord),
    FillIdentity{
        account:&'a str,
        parent:&'a str,
        observation:&'a str
    },
    Genesis{
        account:&'a str,
        command:&'a str,
        previous:&'a str,
        payload:&'a[u8]
    }
}
impl ClosedFinancialHash<'_>{
    fn execution(&self)->bool{
        matches!(self, Self::ExecutionManifest(_)|Self::ExecutionWindow(_)|Self::FillIdentity{
            ..
        })
    }
    fn domain(&self)->Option<&'static[u8]>{
        match self{
            Self::Inventory(_)=>None,
            Self::ExecutionManifest(_)=>Some(b"paper-parent-execution-manifest/v1"),
            Self::ExecutionWindow(_)=>Some(b"paper-execution-window/v1"),
            Self::FillIdentity{
                ..
            }
            =>Some(b"paper-parent-fill-id/v1"),
            Self::Genesis{
                ..
            }
            =>Some(b"paper-book-v2-genesis-event/v1")
        }
    }
    fn historical_json(&self)->std::result::Result<Vec<u8>,
    LedgerError>{
        let r=match self{
            Self::Inventory(v)=>serde_json::to_vec(v),
            Self::ExecutionManifest(v)=>serde_json::to_vec(v),
            Self::ExecutionWindow(v)=>serde_json::to_vec(v),
            Self::FillIdentity{
                account,
                parent,
                observation
            }
            =>serde_json::to_vec(&(account, parent, observation)),
            Self::Genesis{
                account,
                command,
                previous,
                payload
            }
            =>serde_json::to_vec(&(account, 1_i64, command, previous, payload))
        };
        r.map_err(|e|LedgerError::IntegrityFailure(e.to_string()))
    }
    fn write_json(&self, out:&mut JsonSink<'_>)->serde_json::Result<()>{
        match self{
            Self::Inventory(v)=>serde_json::to_writer(out, v),
            Self::ExecutionManifest(v)=>serde_json::to_writer(out, v),
            Self::ExecutionWindow(v)=>serde_json::to_writer(out, v),
            Self::FillIdentity{
                account,
                parent,
                observation
            }
            =>serde_json::to_writer(out, &(account, parent, observation)),
            Self::Genesis{
                account,
                command,
                previous,
                payload
            }
            =>serde_json::to_writer(out, &(account, 1_i64, command, previous, payload))
        }
    }
}
enum JsonSink<'a>{
    Count(usize),
    Output{
        bytes:&'a mut Vec<u8>,
        limit:usize
    }
}
impl JsonSink<'_>{
    fn count(&self)->usize{
        match self{
            Self::Count(n)=>*n,
            _=>unreachable!()
        }
    }
}
impl std::io::Write for JsonSink<'_>{
    fn write(&mut self, b:&[u8])->std::io::Result<usize>{
        match self{
            Self::Count(n)=>{
                *n=n.checked_add(b.len()).ok_or(std::io::ErrorKind::OutOfMemory)?;
            },
            Self::Output{
                bytes,
                limit
            }
            =>{
                if b.len()>limit.saturating_sub(bytes.len()){
                    return Err(std::io::ErrorKind::OutOfMemory.into());
                }
                bytes.extend_from_slice(b);
            }
        }
        Ok(b.len())
    }
    fn flush(&mut self)->std::io::Result<()>{
        Ok(())
    }
}
#[cfg(test)]
impl FinancialWork<'_, '_>{
    pub(crate) fn used(&self)->u64{
        match self{
            Self::Fixture(m)=>m.used(),
            _=>panic!("lower usage only")
        }
    }
}
#[cfg(test)]
enum HashAction{
    Output,
    Guard,
    PayloadHash,
    Hex
}
#[cfg(test)]
impl FinancialWork<'_, '_>{
    fn note_hash(&mut self, action:HashAction){
        if let Self::Fixture(m)=self{
            m.hash_hits[match action{
                HashAction::Output=>0,
                HashAction::Guard=>1,
                HashAction::PayloadHash=>2,
                HashAction::Hex=>3
            } ]+=1;
        }
    }
    pub(crate) fn hash_hits(&self)->[usize; 4]{
        match self{
            Self::Fixture(m)=>m.hash_hits,
            _=>[0; 4]
        }
    }
}
#[cfg(test)]
impl FinancialWork<'_, '_> {
    pub(crate) fn operation_entries(&self) -> [usize; 11] {
        match self {
            Self::Fixture(loan) => loan.entries(),
            _ => panic!("fixed fixture only")
        }
    }
}
#[cfg(test)]
impl FinancialWork<'_, '_> {
    pub(crate) fn assert_fixture_resource(&self, expected_used: u64) {
        match self {
            Self::Fixture(loan) => loan.assert_resource(expected_used),
            _ => panic!("fixed fixture only")
        }
    }
    pub(crate) fn expected_boundary_cost(&self) -> u64 {
        match self {
            Self::Fixture(loan) => loan.expected_boundary_cost(),
            _ => panic!("fixed fixture only")
        }
    }
}
