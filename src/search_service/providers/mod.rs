//! 搜索引擎 provider 实现集合
//!
//! 原 `search_service.rs` 中所有 provider 按引擎拆分。

pub mod bocha;
pub mod eastmoney;
pub mod jin10;
pub mod serpapi;
pub mod tavily;
pub mod wallstreetcn;

pub use bocha::BochaSearchProvider;
pub use eastmoney::EastmoneyNewsProvider;
pub use jin10::{Jin10CalendarEvent, Jin10Provider};
pub use serpapi::SerpAPISearchProvider;
pub use tavily::TavilySearchProvider;
pub use wallstreetcn::WallStreetCnProvider;
