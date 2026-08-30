//! Provider identity used by locally owned market-domain records.
//!
//! Variant names are part of the JSON and debug-name wire contract.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderId {
    Tdx,
    Tencent,
    Eastmoney,
    Sina,
    Baostock,
    Baidu,
    Tonghuashun,
    Iwencai,
    Cninfo,
    Cailianpress,
    Jin10,
    ThePaper,
    Yonhap,
    WallstreetCn,
    Sse,
    Szse,
    Hkex,
    Cffex,
    StateCouncil,
    Nbs,
    Pbc,
    Cfets,
    Fred,
    Imf,
    WorldBank,
    SecEdgar,
    XinhuaFinance,
    Yicai,
    SecuritiesTimes,
    LocalAnalysis,
    /// Read-only data exposed by an authorized local terminal/SDK.
    LocalTerminal,
    Custom,
}
