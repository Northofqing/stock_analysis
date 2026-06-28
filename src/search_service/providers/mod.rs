//! 搜索引擎 provider 实现集合
//!
//! 原 `search_service.rs` 中所有 provider 按引擎拆分。

pub mod bocha;
pub mod cls;
pub mod cninfo;
pub mod eastmoney;
pub mod jin10;
pub mod kcb_daily;
pub mod serpapi;
pub mod sse_szse;
pub mod tavily;
pub mod wallstreetcn;

pub use bocha::BochaSearchProvider;
pub use cls::ClsProvider;
pub use cninfo::CninfoProvider;
pub use eastmoney::EastmoneyNewsProvider;
pub use jin10::{Jin10CalendarEvent, Jin10Provider};
pub use kcb_daily::KcbDailyProvider;
pub use serpapi::SerpAPISearchProvider;
pub use sse_szse::SseSzseProvider;
pub use tavily::TavilySearchProvider;
pub use wallstreetcn::WallStreetCnProvider;
