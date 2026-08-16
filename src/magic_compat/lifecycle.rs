//! UnverifiedSourceUnit / CorporateActionCategory / CorporateActionStatus /
//! CorporateActionTerms 本地镜像 (M5, Task #76, feature 关时使用)。
//! 与上游 magic_market_core rev 75ee2a2 (lifecycle.rs) 同构:
//! 变体集/字段/serde 表示/校验语义与错误字符串逐字一致
//! (convert.rs 用 from_value 反序列化 Terms, 校验必须生效)。

#[cfg(not(feature = "magic-gateway"))]
use serde::{de, Deserialize, Deserializer, Serialize};

#[cfg(not(feature = "magic-gateway"))]
use super::instrument::CoreError;
#[cfg(not(feature = "magic-gateway"))]
use super::value::{FiniteNumber, Price, Ratio, RatioUnit};

/// A provider-native quantity whose physical unit has not been independently verified.
#[cfg(not(feature = "magic-gateway"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UnverifiedSourceUnit {
    ProviderNative,
}

/// Source-published lifecycle state.
#[cfg(not(feature = "magic-gateway"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CorporateActionStatus {
    Implemented,
    Proposed,
    Cancelled,
    Unknown,
}

/// Corporate action family.
#[cfg(not(feature = "magic-gateway"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CorporateActionCategory {
    Distribution,
    BonusRightsListing,
    NonTradableShareListing,
    UnknownCapitalChange,
    CapitalChange,
    AdditionalIssuance,
    ShareRepurchase,
    AdditionalIssuanceListing,
    TransferredAllotmentListing,
    ConvertibleBondListing,
    CapitalRescaling,
    NonTradableReverseSplit,
    SubscriptionWarrantGrant,
    PutWarrantGrant,
}

/// Checked economic terms for one corporate action.
#[cfg(not(feature = "magic-gateway"))]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum CorporateActionTerms {
    Distribution {
        cash_per_share: Option<FiniteNumber>,
        bonus_per_share: Option<FiniteNumber>,
        rights_per_share: Option<FiniteNumber>,
        rights_price: Option<Price>,
    },
    CapitalRescaling {
        ratio: Ratio,
    },
    NonTradableReverseSplit {
        ratio: Ratio,
    },
    ProviderNativeRatio {
        category: CorporateActionCategory,
        source_ratio: FiniteNumber,
        source_ratio_unit: UnverifiedSourceUnit,
    },
    CapitalStructure {
        category: CorporateActionCategory,
        tradable_before: FiniteNumber,
        tradable_after: FiniteNumber,
        total_before: FiniteNumber,
        total_after: FiniteNumber,
        unit: UnverifiedSourceUnit,
    },
    WarrantGrant {
        category: CorporateActionCategory,
        exercise_price: Price,
        source_quantity: FiniteNumber,
        source_quantity_unit: UnverifiedSourceUnit,
    },
}

#[cfg(not(feature = "magic-gateway"))]
impl CorporateActionTerms {
    pub fn distribution(
        cash_per_share: Option<FiniteNumber>,
        bonus_per_share: Option<FiniteNumber>,
        rights_per_share: Option<FiniteNumber>,
        rights_price: Option<Price>,
    ) -> Result<Self, CoreError> {
        for (field, value) in [
            ("cash_per_share", cash_per_share),
            ("bonus_per_share", bonus_per_share),
            ("rights_per_share", rights_per_share),
        ] {
            if let Some(value) = value {
                if value.get() < 0.0 {
                    return Err(CoreError::InvalidValue {
                        field,
                        value: value.get().to_string(),
                        reason: "must be non-negative",
                    });
                }
            }
        }
        if ![cash_per_share, bonus_per_share, rights_per_share]
            .into_iter()
            .flatten()
            .any(|value| value.get() > 0.0)
        {
            return Err(CoreError::InvalidRequest(
                "distribution requires at least one positive per-share term".into(),
            ));
        }
        if rights_price.is_some() && !rights_per_share.is_some_and(|quantity| quantity.get() > 0.0)
        {
            return Err(CoreError::InvalidRequest(
                "rights price requires a positive rights-per-share quantity".into(),
            ));
        }
        Ok(Self::Distribution {
            cash_per_share,
            bonus_per_share,
            rights_per_share,
            rights_price,
        })
    }

    pub fn capital_rescaling(
        category: CorporateActionCategory,
        ratio: Ratio,
    ) -> Result<Self, CoreError> {
        if ratio.unit() != RatioUnit::Decimal || ratio.get() <= 0.0 || ratio.get() == 1.0 {
            return Err(CoreError::InvalidValue {
                field: "corporate_action_ratio",
                value: ratio.get().to_string(),
                reason: "must be a positive non-identity decimal ratio",
            });
        }
        match category {
            CorporateActionCategory::CapitalRescaling => Ok(Self::CapitalRescaling { ratio }),
            CorporateActionCategory::NonTradableReverseSplit => {
                Ok(Self::NonTradableReverseSplit { ratio })
            }
            _ => Err(CoreError::InvalidRequest(
                "only split categories use split terms".into(),
            )),
        }
    }

    /// Preserves a provider-native ratio whose physical meaning or scale is not verified.
    pub fn provider_native_ratio(
        category: CorporateActionCategory,
        source_ratio: FiniteNumber,
        source_ratio_unit: UnverifiedSourceUnit,
    ) -> Result<Self, CoreError> {
        if !matches!(
            category,
            CorporateActionCategory::CapitalRescaling
                | CorporateActionCategory::NonTradableReverseSplit
        ) {
            return Err(CoreError::InvalidRequest(
                "category does not use provider-native ratio terms".into(),
            ));
        }
        if source_ratio.get() <= 0.0 {
            return Err(CoreError::InvalidValue {
                field: "source_ratio",
                value: source_ratio.get().to_string(),
                reason: "must be positive",
            });
        }
        Ok(Self::ProviderNativeRatio {
            category,
            source_ratio,
            source_ratio_unit,
        })
    }

    pub fn capital_structure(
        category: CorporateActionCategory,
        tradable_before: FiniteNumber,
        tradable_after: FiniteNumber,
        total_before: FiniteNumber,
        total_after: FiniteNumber,
        unit: UnverifiedSourceUnit,
    ) -> Result<Self, CoreError> {
        if !matches!(
            category,
            CorporateActionCategory::BonusRightsListing
                | CorporateActionCategory::NonTradableShareListing
                | CorporateActionCategory::UnknownCapitalChange
                | CorporateActionCategory::CapitalChange
                | CorporateActionCategory::AdditionalIssuance
                | CorporateActionCategory::ShareRepurchase
                | CorporateActionCategory::AdditionalIssuanceListing
                | CorporateActionCategory::TransferredAllotmentListing
                | CorporateActionCategory::ConvertibleBondListing
        ) {
            return Err(CoreError::InvalidRequest(
                "category does not use capital-structure terms".into(),
            ));
        }
        for (field, value) in [
            ("tradable_before", tradable_before),
            ("tradable_after", tradable_after),
            ("total_before", total_before),
            ("total_after", total_after),
        ] {
            if value.get() < 0.0 {
                return Err(CoreError::InvalidValue {
                    field,
                    value: value.get().to_string(),
                    reason: "must be non-negative",
                });
            }
        }
        Ok(Self::CapitalStructure {
            category,
            tradable_before,
            tradable_after,
            total_before,
            total_after,
            unit,
        })
    }

    pub fn warrant_grant(
        category: CorporateActionCategory,
        exercise_price: Price,
        source_quantity: FiniteNumber,
        source_quantity_unit: UnverifiedSourceUnit,
    ) -> Result<Self, CoreError> {
        if !matches!(
            category,
            CorporateActionCategory::SubscriptionWarrantGrant
                | CorporateActionCategory::PutWarrantGrant
        ) {
            return Err(CoreError::InvalidRequest(
                "category does not use warrant-grant terms".into(),
            ));
        }
        if source_quantity.get() <= 0.0 {
            return Err(CoreError::InvalidValue {
                field: "source_quantity",
                value: source_quantity.get().to_string(),
                reason: "must be positive",
            });
        }
        Ok(Self::WarrantGrant {
            category,
            exercise_price,
            source_quantity,
            source_quantity_unit,
        })
    }
}

#[cfg(not(feature = "magic-gateway"))]
impl<'de> Deserialize<'de> for CorporateActionTerms {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        enum Wire {
            Distribution {
                cash_per_share: Option<FiniteNumber>,
                bonus_per_share: Option<FiniteNumber>,
                rights_per_share: Option<FiniteNumber>,
                rights_price: Option<Price>,
            },
            CapitalRescaling {
                ratio: Ratio,
            },
            NonTradableReverseSplit {
                ratio: Ratio,
            },
            ProviderNativeRatio {
                category: CorporateActionCategory,
                source_ratio: FiniteNumber,
                source_ratio_unit: UnverifiedSourceUnit,
            },
            CapitalStructure {
                category: CorporateActionCategory,
                tradable_before: FiniteNumber,
                tradable_after: FiniteNumber,
                total_before: FiniteNumber,
                total_after: FiniteNumber,
                unit: UnverifiedSourceUnit,
            },
            WarrantGrant {
                category: CorporateActionCategory,
                exercise_price: Price,
                source_quantity: FiniteNumber,
                source_quantity_unit: UnverifiedSourceUnit,
            },
        }

        match Wire::deserialize(deserializer)? {
            Wire::Distribution {
                cash_per_share,
                bonus_per_share,
                rights_per_share,
                rights_price,
            } => Self::distribution(
                cash_per_share,
                bonus_per_share,
                rights_per_share,
                rights_price,
            ),
            Wire::CapitalRescaling { ratio } => {
                Self::capital_rescaling(CorporateActionCategory::CapitalRescaling, ratio)
            }
            Wire::NonTradableReverseSplit { ratio } => {
                Self::capital_rescaling(CorporateActionCategory::NonTradableReverseSplit, ratio)
            }
            Wire::ProviderNativeRatio {
                category,
                source_ratio,
                source_ratio_unit,
            } => Self::provider_native_ratio(category, source_ratio, source_ratio_unit),
            Wire::CapitalStructure {
                category,
                tradable_before,
                tradable_after,
                total_before,
                total_after,
                unit,
            } => Self::capital_structure(
                category,
                tradable_before,
                tradable_after,
                total_before,
                total_after,
                unit,
            ),
            Wire::WarrantGrant {
                category,
                exercise_price,
                source_quantity,
                source_quantity_unit,
            } => Self::warrant_grant(
                category,
                exercise_price,
                source_quantity,
                source_quantity_unit,
            ),
        }
        .map_err(de::Error::custom)
    }
}
