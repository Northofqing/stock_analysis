use encoding_rs::GBK;

use crate::constants::{max_valid_year, read_u16, read_u32};
use crate::error::{Result, TdxError};
use crate::error_codes::ErrorCode;
use crate::helpers::{get_price, get_volume};

use super::cursor::PacketCursor;
use super::types::*;

fn checked_cumulative_value(base: i64, delta: i64, context: &str) -> Result<i64> {
    base.checked_add(delta)
        .ok_or_else(|| ErrorCode::TYPE_MISMATCH.err(format!("{context} overflow")))
}

fn checked_nonnegative_value(value: i64, context: &str) -> Result<i64> {
    if value < 0 {
        return Err(ErrorCode::TYPE_MISMATCH.err(format!("{context} is negative")));
    }
    Ok(value)
}

fn checked_nonnegative_cumulative_value(base: i64, delta: i64, context: &str) -> Result<i64> {
    checked_nonnegative_value(checked_cumulative_value(base, delta, context)?, context)
}

fn checked_unsigned_32(value: i64, context: &str) -> Result<u32> {
    if value < 0 {
        return Err(ErrorCode::TYPE_MISMATCH.err(format!("{context} is negative")));
    }
    u32::try_from(value).map_err(|_| {
        ErrorCode::TYPE_MISMATCH.err(format!("{context} is outside the unsigned 32-bit domain"))
    })
}

/// 有符号 32 位收窄, 保留负值语义.
/// 用于 TDX 协议中定义为 `-price` 的保留字段, 这类字段在正常行情下恒为负,
/// 施加非负约束会让所有非停牌标的解析失败.
fn checked_signed_32(value: i64, context: &str) -> Result<i32> {
    i32::try_from(value).map_err(|_| {
        ErrorCode::TYPE_MISMATCH.err(format!("{context} is outside the signed 32-bit domain"))
    })
}

/// 解码 TDX 401 quote 行的服务端时间字段 (`reversed_bytes0`).
///
/// 字段是交易时段内的时刻, 以十进制位编码: `HH MM (SS.fff × 1000/6)`
/// (pytdx `_format_time` 语义, 来源 rainx/pytdx issue #187)。例如
/// `11293635` → 11:29:21.810。历史样本中分钟位可能越过 60
/// (`14999269` → 14:59:57.368), 此时后 6 位按 10^6/60 每分钟的尺度编码
/// 分钟(含小数), 两种分支都保留以贴合 pytdx 参考实现。
///
/// 输出 `"HH:MM:SS"`(整秒四舍五入)——下游按 `%H:%M:%S` 解析, 且 TDX
/// quote 约 3 秒才刷新一次, 毫秒无业务意义。非正或非法值返回空串:
/// 不伪造时间证据, 由调用方按缺时间处理。
fn format_servertime(value: i32) -> String {
    if value <= 0 {
        return String::new();
    }
    // 补齐前导 0 (如 09:15:00 → 9150000 → "09150000") 后再切位。
    let digits = format!("{value:08}");
    let digits = digits.as_bytes();
    let parse2 = |bytes: &[u8]| -> i32 {
        bytes
            .iter()
            .fold(0i32, |acc, b| acc * 10 + i32::from(b.wrapping_sub(b'0')))
    };
    let hour = parse2(&digits[0..2]);
    let mm_part = parse2(&digits[2..4]);
    let (minute, seconds) = if mm_part < 60 {
        // 常规形态: 分钟两位, 后 4 位按 10000/60 每秒编码秒(含毫秒小数)。
        let seconds_fff = parse2(&digits[4..8]) as f64 * 60.0 / 10_000.0;
        (f64::from(mm_part), seconds_fff)
    } else {
        // 历史样本形态: 后 6 位按 10^6/60 每分钟编码分钟(含小数), 小时在最高位。
        let mm_fff = parse2(&digits[2..8]) as f64 * 60.0 / 1_000_000.0;
        let minute = mm_fff.floor();
        (minute, (mm_fff - minute) * 60.0)
    };
    if !(0..=23).contains(&hour) || !(0.0..60.0).contains(&minute) || !(0.0..60.0).contains(&seconds)
    {
        return String::new();
    }
    // 向下取整: last4 = 9999 → 59.994 s, 四舍五入会产生非法的 ":60"。
    // 整秒误差 <1 s, TDX quote 约 3 秒刷新, 无业务影响。
    format!(
        "{hour:02}:{:02}:{:02}",
        minute as u8,
        seconds.floor() as u8
    )
}

// ============================================================
// 解析证券数量
// ============================================================

pub fn parse_security_count(body: &[u8]) -> Result<u16> {
    if body.len() < 2 {
        return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH.err("body too short for count"));
    }
    read_u16(body, 0)
}

// ============================================================
// 解析证券列表
// ============================================================

pub fn parse_security_list(body: &[u8]) -> Result<Vec<SecurityInfo>> {
    let mut cursor = PacketCursor::new(body);
    let count = usize::from(cursor.read_u16_le("security-list count")?);
    let mut result = Vec::with_capacity(count);

    for record_index in 0..count {
        cursor.set_record(record_index);
        let code_bytes = cursor.read_slice(6, "security code")?;
        let code = String::from_utf8_lossy(code_bytes)
            .trim_end_matches('\0')
            .to_string();
        let volunit = cursor.read_u16_le("volume unit")?;
        let name_bytes = cursor.read_slice(8, "security name")?;
        let (name, _, _) = GBK.decode(name_bytes);
        let name = name.trim_end_matches('\0').to_string();
        cursor.read_slice(4, "security-list reserved bytes")?;
        let decimal_point = cursor.read_u8("decimal point")?;
        let pre_close_raw = i64::from(cursor.read_u32_le("previous close")?);
        let pre_close = get_volume(pre_close_raw);
        cursor.read_slice(4, "security-list trailing reserved bytes")?;

        result.push(SecurityInfo {
            code,
            volunit,
            decimal_point,
            name,
            pre_close,
        });
    }
    if !cursor.is_empty() {
        return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH.err(format!(
            "security-list response has {} trailing bytes after {count} declared records",
            cursor.remaining()
        )));
    }

    Ok(result)
}

// ============================================================
// 解析K线数据 (个股)
// ============================================================

pub fn parse_security_bars(body: &[u8], category: u8) -> Result<Vec<SecurityBar>> {
    if body.len() < 2 {
        return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH.err("body too short"));
    }

    let count = read_u16(body, 0)? as usize;
    let mut pos = 2;
    let mut result = Vec::with_capacity(count);
    let mut pre_diff_base: i64 = 0;

    for row_index in 0..count {
        // Bounds check: datetime(4) + 4*price(var) + vol(4) + amount(4) = min 16
        if pos + 16 > body.len() {
            return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH
                .err(format!("security bar row {row_index} is truncated")));
        }

        let mut bar = SecurityBar {
            open: 0.0,
            close: 0.0,
            high: 0.0,
            low: 0.0,
            vol: 0.0,
            amount: 0.0,
            year: 0,
            month: 0,
            day: 0,
            hour: 0,
            minute: 0,
            datetime: String::new(),
        };

        // 日期时间
        let (year, month, day, hour, minute, new_pos) = get_datetime(category, body, pos)?;
        // 校验日期合法性 — 服务器可能返回损坏数据或无效代码的垃圾数据
        if year < 1980
            || year > max_valid_year()
            || !(1..=12).contains(&month)
            || !(1..=31).contains(&day)
        {
            return Err(ErrorCode::INVALID_DATE.err(format!(
                "security bar row {row_index} has invalid date {year:04}-{month:02}-{day:02}"
            )));
        }
        bar.year = year;
        bar.month = month;
        bar.day = day;
        bar.hour = hour;
        bar.minute = minute;
        pos = new_pos;

        if category < 4 || category == 7 || category == 8 {
            bar.datetime = format!(
                "{:04}-{:02}-{:02} {:02}:{:02}",
                year, month, day, hour, minute
            );
        } else {
            bar.datetime = format!("{:04}-{:02}-{:02}", year, month, day);
        }

        // 价格: 差分编码 (Python order: open, close, high, low)
        let (price_open_diff, new_pos) = get_price(body, pos)?;
        let accumulated = checked_nonnegative_cumulative_value(
            pre_diff_base,
            price_open_diff,
            &format!("security bar row {row_index} open price"),
        )?;
        bar.open = accumulated as f64 / 1000.0;
        pos = new_pos;

        let (price_close_diff, new_pos) = get_price(body, pos)?;
        let close = checked_nonnegative_cumulative_value(
            accumulated,
            price_close_diff,
            &format!("security bar row {row_index} close price"),
        )?;
        bar.close = close as f64 / 1000.0;
        pos = new_pos;

        let (price_high_diff, new_pos) = get_price(body, pos)?;
        bar.high = checked_nonnegative_cumulative_value(
            accumulated,
            price_high_diff,
            &format!("security bar row {row_index} high price"),
        )? as f64
            / 1000.0;
        pos = new_pos;

        let (price_low_diff, new_pos) = get_price(body, pos)?;
        bar.low = checked_nonnegative_cumulative_value(
            accumulated,
            price_low_diff,
            &format!("security bar row {row_index} low price"),
        )? as f64
            / 1000.0;
        pos = new_pos;

        pre_diff_base = close;

        // vol (u32) - Python reads vol first
        let vol_raw = read_u32(body, pos)? as i64;
        bar.vol = get_volume(vol_raw);
        pos += 4;

        // amount (u32) - Python reads amount (db_vol) second
        let amount_raw = read_u32(body, pos)? as i64;
        bar.amount = get_volume(amount_raw);
        pos += 4;

        result.push(bar);
    }
    let trailing = body.len() - pos;
    if trailing != 0 && trailing != 4 {
        return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH.err(format!(
            "security bar response has {trailing} unsupported trailing bytes after {count} declared records"
        )));
    }

    Ok(result)
}

// ============================================================
// 解析K线数据 (指数)
// ============================================================

pub fn parse_index_bars(body: &[u8], category: u8) -> Result<Vec<IndexBar>> {
    if body.len() < 2 {
        return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH.err("body too short"));
    }

    let count = read_u16(body, 0)? as usize;
    let mut pos = 2;
    let mut result = Vec::with_capacity(count);
    let mut pre_diff_base: i64 = 0;

    for row_index in 0..count {
        // datetime(4) + 4*price(var) + vol(4) + amount(4) + up_count(2) + down_count(2) = min 24
        if pos + 24 > body.len() {
            return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH
                .err(format!("index bar row {row_index} is truncated")));
        }
        let mut bar = IndexBar {
            open: 0.0,
            close: 0.0,
            high: 0.0,
            low: 0.0,
            vol: 0.0,
            amount: 0.0,
            year: 0,
            month: 0,
            day: 0,
            hour: 0,
            minute: 0,
            datetime: String::new(),
            up_count: 0,
            down_count: 0,
        };

        // 日期时间
        let (year, month, day, hour, minute, new_pos) = get_datetime(category, body, pos)?;
        // 校验日期合法性 — 服务器可能返回损坏数据或无效代码的垃圾数据
        if year < 1980
            || year > max_valid_year()
            || !(1..=12).contains(&month)
            || !(1..=31).contains(&day)
        {
            return Err(ErrorCode::INVALID_DATE.err(format!(
                "index bar row {row_index} has invalid date {year:04}-{month:02}-{day:02}"
            )));
        }
        bar.year = year;
        bar.month = month;
        bar.day = day;
        bar.hour = hour;
        bar.minute = minute;
        pos = new_pos;

        if category < 4 || category == 7 || category == 8 {
            bar.datetime = format!(
                "{:04}-{:02}-{:02} {:02}:{:02}",
                year, month, day, hour, minute
            );
        } else {
            bar.datetime = format!("{:04}-{:02}-{:02}", year, month, day);
        }

        // 价格: 差分编码 (Python order: open, close, high, low)
        let (price_open_diff, new_pos) = get_price(body, pos)?;
        let accumulated = checked_nonnegative_cumulative_value(
            pre_diff_base,
            price_open_diff,
            &format!("index bar row {row_index} open price"),
        )?;
        bar.open = accumulated as f64 / 1000.0;
        pos = new_pos;

        let (price_close_diff, new_pos) = get_price(body, pos)?;
        let close = checked_nonnegative_cumulative_value(
            accumulated,
            price_close_diff,
            &format!("index bar row {row_index} close price"),
        )?;
        bar.close = close as f64 / 1000.0;
        pos = new_pos;

        let (price_high_diff, new_pos) = get_price(body, pos)?;
        bar.high = checked_nonnegative_cumulative_value(
            accumulated,
            price_high_diff,
            &format!("index bar row {row_index} high price"),
        )? as f64
            / 1000.0;
        pos = new_pos;

        let (price_low_diff, new_pos) = get_price(body, pos)?;
        bar.low = checked_nonnegative_cumulative_value(
            accumulated,
            price_low_diff,
            &format!("index bar row {row_index} low price"),
        )? as f64
            / 1000.0;
        pos = new_pos;

        pre_diff_base = close;

        let vol_raw = read_u32(body, pos)? as i64;
        bar.vol = get_volume(vol_raw);
        pos += 4;

        let amount_raw = read_u32(body, pos)? as i64;
        bar.amount = get_volume(amount_raw);
        pos += 4;

        // up_count, down_count (u16 each)
        bar.up_count = read_u16(body, pos)? as u32;
        pos += 2;
        bar.down_count = read_u16(body, pos)? as u32;
        pos += 2;

        result.push(bar);
    }
    let trailing = body.len() - pos;
    if trailing != 0 && trailing != 4 {
        return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH.err(format!(
            "index bar response has {trailing} unsupported trailing bytes after {count} declared records"
        )));
    }

    Ok(result)
}

// ============================================================
// 解析分时数据
// ============================================================

/// 根据分时数据索引计算时间字符串
///
/// TDX 分时数据每天 240 个点，开盘集合竞价视为无有效数据点:
/// - 上午 120 个: 09:31 ~ 11:30 (index 0-119)，不含 09:30
/// - 下午 120 个: 13:01 ~ 15:00 (index 120-239)，不含 13:00
pub fn minute_time_from_index(index: usize) -> String {
    let total = if index < 120 {
        9 * 60 + 31 + index // 09:31 + index → 09:31 ~ 11:30
    } else {
        13 * 60 + 1 + (index - 120) // 13:01 + (index-120) → 13:01 ~ 15:00
    };
    format!("{:02}:{:02}", total / 60, total % 60)
}

/// 解析当日分时数据
///
/// ⚠️ 已知问题: TDX 实时分时 API (命令码 0x051d) 的数据格式与历史分时 API 不同，
/// 且数据编码存在异常（价格差分编码在某些记录会重置）。
///
/// 建议使用 `get_history_minute_time_data` API 替代，传入今日日期即可获取当日数据，
/// 该 API 数据格式稳定且已验证正确。
///
/// 当前实现基于逆向分析，头部偏移 13 字节，但部分场景下价格可能异常。
pub fn parse_minute_time_data(body: &[u8], market: u8, code: &str) -> Result<Vec<MinuteTimePrice>> {
    let coefficient = super::types::get_security_coefficient(market, code);

    if body.len() < 13 {
        return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH.err("minute header is truncated"));
    }

    let count = read_u16(body, 0)? as usize;
    // 实时分时数据头部: 2(count) + 2(padding) + 1(indicator) + 6(stock_code) + 2(unknown) = 13 bytes
    // 注意: 此偏移量基于逆向分析，可能不完全准确
    let mut pos = 13;
    let mut result = Vec::with_capacity(count);

    let mut pre_diff_base: i64 = 0;
    let mut cum_amount: f64 = 0.0;
    let mut cum_vol: f64 = 0.0;

    for i in 0..count {
        let (price_diff, new_pos) = get_price(body, pos)?;
        pre_diff_base = checked_nonnegative_cumulative_value(
            pre_diff_base,
            price_diff,
            &format!("realtime minute row {i} cumulative price"),
        )?;
        let price = (pre_diff_base as f64) * coefficient;
        pos = new_pos;

        // reversed1 (skipped)
        let (_, new_pos) = get_price(body, pos)?;
        pos = new_pos;

        let (vol_diff, new_pos) = get_price(body, pos)?;
        let vol = f64::from(checked_unsigned_32(
            vol_diff,
            &format!("realtime minute row {i} volume"),
        )?);
        pos = new_pos;

        // 均价 = 累计金额 / 累计成交量
        cum_amount += price * vol;
        cum_vol += vol;
        let avg_price = if cum_vol > 0.0 {
            cum_amount / cum_vol
        } else {
            price
        };

        let time = minute_time_from_index(i);
        result.push(MinuteTimePrice {
            time,
            price,
            avg_price,
            vol,
        });
    }

    if pos != body.len() {
        return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH.err(format!(
            "realtime minute response has {} trailing bytes after {count} declared records",
            body.len() - pos
        )));
    }

    // 倒序排列：最新记录在前
    result.reverse();
    Ok(result)
}

// ============================================================
// 解析历史分时数据
// ============================================================

pub fn parse_history_minute_time_data(
    body: &[u8],
    market: u8,
    code: &str,
) -> Result<Vec<MinuteTimePrice>> {
    let coefficient = super::types::get_security_coefficient(market, code);

    if body.len() < 6 {
        return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH.err("history minute header is truncated"));
    }
    // 跳过 6 bytes header
    let mut pos = 6;

    let mut result = Vec::new();
    let mut pre_diff_base: i64 = 0;
    let mut cum_amount: f64 = 0.0;
    let mut cum_vol: f64 = 0.0;
    let mut index: usize = 0;

    while pos < body.len() {
        let (price_diff, new_pos) = get_price(body, pos)?;
        pre_diff_base = checked_nonnegative_cumulative_value(
            pre_diff_base,
            price_diff,
            &format!("history minute row {index} cumulative price"),
        )?;
        let price = (pre_diff_base as f64) * coefficient;
        pos = new_pos;

        // reversed1 (skipped)
        let (_, new_pos) = get_price(body, pos)?;
        pos = new_pos;

        let (vol_diff, new_pos) = get_price(body, pos)?;
        let vol = f64::from(checked_unsigned_32(
            vol_diff,
            &format!("history minute row {index} volume"),
        )?);
        pos = new_pos;

        // 均价 = 累计金额 / 累计成交量
        cum_amount += price * vol;
        cum_vol += vol;
        let avg_price = if cum_vol > 0.0 {
            cum_amount / cum_vol
        } else {
            price
        };

        let time = minute_time_from_index(index);
        result.push(MinuteTimePrice {
            time,
            price,
            avg_price,
            vol,
        });
        index += 1;
    }

    // 倒序排列：最新记录在前
    result.reverse();
    Ok(result)
}

// ============================================================
// 解析逐笔成交
// ============================================================

fn checked_transaction_varint(
    body: &[u8],
    pos: usize,
    row_index: usize,
    field: &str,
) -> Result<(i64, usize)> {
    const MAX_ENCODED_LEN: usize = 9;
    let encoded_len = body
        .get(pos..)
        .and_then(|remaining| {
            remaining
                .iter()
                .position(|byte| byte & 0x80 == 0)
                .map(|offset| offset + 1)
        })
        .filter(|encoded_len| *encoded_len <= MAX_ENCODED_LEN)
        .ok_or_else(|| {
            ErrorCode::RESPONSE_LENGTH_MISMATCH.err(format!(
                "current transaction row {row_index} {field} has invalid variable-length framing"
            ))
        })?;
    let (value, new_pos) = get_price(body, pos)?;
    debug_assert_eq!(new_pos, pos + encoded_len);
    Ok((value, new_pos))
}

fn checked_transaction_u32(
    body: &[u8],
    pos: usize,
    row_index: usize,
    field: &str,
) -> Result<(u32, usize)> {
    let (value, new_pos) = checked_transaction_varint(body, pos, row_index, field)?;
    let value = u32::try_from(value).map_err(|_| {
        ErrorCode::TYPE_MISMATCH.err(format!(
            "current transaction row {row_index} {field} is outside the unsigned 32-bit domain"
        ))
    })?;
    Ok((value, new_pos))
}

pub fn parse_transaction_data(body: &[u8]) -> Result<Vec<TickData>> {
    parse_transaction_data_with_coefficient(body, 0.01)
}

pub fn parse_transaction_data_with_coefficient(
    body: &[u8],
    coefficient: f64,
) -> Result<Vec<TickData>> {
    if body.len() < 2 {
        return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH.err("body too short"));
    }

    let count = read_u16(body, 0)? as usize;
    let mut pos = 2;
    let mut result = Vec::with_capacity(count);

    let mut last_price: i64 = 0;

    for row_index in 0..count {
        // time(2) plus five minimally one-byte variable integers.
        if body.len().saturating_sub(pos) < 7 {
            return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH
                .err(format!("current transaction row {row_index} is truncated")));
        }

        // time (u16 minutes)
        let minutes = read_u16(body, pos)? as u32;
        pos += 2;
        let hour = minutes / 60;
        let minute = minutes % 60;
        let time = format!("{:02}:{:02}", hour, minute);

        // price (delta encoded)
        let (price_diff, new_pos) = checked_transaction_varint(body, pos, row_index, "price")?;
        last_price = last_price.checked_add(price_diff).ok_or_else(|| {
            ErrorCode::TYPE_MISMATCH.err(format!(
                "current transaction row {row_index} cumulative price overflow"
            ))
        })?;
        if last_price < 0 {
            return Err(ErrorCode::TYPE_MISMATCH.err(format!(
                "current transaction row {row_index} cumulative price is negative"
            )));
        }
        let price = last_price as f64 * coefficient;
        pos = new_pos;

        // vol
        let (vol, new_pos) = checked_transaction_u32(body, pos, row_index, "volume")?;
        let vol = f64::from(vol);
        pos = new_pos;

        // num
        let (num, new_pos) = checked_transaction_u32(body, pos, row_index, "trade count")?;
        pos = new_pos;

        // buyorsell
        let (buyorsell, new_pos) = checked_transaction_u32(body, pos, row_index, "trade side")?;
        if buyorsell > 2 {
            return Err(ErrorCode::TYPE_MISMATCH.err(format!(
                "current transaction row {row_index} trade side {buyorsell} is outside 0..=2"
            )));
        }
        pos = new_pos;

        // reserved (原 extra field，具体含义待确认)
        let (reserved, new_pos) = checked_transaction_u32(body, pos, row_index, "reserved")?;
        pos = new_pos;

        result.push(TickData {
            time,
            price,
            vol,
            num,
            buyorsell,
            reserved,
        });
    }
    if pos != body.len() {
        return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH.err(format!(
            "current transaction response has {} trailing bytes after {count} declared records",
            body.len() - pos
        )));
    }

    Ok(result)
}

// ============================================================
// 解析历史逐笔成交
// ============================================================

pub fn parse_history_transaction_data(body: &[u8]) -> Result<Vec<TickData>> {
    parse_history_transaction_data_with_coefficient(body, 0.01)
}

pub fn parse_history_transaction_data_with_coefficient(
    body: &[u8],
    coefficient: f64,
) -> Result<Vec<TickData>> {
    if body.len() < 6 {
        return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH.err("body too short"));
    }

    let count = read_u16(body, 0)? as usize;
    // 跳过 2 bytes count + 4 bytes header
    let mut pos = 6;

    let mut result = Vec::with_capacity(count);
    let mut last_price: i64 = 0;

    for row_index in 0..count {
        // time(2) plus four minimally one-byte variable integers.
        if body.len().saturating_sub(pos) < 6 {
            return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH.err(format!(
                "historical transaction row {row_index} is truncated"
            )));
        }
        // time (u16 minutes)
        let minutes = read_u16(body, pos)? as u32;
        pos += 2;
        let hour = minutes / 60;
        let minute = minutes % 60;
        let time = format!("{:02}:{:02}", hour, minute);

        // price (delta encoded)
        let (price_diff, new_pos) = get_price(body, pos)?;
        last_price = checked_cumulative_value(
            last_price,
            price_diff,
            &format!("historical transaction row {row_index} cumulative price"),
        )?;
        if last_price < 0 {
            return Err(ErrorCode::TYPE_MISMATCH.err(format!(
                "historical transaction row {row_index} cumulative price is negative"
            )));
        }
        let price = last_price as f64 * coefficient;
        pos = new_pos;

        // vol
        let (vol, new_pos) = get_price(body, pos)?;
        let vol = u32::try_from(vol).map_err(|_| {
            ErrorCode::TYPE_MISMATCH.err(format!(
                "historical transaction row {row_index} volume is outside the unsigned 32-bit domain"
            ))
        })?;
        let vol = f64::from(vol);
        pos = new_pos;

        // buyorsell
        let (buyorsell, new_pos) = get_price(body, pos)?;
        let buyorsell = u32::try_from(buyorsell).map_err(|_| {
            ErrorCode::TYPE_MISMATCH.err(format!(
                "historical transaction row {row_index} trade side is outside the unsigned 32-bit domain"
            ))
        })?;
        if buyorsell > 2 {
            return Err(ErrorCode::TYPE_MISMATCH.err(format!(
                "historical transaction row {row_index} trade side {buyorsell} is outside 0..=2"
            )));
        }
        pos = new_pos;

        // reserved (原 extra field，具体含义待确认)
        let (reserved, new_pos) = get_price(body, pos)?;
        let reserved = u32::try_from(reserved).map_err(|_| {
            ErrorCode::TYPE_MISMATCH.err(format!(
                "historical transaction row {row_index} reserved is outside the unsigned 32-bit domain"
            ))
        })?;
        pos = new_pos;

        result.push(TickData {
            time,
            price,
            vol,
            num: 0,
            buyorsell,
            reserved,
        });
    }
    if pos != body.len() {
        return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH.err(format!(
            "historical transaction response has {} trailing bytes after {count} declared records",
            body.len() - pos
        )));
    }

    Ok(result)
}

// ============================================================
// 解析实时行情 (最复杂的解析器)
// ============================================================

pub fn parse_security_quotes(body: &[u8]) -> Result<Vec<SecurityQuote>> {
    if body.len() < 4 {
        return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH.err("body too short"));
    }

    let mut pos = 0;
    pos += 2; // skip b1 cb

    let count = read_u16(body, pos)? as usize;
    pos += 2;

    let mut result = Vec::with_capacity(count);

    for row_index in 0..count {
        if body.len().saturating_sub(pos) < 30 {
            return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH
                .err(format!("quote row {row_index} is truncated")));
        }

        // market (u8) + code (6 bytes) + active1 (u16)
        let market = body[pos];
        pos += 1;

        let code_bytes = &body[pos..pos + 6];
        let code = String::from_utf8_lossy(code_bytes)
            .trim_end_matches('\0')
            .to_string();
        pos += 6;

        let active1 = read_u16(body, pos)?;
        pos += 2;

        let coefficient = super::types::get_security_coefficient(market, &code);

        // price (base price, delta encoded)
        let (price_raw, new_pos) = get_price(body, pos)?;
        let price_raw =
            checked_nonnegative_value(price_raw, &format!("quote row {row_index} base price"))?;
        pos = new_pos;

        // last_close (diff from price)
        let (last_close_diff, new_pos) = get_price(body, pos)?;
        pos = new_pos;

        // open (diff from price)
        let (open_diff, new_pos) = get_price(body, pos)?;
        pos = new_pos;

        // high (diff from price)
        let (high_diff, new_pos) = get_price(body, pos)?;
        pos = new_pos;

        // low (diff from price)
        let (low_diff, new_pos) = get_price(body, pos)?;
        pos = new_pos;

        // reversed_bytes0 (get_price as i64, used for servertime)
        let (reversed_bytes0, new_pos) = get_price(body, pos)?;
        let reversed_bytes0 = checked_signed_32(
            reversed_bytes0,
            &format!("quote row {row_index} reversed_bytes0"),
        )?;
        pos = new_pos;

        let (reversed_bytes1, new_pos) = get_price(body, pos)?;
        let reversed_bytes1 = checked_signed_32(
            reversed_bytes1,
            &format!("quote row {row_index} reversed_bytes1"),
        )?;
        pos = new_pos;

        // vol (get_price)
        let (vol, new_pos) = get_price(body, pos)?;
        let vol = checked_unsigned_32(vol, &format!("quote row {row_index} volume"))?;
        pos = new_pos;

        // cur_vol (get_price)
        let (cur_vol, new_pos) = get_price(body, pos)?;
        let cur_vol =
            checked_unsigned_32(cur_vol, &format!("quote row {row_index} current volume"))?;
        pos = new_pos;

        // amount (u32 raw, use get_volume)
        let amount_raw = read_u32(body, pos)? as i64;
        let amount = get_volume(amount_raw);
        pos += 4;

        // s_vol (get_price)
        let (s_vol, new_pos) = get_price(body, pos)?;
        let s_vol = checked_unsigned_32(s_vol, &format!("quote row {row_index} sell volume"))?;
        pos = new_pos;

        // b_vol (get_price)
        let (b_vol, new_pos) = get_price(body, pos)?;
        let b_vol = checked_unsigned_32(b_vol, &format!("quote row {row_index} buy volume"))?;
        pos = new_pos;

        // reversed_bytes2, reversed_bytes3
        let (reversed_bytes2, new_pos) = get_price(body, pos)?;
        let reversed_bytes2 = checked_signed_32(
            reversed_bytes2,
            &format!("quote row {row_index} reversed_bytes2"),
        )?;
        pos = new_pos;
        let (reversed_bytes3, new_pos) = get_price(body, pos)?;
        let reversed_bytes3 = checked_signed_32(
            reversed_bytes3,
            &format!("quote row {row_index} reversed_bytes3"),
        )?;
        pos = new_pos;

        // bid1-ask5: interleaved pairs (bid, ask, bid_vol, ask_vol) x 5
        let mut bid_prices = [0.0f64; 5];
        let mut ask_prices = [0.0f64; 5];
        let mut bid_vols = [0.0f64; 5];
        let mut ask_vols = [0.0f64; 5];

        for i in 0..5 {
            let (diff, new_pos) = get_price(body, pos)?;
            bid_prices[i] = checked_nonnegative_cumulative_value(
                price_raw,
                diff,
                &format!("quote row {row_index} bid level {} price", i + 1),
            )? as f64
                * coefficient;
            pos = new_pos;

            let (diff, new_pos) = get_price(body, pos)?;
            ask_prices[i] = checked_nonnegative_cumulative_value(
                price_raw,
                diff,
                &format!("quote row {row_index} ask level {} price", i + 1),
            )? as f64
                * coefficient;
            pos = new_pos;

            let (vol, new_pos) = get_price(body, pos)?;
            bid_vols[i] = f64::from(checked_unsigned_32(
                vol,
                &format!("quote row {row_index} bid level {} volume", i + 1),
            )?);
            pos = new_pos;

            let (vol, new_pos) = get_price(body, pos)?;
            ask_vols[i] = f64::from(checked_unsigned_32(
                vol,
                &format!("quote row {row_index} ask level {} volume", i + 1),
            )?);
            pos = new_pos;
        }

        // reversed_bytes4 (u16)
        let reversed_bytes4 = read_u16(body, pos)? as u32;
        pos += 2;

        // reversed_bytes5, reversed_bytes6, reversed_bytes7, reversed_bytes8
        let (reversed_bytes5, new_pos) = get_price(body, pos)?;
        let reversed_bytes5 = checked_signed_32(
            reversed_bytes5,
            &format!("quote row {row_index} reversed_bytes5"),
        )?;
        pos = new_pos;
        let (reversed_bytes6, new_pos) = get_price(body, pos)?;
        let reversed_bytes6 = checked_signed_32(
            reversed_bytes6,
            &format!("quote row {row_index} reversed_bytes6"),
        )?;
        pos = new_pos;
        let (reversed_bytes7, new_pos) = get_price(body, pos)?;
        let reversed_bytes7 = checked_signed_32(
            reversed_bytes7,
            &format!("quote row {row_index} reversed_bytes7"),
        )?;
        pos = new_pos;
        let (reversed_bytes8, new_pos) = get_price(body, pos)?;
        let reversed_bytes8 = checked_signed_32(
            reversed_bytes8,
            &format!("quote row {row_index} reversed_bytes8"),
        )?;
        pos = new_pos;

        // reversed_bytes9 (opaque u16 wire bits) + active2 (u16)
        if pos.checked_add(4).is_none_or(|end| end > body.len()) {
            return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH.err("quote tail truncated"));
        }
        let reversed_bytes9 = read_u16(body, pos)?;
        pos += 2;
        let active2 = read_u16(body, pos)?;
        pos += 2;

        // `reversed_bytes0` 编码服务端时间 (见 `format_servertime`), 解码失败
        // (非法值) 时为空串: 不伪造时间证据, 由调用方按缺时间处理。
        let servertime = format_servertime(reversed_bytes0);

        let price = (price_raw as f64) * coefficient;
        let last_close = checked_nonnegative_cumulative_value(
            price_raw,
            last_close_diff,
            &format!("quote row {row_index} last close price"),
        )? as f64
            * coefficient;
        let open = checked_nonnegative_cumulative_value(
            price_raw,
            open_diff,
            &format!("quote row {row_index} open price"),
        )? as f64
            * coefficient;
        let high = checked_nonnegative_cumulative_value(
            price_raw,
            high_diff,
            &format!("quote row {row_index} high price"),
        )? as f64
            * coefficient;
        let low = checked_nonnegative_cumulative_value(
            price_raw,
            low_diff,
            &format!("quote row {row_index} low price"),
        )? as f64
            * coefficient;

        result.push(SecurityQuote {
            market,
            code,
            active1,
            price,
            last_close,
            open,
            high,
            low,
            servertime,
            vol: f64::from(vol),
            cur_vol: f64::from(cur_vol),
            amount,
            s_vol: f64::from(s_vol),
            b_vol: f64::from(b_vol),
            bid1: bid_prices[0],
            bid_vol1: bid_vols[0],
            bid2: bid_prices[1],
            bid_vol2: bid_vols[1],
            bid3: bid_prices[2],
            bid_vol3: bid_vols[2],
            bid4: bid_prices[3],
            bid_vol4: bid_vols[3],
            bid5: bid_prices[4],
            bid_vol5: bid_vols[4],
            ask1: ask_prices[0],
            ask_vol1: ask_vols[0],
            ask2: ask_prices[1],
            ask_vol2: ask_vols[1],
            ask3: ask_prices[2],
            ask_vol3: ask_vols[2],
            ask4: ask_prices[3],
            ask_vol4: ask_vols[3],
            ask5: ask_prices[4],
            ask_vol5: ask_vols[4],
            reversed_bytes0,
            reversed_bytes1,
            reversed_bytes2,
            reversed_bytes3,
            reversed_bytes4,
            reversed_bytes5,
            reversed_bytes6,
            reversed_bytes7,
            reversed_bytes8,
            reversed_bytes9: u32::from(reversed_bytes9),
            active2,
        });
    }
    if pos != body.len() {
        return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH.err(format!(
            "quote response has {} trailing bytes after {count} declared records",
            body.len() - pos
        )));
    }

    Ok(result)
}

// ============================================================
// 解析财务信息
// ============================================================

pub fn parse_finance_info(body: &[u8], market: u8, code: &str) -> Result<FinanceInfo> {
    // Single-security response: count(2) + market(1) + code(6) + struct(136).
    if body.len() < 2 {
        return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH.err("body too short for finance count"));
    }
    let count = read_u16(body, 0)?;
    if count != 1 {
        return Err(TdxError::InvalidData(format!(
            "single-security finance response declared {count} records instead of exactly one"
        )));
    }
    const EXPECTED_LEN: usize = 2 + 1 + 6 + 136;
    if body.len() != EXPECTED_LEN {
        return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH.err(format!(
            "single-security finance response requires exactly {EXPECTED_LEN} bytes, received {}",
            body.len()
        )));
    }
    let response_market = body[2];
    let response_code = std::str::from_utf8(&body[3..9])
        .map_err(|_| TdxError::InvalidData("finance response code is not valid ASCII".into()))?;
    if response_market != market || response_code != code {
        return Err(TdxError::InvalidData(format!(
            "finance response identity ({response_market}, {response_code:?}) does not match request ({market}, {code:?})"
        )));
    }

    let mut pos = 9; // skip count(2) + market(1) + code(6)

    // f32 liutongguben — TDX 原始值 (单位不固定，由用户自行判断)
    let liutongguben =
        f32::from_le_bytes([body[pos], body[pos + 1], body[pos + 2], body[pos + 3]]) as f64;
    pos += 4;

    // u16 province
    let province = read_u16(body, pos)?;
    pos += 2;

    // u16 industry
    let industry = read_u16(body, pos)?;
    pos += 2;

    // u32 updated_date
    let updated_date = read_u32(body, pos)?;
    pos += 4;

    // u32 ipo_date
    let ipo_date = read_u32(body, pos)?;
    pos += 4;
    if ipo_date != 0 {
        validate_compact_source_date(ipo_date, "finance IPO date")?;
    }

    // 30 个 f32 字段 — 全部返回 TDX 原始值，不做单位转换
    let mut fields = Vec::with_capacity(30);
    for _ in 0..30 {
        let val =
            f32::from_le_bytes([body[pos], body[pos + 1], body[pos + 2], body[pos + 3]]) as f64;
        if !val.is_finite() {
            return Err(TdxError::InvalidData(
                "finance response contains a non-finite numeric field".into(),
            ));
        }
        fields.push(val);
        pos += 4;
    }
    if !liutongguben.is_finite() {
        return Err(TdxError::InvalidData(
            "finance response contains a non-finite circulating-share value".into(),
        ));
    }

    Ok(FinanceInfo {
        market,
        code: code.to_string(),
        liutongguben,
        province,
        industry,
        updated_date,
        ipo_date,
        zongguben: fields[0],
        guojiagu: fields[1],
        faqirenfarengu: fields[2],
        farengu: fields[3],
        bgu: fields[4],
        hgu: fields[5],
        zhigonggu: fields[6],
        zongzichan: fields[7],
        liudongzichan: fields[8],
        gudingzichan: fields[9],
        wuxingzichan: fields[10],
        gudongrenshu: fields[11],
        liudongfuzhai: fields[12],
        changqifuzhai: fields[13],
        zibengongjijin: fields[14],
        jingzichan: fields[15],
        zhuyingshouru: fields[16],
        zhuyinglirun: fields[17],
        yingshouzhangkuan: fields[18],
        yingyelirun: fields[19],
        touzishouyu: fields[20],
        jingyingxianjinliu: fields[21],
        zongxianjinliu: fields[22],
        cunhuo: fields[23],
        lirunzonghe: fields[24],
        shuihoulirun: fields[25],
        jinglirun: fields[26],
        weifenpeilirun: fields[27],
        meigujingzichan: fields[28],
    })
}

// ============================================================
// 解析除权除息
// ============================================================

pub fn parse_xdxr_info(body: &[u8]) -> Result<Vec<XdXrInfo>> {
    // Python: pos=0, pos+=9 (skip 9 bytes), read count at pos=9
    if body.len() < 11 {
        return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH.err("body too short"));
    }

    let mut pos = 9;
    let count = read_u16(body, pos)? as usize;
    pos += 2;
    let expected_len = 11usize
        .checked_add(count.checked_mul(29).ok_or_else(|| {
            TdxError::InvalidData("XDXR declared record count overflows response length".into())
        })?)
        .ok_or_else(|| {
            TdxError::InvalidData("XDXR declared response length overflows usize".into())
        })?;
    if body.len() != expected_len {
        return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH.err(format!(
            "XDXR declared {count} records require {expected_len} bytes, received {}",
            body.len()
        )));
    }

    let mut result = Vec::with_capacity(count);
    let response_market = body[2];
    let response_code = &body[3..9];

    for _ in 0..count {
        // market(1) + code(6) + reserved(1) + date(4) + category(1) + data(16).
        let row_market = body[pos];
        let row_code = &body[pos + 1..pos + 7];
        if row_market != response_market || row_code != response_code {
            return Err(TdxError::InvalidData(format!(
                "XDXR row identity ({row_market}, {:?}) does not match response identity ({response_market}, {:?})",
                String::from_utf8_lossy(row_code),
                String::from_utf8_lossy(response_code)
            )));
        }
        pos += 7;
        // One reserved source byte.
        pos += 1;

        // datetime (category 9 → YYYYMMDD u32)
        let (year, month, day, _hour, _minute, new_pos) = get_datetime(9, body, pos)?;
        pos = new_pos;
        let compact_date = year
            .checked_mul(10_000)
            .and_then(|value| value.checked_add(month * 100))
            .and_then(|value| value.checked_add(day))
            .ok_or_else(|| TdxError::InvalidData("XDXR date overflows u32".into()))?;
        validate_compact_source_date(compact_date, "XDXR effective date")?;

        // category (u8)
        let category = body[pos] as u32;
        pos += 1;

        // 16 bytes data (parsed differently by category)
        let mut fenhong = None;
        let mut peigujia = None;
        let mut songzhuangu = None;
        let mut peigu = None;
        let mut suogu = None;
        let mut panqianliutong = None;
        let mut panhouliutong = None;
        let mut qianzongguben = None;
        let mut houzongguben = None;
        let mut fenshu = None;
        let mut xingquanjia = None;

        if pos + 16 <= body.len() {
            let d = &body[pos..pos + 16];
            if category == 1 {
                fenhong = Some(f32::from_le_bytes([d[0], d[1], d[2], d[3]]) as f64);
                peigujia = Some(f32::from_le_bytes([d[4], d[5], d[6], d[7]]) as f64);
                songzhuangu = Some(f32::from_le_bytes([d[8], d[9], d[10], d[11]]) as f64);
                peigu = Some(f32::from_le_bytes([d[12], d[13], d[14], d[15]]) as f64);
            } else if category == 11 || category == 12 {
                suogu = Some(f32::from_le_bytes([d[8], d[9], d[10], d[11]]) as f64);
            } else if category == 13 || category == 14 {
                xingquanjia = Some(f32::from_le_bytes([d[0], d[1], d[2], d[3]]) as f64);
                fenshu = Some(f32::from_le_bytes([d[8], d[9], d[10], d[11]]) as f64);
            } else {
                let pqlt_raw = read_u32(d, 0)?;
                let qzgb_raw = read_u32(d, 4)?;
                let phlt_raw = read_u32(d, 8)?;
                let hzgb_raw = read_u32(d, 12)?;
                panqianliutong = Some(if pqlt_raw == 0 {
                    0.0
                } else {
                    get_volume(pqlt_raw as i64)
                });
                panhouliutong = Some(if phlt_raw == 0 {
                    0.0
                } else {
                    get_volume(phlt_raw as i64)
                });
                qianzongguben = Some(if qzgb_raw == 0 {
                    0.0
                } else {
                    get_volume(qzgb_raw as i64)
                });
                houzongguben = Some(if hzgb_raw == 0 {
                    0.0
                } else {
                    get_volume(hzgb_raw as i64)
                });
            }
        }
        pos += 16;
        for value in [
            fenhong,
            peigujia,
            songzhuangu,
            peigu,
            suogu,
            panqianliutong,
            panhouliutong,
            qianzongguben,
            houzongguben,
            fenshu,
            xingquanjia,
        ]
        .into_iter()
        .flatten()
        {
            if !value.is_finite() {
                return Err(TdxError::InvalidData(
                    "XDXR response contains a non-finite numeric field".into(),
                ));
            }
        }

        let name = match category {
            1 => "除权除息",
            2 => "送配股上市",
            3 => "非流通股上市",
            4 => "未知股本变动",
            5 => "股本变化",
            6 => "增发新股",
            7 => "股份回购",
            8 => "增发新股上市",
            9 => "转配股上市",
            10 => "可转债上市",
            11 => "扩缩股",
            12 => "非流通股缩股",
            13 => "送认购权证",
            14 => "送认沽权证",
            _ => "未知",
        }
        .to_string();

        result.push(XdXrInfo {
            year,
            month,
            day,
            category,
            name,
            fenhong,
            peigujia,
            songzhuangu,
            peigu,
            suogu,
            panqianliutong,
            panhouliutong,
            qianzongguben,
            houzongguben,
            fenshu,
            xingquanjia,
        });
    }

    Ok(result)
}

/// Parses an XDXR packet after proving that its response identity matches the request.
pub fn parse_xdxr_info_for(body: &[u8], market: u8, code: &str) -> Result<Vec<XdXrInfo>> {
    if body.len() < 9 {
        return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH.err("body too short for XDXR identity"));
    }
    let response_market = body[2];
    let response_code = std::str::from_utf8(&body[3..9])
        .map_err(|_| TdxError::InvalidData("XDXR response code is not valid ASCII".into()))?;
    if response_market != market || response_code != code {
        return Err(TdxError::InvalidData(format!(
            "XDXR response identity ({response_market}, {response_code:?}) does not match request ({market}, {code:?})"
        )));
    }
    parse_xdxr_info(body)
}

fn validate_compact_source_date(value: u32, field: &str) -> Result<()> {
    let text = format!("{value:08}");
    if text.len() != 8 {
        return Err(TdxError::InvalidData(format!(
            "{field} must contain exactly eight digits"
        )));
    }
    let iso = format!("{}-{}-{}", &text[0..4], &text[4..6], &text[6..8]);
    magic_market_core::IsoDate::new(iso)
        .map_err(|error| TdxError::InvalidData(format!("invalid {field}: {error}")))?;
    if value > crate::net::utils::today_yyyymmdd() {
        return Err(TdxError::InvalidData(format!(
            "{field} {value} is in the future"
        )));
    }
    Ok(())
}

// ============================================================
// 解析板块元数据
// ============================================================

pub fn parse_block_info_meta(body: &[u8]) -> Result<BlockInfoMeta> {
    if body.len() < 38 {
        return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH.err("body too short for block meta"));
    }

    let size = read_u32(body, 0)?;

    // 1 byte separator
    // 32 bytes hash
    let hash_bytes = &body[5..37];
    let hash_value: String = hash_bytes.iter().map(|b| format!("{:02x}", b)).collect();

    Ok(BlockInfoMeta { size, hash_value })
}

// ============================================================
// 解析板块数据 (返回原始字节)
// ============================================================

pub fn parse_block_info(body: &[u8]) -> Result<Vec<u8>> {
    // 跳过前 4 bytes header
    if body.len() < 4 {
        return Err(ErrorCode::RESPONSE_LENGTH_MISMATCH.err("block-info header is truncated"));
    }
    Ok(body[4..].to_vec())
}

// ============================================================
// 辅助函数: 日期时间解码
// ============================================================

fn get_datetime(
    category: u8,
    buffer: &[u8],
    pos: usize,
) -> Result<(u32, u32, u32, u32, u32, usize)> {
    if category < 4 || category == 7 || category == 8 {
        // 分钟级: u16 date + u16 minutes
        let zip_day = read_u16(buffer, pos)? as u32;
        let minutes = read_u16(buffer, pos + 2)? as u32;

        let year = (zip_day >> 11) + 2004;
        let month = (zip_day % 2048) / 100;
        let day = (zip_day % 2048) % 100;
        let hour = minutes / 60;
        let minute = minutes % 60;

        Ok((year, month, day, hour, minute, pos + 4))
    } else {
        // 日/周/月级: u32 date (YYYYMMDD)
        let zip_day = read_u32(buffer, pos)?;
        let year = zip_day / 10000;
        let month = (zip_day % 10000) / 100;
        let day = zip_day % 100;

        Ok((year, month, day, 0, 0, pos + 4))
    }
}

#[cfg(test)]
#[path = "../../tests/internal/protocol_parsers.rs"]
mod tests;
