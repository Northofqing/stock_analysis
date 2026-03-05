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
    }
}

diesel::allow_tables_to_appear_in_same_query!(
    stock_daily,
    lhb_daily,
    analysis_result,
);
