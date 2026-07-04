// @generated automatically by Diesel CLI.

diesel::table! {
    stock_daily (id) {
        id -> Integer,
        code -> Text,
        date -> Date,
        open -> Nullable<Double>,
        high -> Nullable<Double>,
        low -> Nullable<Double>,
        close -> Nullable<Double>,
        volume -> Nullable<Double>,
        amount -> Nullable<Double>,
        pct_chg -> Nullable<Double>,
        ma5 -> Nullable<Double>,
        ma10 -> Nullable<Double>,
        ma20 -> Nullable<Double>,
        volume_ratio -> Nullable<Double>,
        data_source -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    lhb_daily (id) {
        id -> Integer,
        code -> Text,
        name -> Text,
        trade_date -> Text,
        reason -> Text,
        pct_change -> Double,
        close_price -> Double,
        buy_amount -> Double,
        sell_amount -> Double,
        net_amount -> Double,
        total_amount -> Double,
        lhb_ratio -> Double,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    analysis_result (id) {
        id -> Integer,
        code -> Text,
        name -> Text,
        date -> Date,
        sentiment_score -> Integer,
        operation_advice -> Text,
        trend_prediction -> Text,
        pe_ratio -> Nullable<Double>,
        pb_ratio -> Nullable<Double>,
        turnover_rate -> Nullable<Double>,
        market_cap -> Nullable<Double>,
        circulating_cap -> Nullable<Double>,
        close_price -> Nullable<Double>,
        pct_chg -> Nullable<Double>,
        data_source -> Nullable<Text>,
        created_at -> Timestamp,
        score_breakdown_json -> Nullable<Text>,
        original_advice -> Nullable<Text>,
        veto_flags_json -> Nullable<Text>,
    }
}

diesel::table! {
    stock_position (id) {
        id -> Integer,
        code -> Text,
        name -> Text,
        buy_date -> Text,
        buy_price -> Double,
        quantity -> Integer,
        status -> Text,
        sell_date -> Nullable<Text>,
        sell_price -> Nullable<Double>,
        return_rate -> Nullable<Double>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        chain_name -> Nullable<Text>,
    }
}

// v12 PR3-3.1 (BR-023/024) — 虚拟盘 + 调整 + 执行跟踪
diesel::table! {
    paper_trades (id) {
        id -> Integer,
        plan_id -> Text,
        code -> Text,
        name -> Text,
        direction -> Text,
        price -> Double,
        quantity -> Integer,
        status -> Text,
        fill_price -> Nullable<Double>,
        not_fill_reason -> Nullable<Text>,
        virtual_reason -> Text,
        account_mode -> Text,
        data_mode -> Text,
        ts -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    execution_tracking (id) {
        id -> Integer,
        paper_trade_id -> Integer,
        plan_id -> Text,
        code -> Text,
        expected_price -> Double,
        actual_change_t1 -> Nullable<Double>,
        actual_change_t3 -> Nullable<Double>,
        actual_change_t5 -> Nullable<Double>,
        mfe -> Nullable<Double>,
        mae -> Nullable<Double>,
        t1_special_case -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    position_adjustments (id) {
        id -> Integer,
        code -> Text,
        delta -> Integer,
        source -> Text,
        reason -> Text,
        effective_date -> Text,
        applied_immediately -> Integer,
        operator -> Nullable<Text>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    trades (id) {
        id -> Integer,
        code -> Text,
        name -> Text,
        direction -> Text,
        price -> Double,
        shares -> Integer,
        amount -> Double,
        reason -> Text,
        traded_at -> Text,
        created_at -> Timestamp,
        // 修复 P1.3: 量化分析师要求的真实业绩归因字段
        commission_amount -> Nullable<Double>,
        stamp_tax_amount -> Nullable<Double>,
        slippage_amount -> Nullable<Double>,
        realized_pnl -> Nullable<Double>,
        strategy_tag -> Nullable<Text>,
        signal_id -> Nullable<Text>,
    }
}

diesel::table! {
    ledger (id) {
        id -> Integer,
        date -> Text,
        total_value -> Double,
        cash -> Double,
        market_value -> Double,
        daily_pnl -> Double,
        created_at -> Timestamp,
    }
}

// v12 PR1-1.5 (BR-021): 账户模式变更日志表
diesel::table! {
    account_mode_log (id) {
        id -> Integer,
        ts -> Timestamp,
        prev_mode -> Text,
        new_mode -> Text,
        trigger_reason -> Text,
        today_pnl_pct -> Nullable<Double>,
        consecutive_n -> Nullable<Integer>,
        total_pos_cheng -> Nullable<Integer>,
        data_complete -> Integer,
        pushed -> Integer,
        push_attempted_at -> Nullable<Timestamp>,
    }
}

diesel::allow_tables_to_appear_in_same_query!(
    stock_daily,
    lhb_daily,
    analysis_result,
    stock_position,
    trades,
    ledger,
    account_mode_log,
);
