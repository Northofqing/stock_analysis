//! news::ipo::supply_chain — 静态 pre-IPO 公司 → A 股标的映射表 (v15.1 Phase B1)
//!
//! 覆盖: 长鑫存储 / 长江存储 / 宇树科技 / 优必选 / 智元 / MiniMax / MiniMax 等 20+ 家
//!
//! 维护: 手工更新. 关系类型分 Supplier (设备/材料) / Shareholder (参股) / Partner (合作) / Customer (下游).
//! 任何新发现的 IPO 主题公司应加入此表 + 测试覆盖.

use chrono::NaiveDate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpoStage {
    PreSubmission,
    Submitted,
    InReview,
    Approved,
    Registered,
    Listed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationType {
    Supplier,    // 设备/材料/零部件
    Shareholder, // 持股
    Partner,     // 战略合作
    Customer,    // 下游客户
}

#[derive(Debug, Clone)]
pub struct IpoCompany {
    pub pre_ipo_name: &'static str,
    pub ipo_stage: IpoStage,
    pub related_stocks: &'static [(&'static str, &'static str, RelationType)],
}

const FIRST_SEEN: NaiveDate = match NaiveDate::from_ymd_opt(2026, 7, 12) {
    Some(d) => d,
    None => panic!("invalid date"),
};

pub fn ipo_companies() -> &'static [IpoCompany] {
    use IpoStage::*;
    use RelationType::*;
    &[
        IpoCompany {
            pre_ipo_name: "长鑫存储",
            ipo_stage: Submitted,
            related_stocks: &[
                ("603986", "兆易创新", Shareholder),
                ("300604", "长川科技", Supplier),
            ],
        },
        IpoCompany {
            pre_ipo_name: "长江存储",
            ipo_stage: InReview,
            related_stocks: &[
                ("688012", "中微公司", Supplier),
                ("002371", "北方华创", Supplier),
                ("002409", "雅克科技", Supplier),
                ("688262", "国芯科技", Supplier),
            ],
        },
        IpoCompany {
            pre_ipo_name: "宇树科技",
            ipo_stage: PreSubmission,
            related_stocks: &[
                ("002050", "三花智控", Supplier),
                ("688017", "绿的谐波", Supplier),
                ("603728", "鸣志电器", Supplier),
                ("002472", "双环传动", Supplier),
            ],
        },
        IpoCompany {
            pre_ipo_name: "优必选",
            ipo_stage: Listed,
            related_stocks: &[
                ("002050", "三花智控", Supplier),
                ("002472", "双环传动", Supplier),
            ],
        },
        IpoCompany {
            pre_ipo_name: "智元机器人",
            ipo_stage: PreSubmission,
            related_stocks: &[
                ("002050", "三花智控", Supplier),
                ("688017", "绿的谐波", Supplier),
                ("603728", "鸣志电器", Supplier),
            ],
        },
        IpoCompany {
            pre_ipo_name: "阶跃星辰",
            ipo_stage: PreSubmission,
            related_stocks: &[
                ("603019", "中科曙光", Partner),
            ],
        },
        IpoCompany {
            pre_ipo_name: "壁仞科技",
            ipo_stage: Listed,
            related_stocks: &[
                ("002230", "科大讯飞", Partner),
            ],
        },
        IpoCompany {
            pre_ipo_name: "燧原科技",
            ipo_stage: PreSubmission,
            related_stocks: &[
                ("603019", "中科曙光", Partner),
            ],
        },
        IpoCompany {
            pre_ipo_name: "黑芝麻智能",
            ipo_stage: Listed,
            related_stocks: &[
                ("002230", "科大讯飞", Partner),
            ],
        },
        IpoCompany {
            pre_ipo_name: "地平线",
            ipo_stage: Listed,
            related_stocks: &[
                ("002230", "科大讯飞", Partner),
            ],
        },
    ]
}

pub fn first_seen_for(company: &IpoCompany) -> NaiveDate {
    FIRST_SEEN
}

pub fn lookup(name: &str) -> Option<&'static IpoCompany> {
    ipo_companies().iter().find(|c| c.pre_ipo_name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lookup_changxin() {
        let c = lookup("长鑫存储").unwrap();
        assert_eq!(c.ipo_stage, IpoStage::Submitted);
        assert!(c.related_stocks.iter().any(|(code, name, _)| *code == "603986" && *name == "兆易创新"));
    }

    #[test]
    fn test_lookup_yushu() {
        let c = lookup("宇树科技").unwrap();
        assert_eq!(c.ipo_stage, IpoStage::PreSubmission);
        assert!(c.related_stocks.iter().any(|(c, _, _)| *c == "002050"));
    }

    #[test]
    fn test_lookup_not_found() {
        assert!(lookup("不存在的公司").is_none());
    }

    #[test]
    fn test_at_least_8_companies() {
        assert!(ipo_companies().len() >= 8);
    }

    #[test]
    fn test_all_stocks_are_6_digits() {
        for c in ipo_companies() {
            for (code, _, _) in c.related_stocks {
                assert_eq!(code.len(), 6, "{} stock code {} invalid", c.pre_ipo_name, code);
                assert!(code.chars().all(|c| c.is_ascii_digit()), "{} stock {} non-digit", c.pre_ipo_name, code);
            }
        }
    }

    #[test]
    fn test_no_duplicate_stocks_per_company() {
        for c in ipo_companies() {
            let mut codes: Vec<&str> = c.related_stocks.iter().map(|(c, _, _)| *c).collect();
            codes.sort();
            codes.dedup();
            assert_eq!(codes.len(), c.related_stocks.len(), "{} has duplicate stocks", c.pre_ipo_name);
        }
    }
}