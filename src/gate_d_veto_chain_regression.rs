use super::*;

struct StringPanicRule;

impl VetoRule for StringPanicRule {
    fn name(&self) -> &'static str {
        "StringPanicRule"
    }

    fn evaluate(&self, _ctx: &VetoContext) -> VetoVerdict {
        std::panic::panic_any("TEST_CODE string panic".to_string())
    }
}

struct UnknownPanicRule;

impl VetoRule for UnknownPanicRule {
    fn name(&self) -> &'static str {
        "UnknownPanicRule"
    }

    fn evaluate(&self, _ctx: &VetoContext) -> VetoVerdict {
        std::panic::panic_any(7_u8)
    }
}

fn context() -> VetoContext {
    VetoContext {
        code: "TEST_CODE_000001".into(),
        current_price: 10.0,
        signal_score: 50,
        is_buy_signal: false,
        bias_ma5: 0.0,
        is_bearish: false,
        money_flow_days: None,
        pct_chg: None,
        pe_ratio: None,
        net_profit_yoy: None,
    }
}

#[test]
fn panic_payload_variants_are_audited_and_do_not_abort_the_chain() {
    let chain = VetoChain::new(vec![Box::new(StringPanicRule), Box::new(UnknownPanicRule)]);
    let outcome = chain.evaluate_all(&context());
    assert!(outcome.is_empty());
    assert!(!outcome.force_hold);
}
