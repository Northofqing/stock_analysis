//! 基金数据类型定义
//!
//! 定义基金专用的数据结构，包括基金信息、K线、行情等。

use serde::Serialize;

use super::constants::FundType;

/// 基金基本信息
///
/// 从证券列表中筛选出的基金信息。
#[derive(Debug, Clone, Serialize)]
pub struct FundInfo {
    /// 市场代码 (0=深圳, 1=上海)
    pub market: u8,
    /// 基金代码
    pub code: String,
    /// 基金名称
    pub name: String,
    /// 基金类型
    pub fund_type: FundType,
    /// 每手股数
    pub vol_unit: u32,
    /// 小数点位数
    pub decimal_point: u32,
    /// 昨收价
    pub pre_close: f64,
}

/// 基金 K线数据
///
/// 与股票 K线格式相同，但价格精度为 0.001 (3位小数)。
#[derive(Debug, Clone, Serialize)]
pub struct FundBar {
    /// 开盘价 (3位小数)
    pub open: f64,
    /// 收盘价 (3位小数)
    pub close: f64,
    /// 最高价 (3位小数)
    pub high: f64,
    /// 最低价 (3位小数)
    pub low: f64,
    /// 成交量 (股)
    pub vol: f64,
    /// 成交额 (元)
    pub amount: f64,
    /// 年份
    pub year: u32,
    /// 月份
    pub month: u32,
    /// 日
    pub day: u32,
    /// 时 (日线为 0)
    pub hour: u32,
    /// 分 (日线为 0)
    pub minute: u32,
    /// 日期时间字符串 "YYYY-MM-DD HH:MM"
    pub datetime: String,
}

/// 基金实时行情
///
/// 与股票行情格式相同，包含五档买卖盘。
#[derive(Debug, Clone, Serialize)]
pub struct FundQuote {
    /// 市场代码
    pub market: u8,
    /// 基金代码
    pub code: String,
    /// 当前价 (3位小数)
    pub price: f64,
    /// 昨收价
    pub last_close: f64,
    /// 开盘价
    pub open: f64,
    /// 最高价
    pub high: f64,
    /// 最低价
    pub low: f64,
    /// 成交量 (股)
    pub vol: f64,
    /// 成交额 (元)
    pub amount: f64,
    /// 买一价
    pub bid1: f64,
    /// 买一量
    pub bid_vol1: f64,
    /// 买二价
    pub bid2: f64,
    /// 买二量
    pub bid_vol2: f64,
    /// 买三价
    pub bid3: f64,
    /// 买三量
    pub bid_vol3: f64,
    /// 买四价
    pub bid4: f64,
    /// 买四量
    pub bid_vol4: f64,
    /// 买五价
    pub bid5: f64,
    /// 买五量
    pub bid_vol5: f64,
    /// 卖一价
    pub ask1: f64,
    /// 卖一量
    pub ask_vol1: f64,
    /// 卖二价
    pub ask2: f64,
    /// 卖二量
    pub ask_vol2: f64,
    /// 卖三价
    pub ask3: f64,
    /// 卖三量
    pub ask_vol3: f64,
    /// 卖四价
    pub ask4: f64,
    /// 卖四量
    pub ask_vol4: f64,
    /// 卖五价
    pub ask5: f64,
    /// 卖五量
    pub ask_vol5: f64,
    /// 服务器时间
    pub servertime: String,
}

/// 基金除权除息信息
///
/// 基金通常只有分红记录 (category=1)，无送配股。
#[derive(Debug, Clone, Serialize)]
pub struct FundXdXrInfo {
    /// 年份
    pub year: u32,
    /// 月份
    pub month: u32,
    /// 日
    pub day: u32,
    /// 类别 (1=除权除息, 11=扩缩股)
    pub category: u32,
    /// 分红金额
    pub fenhong: Option<f64>,
    /// 配股价
    pub peigujia: Option<f64>,
    /// 送转股
    pub songzhuangu: Option<f64>,
    /// 配股
    pub peigu: Option<f64>,
    /// 缩股
    pub suogu: Option<f64>,
}

/// 基金财务信息 (部分字段)
///
/// 基金财务数据仅包含部分有意义的字段，如总股本、每股净资产等。
#[derive(Debug, Clone, Serialize)]
pub struct FundFinanceInfo {
    /// 市场代码
    pub market: u8,
    /// 基金代码
    pub code: String,
    /// 总股本 (万份)
    pub zongguben: f64,
    /// 流通股本 (万份)
    pub liutongguben: f64,
    /// 每股净资产
    pub meigujingzichan: f64,
    /// 总资产 (可能为 0)
    pub zongzichan: f64,
    /// 净资产 (可能为 0)
    pub jingzichan: f64,
}

// ============================================================
// 转换实现
// ============================================================

impl FundBar {
    /// 从 SecurityBar 转换
    pub fn from_security_bar(bar: &crate::protocol::types::SecurityBar) -> Self {
        Self {
            open: bar.open,
            close: bar.close,
            high: bar.high,
            low: bar.low,
            vol: bar.vol,
            amount: bar.amount,
            year: bar.year,
            month: bar.month,
            day: bar.day,
            hour: bar.hour,
            minute: bar.minute,
            datetime: bar.datetime.clone(),
        }
    }
}

impl FundQuote {
    /// 从 SecurityQuote 转换
    pub fn from_security_quote(quote: &crate::protocol::types::SecurityQuote) -> Self {
        Self {
            market: quote.market,
            code: quote.code.clone(),
            price: quote.price,
            last_close: quote.last_close,
            open: quote.open,
            high: quote.high,
            low: quote.low,
            vol: quote.vol,
            amount: quote.amount,
            bid1: quote.bid1,
            bid_vol1: quote.bid_vol1,
            bid2: quote.bid2,
            bid_vol2: quote.bid_vol2,
            bid3: quote.bid3,
            bid_vol3: quote.bid_vol3,
            bid4: quote.bid4,
            bid_vol4: quote.bid_vol4,
            bid5: quote.bid5,
            bid_vol5: quote.bid_vol5,
            ask1: quote.ask1,
            ask_vol1: quote.ask_vol1,
            ask2: quote.ask2,
            ask_vol2: quote.ask_vol2,
            ask3: quote.ask3,
            ask_vol3: quote.ask_vol3,
            ask4: quote.ask4,
            ask_vol4: quote.ask_vol4,
            ask5: quote.ask5,
            ask_vol5: quote.ask_vol5,
            servertime: quote.servertime.clone(),
        }
    }
}

impl FundXdXrInfo {
    /// 从 XdXrInfo 转换
    pub fn from_xdxr_info(info: &crate::protocol::types::XdXrInfo) -> Self {
        Self {
            year: info.year,
            month: info.month,
            day: info.day,
            category: info.category,
            fenhong: info.fenhong,
            peigujia: info.peigujia,
            songzhuangu: info.songzhuangu,
            peigu: info.peigu,
            suogu: info.suogu,
        }
    }
}

impl FundFinanceInfo {
    /// 从 FinanceInfo 转换
    pub fn from_finance_info(info: &crate::protocol::types::FinanceInfo) -> Self {
        Self {
            market: info.market,
            code: info.code.clone(),
            zongguben: info.zongguben,
            liutongguben: info.liutongguben,
            meigujingzichan: info.meigujingzichan,
            zongzichan: info.zongzichan,
            jingzichan: info.jingzichan,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fund_info_creation() {
        let info = FundInfo {
            market: 1,
            code: "510300".to_string(),
            name: "沪深300ETF".to_string(),
            fund_type: FundType::Etf,
            vol_unit: 100,
            decimal_point: 3,
            pre_close: 4.818,
        };
        assert_eq!(info.market, 1);
        assert_eq!(info.code, "510300");
        assert_eq!(info.fund_type, FundType::Etf);
    }

    #[test]
    fn test_fund_bar_creation() {
        let bar = FundBar {
            open: 4.810,
            close: 4.818,
            high: 4.846,
            low: 4.789,
            vol: 1000000.0,
            amount: 4818000.0,
            year: 2026,
            month: 6,
            day: 18,
            hour: 0,
            minute: 0,
            datetime: "2026-06-18 00:00".to_string(),
        };
        assert_eq!(bar.open, 4.810);
        assert_eq!(bar.close, 4.818);
    }

    #[test]
    fn test_fund_quote_creation() {
        let quote = FundQuote {
            market: 1,
            code: "510300".to_string(),
            price: 49.840,
            last_close: 49.580,
            open: 49.600,
            high: 49.900,
            low: 49.500,
            vol: 5000000.0,
            amount: 249200000.0,
            bid1: 49.830,
            bid_vol1: 10000.0,
            bid2: 49.820,
            bid_vol2: 8000.0,
            bid3: 49.810,
            bid_vol3: 6000.0,
            bid4: 49.800,
            bid_vol4: 4000.0,
            bid5: 49.790,
            bid_vol5: 2000.0,
            ask1: 49.840,
            ask_vol1: 12000.0,
            ask2: 49.850,
            ask_vol2: 9000.0,
            ask3: 49.860,
            ask_vol3: 7000.0,
            ask4: 49.870,
            ask_vol4: 5000.0,
            ask5: 49.880,
            ask_vol5: 3000.0,
            servertime: "15:00:00".to_string(),
        };
        assert_eq!(quote.price, 49.840);
    }

    #[test]
    fn source_records_convert_without_dropping_fund_fields() {
        let bar = crate::protocol::types::SecurityBar {
            open: 1.0,
            close: 2.0,
            high: 3.0,
            low: 0.5,
            vol: 4.0,
            amount: 5.0,
            year: 2026,
            month: 7,
            day: 25,
            hour: 15,
            minute: 0,
            datetime: "2026-07-25 15:00".into(),
        };
        let converted = FundBar::from_security_bar(&bar);
        assert_eq!(converted.datetime, bar.datetime);
        assert_eq!(converted.amount, bar.amount);

        let quote = crate::protocol::types::SecurityQuote {
            market: 1,
            code: "510300".into(),
            active1: 0,
            price: 1.0,
            last_close: 2.0,
            open: 3.0,
            high: 4.0,
            low: 5.0,
            servertime: "15:00:00".into(),
            vol: 6.0,
            cur_vol: 7.0,
            amount: 8.0,
            s_vol: 9.0,
            b_vol: 10.0,
            bid1: 11.0,
            bid_vol1: 12.0,
            bid2: 13.0,
            bid_vol2: 14.0,
            bid3: 15.0,
            bid_vol3: 16.0,
            bid4: 17.0,
            bid_vol4: 18.0,
            bid5: 19.0,
            bid_vol5: 20.0,
            ask1: 21.0,
            ask_vol1: 22.0,
            ask2: 23.0,
            ask_vol2: 24.0,
            ask3: 25.0,
            ask_vol3: 26.0,
            ask4: 27.0,
            ask_vol4: 28.0,
            ask5: 29.0,
            ask_vol5: 30.0,
            reversed_bytes0: 0,
            reversed_bytes1: 0,
            reversed_bytes2: 0,
            reversed_bytes3: 0,
            reversed_bytes4: 0,
            reversed_bytes5: 0,
            reversed_bytes6: 0,
            reversed_bytes7: 0,
            reversed_bytes8: 0,
            reversed_bytes9: 0,
            active2: 0,
        };
        let converted = FundQuote::from_security_quote(&quote);
        assert_eq!(converted.code, quote.code);
        assert_eq!(converted.ask_vol5, quote.ask_vol5);

        let action = crate::protocol::types::XdXrInfo {
            year: 2026,
            month: 7,
            day: 25,
            category: 1,
            name: "fixture".into(),
            fenhong: Some(1.0),
            peigujia: Some(2.0),
            songzhuangu: Some(3.0),
            peigu: Some(4.0),
            suogu: Some(5.0),
            panqianliutong: None,
            panhouliutong: None,
            qianzongguben: None,
            houzongguben: None,
            fenshu: None,
            xingquanjia: None,
        };
        let converted = FundXdXrInfo::from_xdxr_info(&action);
        assert_eq!(converted.fenhong, action.fenhong);
        assert_eq!(converted.suogu, action.suogu);

        let finance = crate::protocol::types::FinanceInfo {
            market: 1,
            code: "510300".into(),
            liutongguben: 1.0,
            province: 0,
            industry: 0,
            updated_date: 0,
            ipo_date: 0,
            zongguben: 2.0,
            guojiagu: 0.0,
            faqirenfarengu: 0.0,
            farengu: 0.0,
            bgu: 0.0,
            hgu: 0.0,
            zhigonggu: 0.0,
            zongzichan: 3.0,
            liudongzichan: 0.0,
            gudingzichan: 0.0,
            wuxingzichan: 0.0,
            gudongrenshu: 0.0,
            liudongfuzhai: 0.0,
            changqifuzhai: 0.0,
            zibengongjijin: 0.0,
            jingzichan: 4.0,
            zhuyingshouru: 0.0,
            zhuyinglirun: 0.0,
            yingshouzhangkuan: 0.0,
            yingyelirun: 0.0,
            touzishouyu: 0.0,
            jingyingxianjinliu: 0.0,
            zongxianjinliu: 0.0,
            cunhuo: 0.0,
            lirunzonghe: 0.0,
            shuihoulirun: 0.0,
            jinglirun: 0.0,
            weifenpeilirun: 0.0,
            meigujingzichan: 5.0,
        };
        let converted = FundFinanceInfo::from_finance_info(&finance);
        assert_eq!(converted.code, finance.code);
        assert_eq!(converted.meigujingzichan, finance.meigujingzichan);
    }
}
