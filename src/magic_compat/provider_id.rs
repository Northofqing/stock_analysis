//! ProviderId 本地镜像 (M5, Task #76, feature 关时使用)。
//!
//! 与上游 magic_market_core (pin rev 75ee2a2, crates/magic-market-core/
//! src/provider.rs) 同构: 32 个 unit variant, 同序同 derive
//! (Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize),
//! 无 serde rename — 序列化 = 变体名, Debug = 变体名 (wire 契约)。

#[cfg(not(feature = "magic-gateway"))]
use serde::{Deserialize, Serialize};

#[cfg(not(feature = "magic-gateway"))]
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
