//! 裸连接客户端：每次请求新建 TCP 连接 + 三步握手，无连接池、无重试、无心跳
//!
//! 适用场景：偶发请求，不需要维护长连接

use std::sync::atomic::{AtomicU8, Ordering};

use crate::error::{Result, TdxError};
use crate::error_codes::ErrorCode;
use crate::loge;
use crate::logw;
use crate::net::connection::TcpConnection;
use crate::net::packet::{ResponseHeader, RSP_HEADER_LEN};
use crate::net::utils;
use crate::protocol::constants::*;
use crate::protocol::parsers::*;
use crate::protocol::types::*;

/// 裸连接客户端
///
/// 不维护连接池，每次 API 调用都经历：新建 TCP → 三步握手 → 发包 → 收包 → 解压 → 断开
pub struct TdxDirectClient {
    ip: String,
    port: u16,
    timeout: f64,
    /// 复权上下文数据量档位 (默认 Mid ≈ 20 年), 以 u8 存储
    fq_context_tier: AtomicU8,
}

impl TdxDirectClient {
    pub fn new(ip: &str, port: u16, timeout: f64) -> Self {
        Self {
            ip: ip.to_string(),
            port,
            timeout,
            fq_context_tier: AtomicU8::new(utils::FqContextTier::default() as u8),
        }
    }

    /// 更新服务器地址
    pub fn set_server(&mut self, ip: &str, port: u16) {
        self.ip = ip.to_string();
        self.port = port;
    }

    /// 更新超时
    pub fn set_timeout(&mut self, timeout: f64) {
        self.timeout = timeout;
    }

    // ================================================================
    // 核心：send_and_recv
    // ================================================================

    fn send_and_recv_on(conn: &mut TcpConnection, packet: &[u8]) -> Result<Vec<u8>> {
        conn.send(packet)?;

        let head_buf = conn.recv(RSP_HEADER_LEN)?;
        let header = ResponseHeader::parse(&head_buf)?;

        let zip_size = header.zip_size as usize;
        let mut body_buf = Vec::with_capacity(zip_size);
        while body_buf.len() < zip_size {
            let remaining = zip_size - body_buf.len();
            let chunk = conn.recv(remaining)?;
            body_buf.extend_from_slice(&chunk);
        }

        if body_buf.is_empty() {
            return Err(crate::error_codes::ErrorCode::DISCONNECTED.err("empty response body"));
        }

        if header.zip_size != header.unzip_size {
            utils::decompress_zlib(&body_buf)
        } else {
            Ok(body_buf)
        }
    }

    fn send_and_recv(&self, packet: &[u8]) -> Result<Vec<u8>> {
        let mut conn = TcpConnection::connect(&self.ip, self.port, self.timeout).map_err(|e| {
            loge!(
                "direct",
                "connect to {}:{} failed: {}",
                self.ip,
                self.port,
                e
            );
            e
        })?;
        utils::perform_handshake(&mut conn)?;
        Self::send_and_recv_on(&mut conn, packet)
    }

    /// 检查代码是否为板块代码 (88xxxx)，如果是则返回错误
    fn check_not_block_code(&self, code: &str) -> Result<()> {
        if crate::error_codes::is_block_code(code) {
            return Err(TdxError::coded(
                ErrorCode::BLOCK_CODE_IN_GENERAL_CLIENT,
                format!("code={}", code),
            ));
        }
        Ok(())
    }

    fn fetch_context_bars_for_adjust(
        &self,
        category: u8,
        market: u8,
        code: &str,
        bars: &[SecurityBar],
        xdxr: &[XdXrInfo],
    ) -> Vec<SecurityBar> {
        utils::fetch_context_bars_for_adjust_with_tier(
            |pkt| self.send_and_recv(pkt),
            category,
            market,
            code,
            bars,
            xdxr,
            self.fq_context_tier(),
        )
    }

    /// 设置复权上下文数据量档位
    pub fn set_fq_context_tier(&self, tier: utils::FqContextTier) {
        let val: u8 = match tier {
            utils::FqContextTier::Low => 0,
            utils::FqContextTier::Mid => 1,
            utils::FqContextTier::High => 2,
        };
        self.fq_context_tier.store(val, Ordering::SeqCst);
    }

    /// 获取当前复权上下文档位
    pub fn fq_context_tier(&self) -> utils::FqContextTier {
        let val = self.fq_context_tier.load(Ordering::SeqCst);
        match val {
            0 => utils::FqContextTier::Low,
            2 => utils::FqContextTier::High,
            _ => utils::FqContextTier::Mid,
        }
    }

    /// 获取复权因子计算所需的上下文数据 (追溯到上市)
    pub fn fetch_context_for_factors(
        &self,
        category: u8,
        market: u8,
        code: &str,
        bars: &[SecurityBar],
        xdxr: &[XdXrInfo],
    ) -> Result<Vec<SecurityBar>> {
        if bars.is_empty() || xdxr.is_empty() {
            return Ok(Vec::new());
        }

        let earliest_event = xdxr
            .iter()
            .filter(|x| x.category == 1)
            .map(|x| x.year * 10000 + x.month * 100 + x.day)
            .min();

        let Some(ee_date) = earliest_event else {
            return Ok(Vec::new());
        };

        let first_bar_date = bars[0].year * 10000 + bars[0].month * 100 + bars[0].day;

        if first_bar_date <= ee_date {
            return Ok(Vec::new());
        }

        let max_per_page = MAX_KLINE_COUNT as u32;
        let max_pages = 30u32;
        let mut context = Vec::new();
        let mut offset = max_per_page;

        for _page in 0..max_pages {
            let pkt = utils::build_security_bars_packet(
                category,
                market,
                code,
                offset,
                MAX_KLINE_COUNT,
                0,
            );
            let body = match self.send_and_recv(&pkt) {
                Ok(b) => b,
                Err(_) => break,
            };
            let batch = match parse_security_bars(&body, category) {
                Ok(b) => b,
                Err(_) => break,
            };
            if batch.is_empty() {
                break;
            }

            let batch_first_date =
                batch[0].year as u32 * 10000 + batch[0].month as u32 * 100 + batch[0].day as u32;

            let len_before = context.len();
            context.splice(0..0, batch);

            if batch_first_date <= ee_date {
                break;
            }

            offset += max_per_page;
            if context.len() == len_before {
                break;
            }
        }

        Ok(context)
    }

    // ================================================================
    // K线
    // ================================================================

    pub fn get_security_bars(
        &self,
        category: u8,
        market: u8,
        code: &str,
        start: u32,
        count: u16,
        fq: u8,
    ) -> Result<Vec<SecurityBar>> {
        self.check_not_block_code(code)?;
        let pkt = utils::build_security_bars_packet(category, market, code, start, count, fq);
        let mut bars = parse_security_bars(&self.send_and_recv(&pkt)?, category)?;
        if fq != 0 {
            if let Ok(xdxr) = self.get_xdxr_info(market, code) {
                use crate::protocol::adjuster::{adjust_security_bars, FqType};
                let fq_enum = if fq == 2 { FqType::Hfq } else { FqType::Qfq };
                let context =
                    self.fetch_context_bars_for_adjust(category, market, code, &bars, &xdxr);
                adjust_security_bars(&mut bars, &context, &xdxr, fq_enum);
            }
        }
        Ok(bars)
    }

    /// 获取指数K线 — 指数不存在复权概念，fq 参数保留接口一致性但不生效
    pub fn get_index_bars(
        &self,
        category: u8,
        market: u8,
        code: &str,
        start: u32,
        count: u16,
        fq: u8,
    ) -> Result<Vec<IndexBar>> {
        self.check_not_block_code(code)?;
        self.get_index_bars_inner(category, market, code, start, count, fq)
    }

    /// 获取指数K线 (内部方法，跳过板块代码检查)
    ///
    /// 供 TdxBlockClient 调用，板块代码 (88xxxx) 需要通过此方法查询。
    pub(crate) fn get_index_bars_inner(
        &self,
        category: u8,
        market: u8,
        code: &str,
        start: u32,
        count: u16,
        fq: u8,
    ) -> Result<Vec<IndexBar>> {
        let _ = fq; // 指数不复权，强制 fq=0 发送
        let pkt = utils::build_index_bars_packet(category, market, code, start, count, 0);
        let body = self.send_and_recv(&pkt)?;
        parse_index_bars(&body, category)
    }

    // ================================================================
    // 实时行情
    // ================================================================

    /// 获取实时行情
    ///
    /// 单次查询上限 60 只 (TDX 服务端硬限制)，超出自动截断并打印警告。
    pub fn get_security_quotes(&self, all_stock: &[(u8, &str)]) -> Result<Vec<SecurityQuote>> {
        // 检查是否有板块代码
        for &(_, code) in all_stock {
            self.check_not_block_code(code)?;
        }
        self.get_security_quotes_inner(all_stock)
    }

    /// 获取实时行情 (内部方法，跳过板块代码检查)
    ///
    /// 供 TdxBlockClient 调用，板块代码 (88xxxx) 需要通过此方法查询。
    pub(crate) fn get_security_quotes_inner(
        &self,
        all_stock: &[(u8, &str)],
    ) -> Result<Vec<SecurityQuote>> {
        // 服务端上限截断
        let all_stock = if all_stock.len() > MAX_QUOTES_COUNT {
            logw!(
                "direct",
                "批量行情查询超过上限 {}/{}，自动截断。请自行分组调用。",
                all_stock.len(),
                MAX_QUOTES_COUNT
            );
            &all_stock[..MAX_QUOTES_COUNT]
        } else {
            all_stock
        };
        let stock_len = all_stock.len() as u16;
        let pkgdatalen = (stock_len as u32) * 7 + 12;
        let mut pkt = Vec::with_capacity(26 + stock_len as usize * 7);
        pkt.extend_from_slice(&0x010Cu16.to_le_bytes());
        pkt.extend_from_slice(&0x02006320u32.to_le_bytes());
        pkt.extend_from_slice(&(pkgdatalen as u16).to_le_bytes());
        pkt.extend_from_slice(&(pkgdatalen as u16).to_le_bytes());
        pkt.extend_from_slice(&CMD_SECURITY_QUOTES.to_le_bytes());
        pkt.extend_from_slice(&0u32.to_le_bytes());
        pkt.extend_from_slice(&0u16.to_le_bytes());
        pkt.extend_from_slice(&stock_len.to_le_bytes());
        for &(market, code) in all_stock {
            pkt.push(market);
            pkt.extend_from_slice(&utils::code_bytes(code));
        }
        parse_security_quotes(&self.send_and_recv(&pkt)?)
    }

    // ================================================================
    // 证券信息
    // ================================================================

    pub fn get_security_list(&self, market: u8, start: u16) -> Result<Vec<SecurityInfo>> {
        let mut pkt = Vec::with_capacity(16);
        pkt.extend_from_slice(&[
            0x0c, 0x01, 0x18, 0x64, 0x01, 0x01, 0x06, 0x00, 0x06, 0x00, 0x50, 0x04,
        ]);
        pkt.extend_from_slice(&(market as u16).to_le_bytes());
        pkt.extend_from_slice(&start.to_le_bytes());
        parse_security_list(&self.send_and_recv(&pkt)?)
    }

    pub fn get_security_count(&self, market: u8) -> Result<u16> {
        let mut pkt = Vec::with_capacity(18);
        pkt.extend_from_slice(&[
            0x0c, 0x0c, 0x18, 0x6c, 0x00, 0x01, 0x08, 0x00, 0x08, 0x00, 0x4e, 0x04,
        ]);
        pkt.extend_from_slice(&(market as u16).to_le_bytes());
        pkt.extend_from_slice(&[0x75, 0xc7, 0x33, 0x01]);
        parse_security_count(&self.send_and_recv(&pkt)?)
    }

    // ================================================================
    // 分时数据
    // ================================================================

    /// 获取当日分时数据 (委托给历史分时 API，避免实时 API 价格编码异常)
    pub fn get_minute_time_data(&self, market: u8, code: &str) -> Result<Vec<MinuteTimePrice>> {
        let today = utils::today_yyyymmdd();
        self.get_history_minute_time_data(market, code, today)
    }

    pub fn get_history_minute_time_data(
        &self,
        market: u8,
        code: &str,
        date: u32,
    ) -> Result<Vec<MinuteTimePrice>> {
        let code_buf = utils::code_bytes(code);
        let mut pkt = Vec::with_capacity(23);
        pkt.extend_from_slice(&[
            0x0c, 0x01, 0x30, 0x00, 0x01, 0x01, 0x0d, 0x00, 0x0d, 0x00, 0xb4, 0x0f,
        ]);
        pkt.extend_from_slice(&date.to_le_bytes());
        pkt.push(market);
        pkt.extend_from_slice(&code_buf);
        parse_history_minute_time_data(&self.send_and_recv(&pkt)?, market, code)
    }

    // ================================================================
    // 逐笔成交
    // ================================================================

    pub fn get_transaction_data(
        &self,
        market: u8,
        code: &str,
        start: u16,
        count: u16,
    ) -> Result<Vec<TickData>> {
        let code_buf = utils::code_bytes(code);
        let mut pkt = Vec::with_capacity(24);
        pkt.extend_from_slice(&[
            0x0c, 0x17, 0x08, 0x01, 0x01, 0x01, 0x0e, 0x00, 0x0e, 0x00, 0xc5, 0x0f,
        ]);
        pkt.extend_from_slice(&(market as u16).to_le_bytes());
        pkt.extend_from_slice(&code_buf);
        pkt.extend_from_slice(&start.to_le_bytes());
        pkt.extend_from_slice(&count.to_le_bytes());
        let coefficient = get_security_coefficient(market, code);
        parse_transaction_data_with_coefficient(&self.send_and_recv(&pkt)?, coefficient)
    }

    pub fn get_history_transaction_data(
        &self,
        market: u8,
        code: &str,
        start: u16,
        count: u16,
        date: u32,
    ) -> Result<Vec<TickData>> {
        let code_buf = utils::code_bytes(code);
        let mut pkt = Vec::with_capacity(28);
        pkt.extend_from_slice(&[
            0x0c, 0x01, 0x30, 0x01, 0x00, 0x01, 0x12, 0x00, 0x12, 0x00, 0xb5, 0x0f,
        ]);
        pkt.extend_from_slice(&date.to_le_bytes());
        pkt.extend_from_slice(&(market as u16).to_le_bytes());
        pkt.extend_from_slice(&code_buf);
        pkt.extend_from_slice(&start.to_le_bytes());
        pkt.extend_from_slice(&count.to_le_bytes());
        let coefficient = get_security_coefficient(market, code);
        parse_history_transaction_data_with_coefficient(&self.send_and_recv(&pkt)?, coefficient)
    }

    // ================================================================
    // 财务 / 除权 / 板块
    // ================================================================

    pub fn get_finance_info(&self, market: u8, code: &str) -> Result<FinanceInfo> {
        let code_buf = utils::code_bytes(code);
        let mut pkt = Vec::with_capacity(21);
        pkt.extend_from_slice(&[
            0x0c, 0x1f, 0x18, 0x76, 0x00, 0x01, 0x0b, 0x00, 0x0b, 0x00, 0x10, 0x00, 0x01, 0x00,
        ]);
        pkt.push(market);
        pkt.extend_from_slice(&code_buf);
        parse_finance_info(&self.send_and_recv(&pkt)?, market, code)
    }

    pub fn get_xdxr_info(&self, market: u8, code: &str) -> Result<Vec<XdXrInfo>> {
        let code_buf = utils::code_bytes(code);
        let mut pkt = Vec::with_capacity(21);
        pkt.extend_from_slice(&[
            0x0c, 0x1f, 0x18, 0x76, 0x00, 0x01, 0x0b, 0x00, 0x0b, 0x00, 0x0f, 0x00, 0x01, 0x00,
        ]);
        pkt.push(market);
        pkt.extend_from_slice(&code_buf);
        parse_xdxr_info_for(&self.send_and_recv(&pkt)?, market, code)
    }

    // ================================================================
    // 板块信息
    // ================================================================

    fn block_info_meta_packet(block_file: &str) -> Vec<u8> {
        let mut name_buf = [0u8; 40];
        let bytes = block_file.as_bytes();
        let len = bytes.len().min(40);
        name_buf[..len].copy_from_slice(&bytes[..len]);

        let mut packet = Vec::with_capacity(52);
        packet.extend_from_slice(&[
            0x0C, 0x39, 0x18, 0x69, 0x00, 0x01, 0x2A, 0x00, 0x2A, 0x00, 0xC5, 0x02,
        ]);
        packet.extend_from_slice(&name_buf);
        packet
    }

    fn block_info_packet(block_file: &str, start: u32, size: u32) -> Vec<u8> {
        let mut name_buf = [0u8; 100];
        let bytes = block_file.as_bytes();
        let len = bytes.len().min(100);
        name_buf[..len].copy_from_slice(&bytes[..len]);

        let mut packet = Vec::with_capacity(120);
        packet.extend_from_slice(&[
            0x0c, 0x37, 0x18, 0x6a, 0x00, 0x01, 0x6e, 0x00, 0x6e, 0x00, 0xb9, 0x06,
        ]);
        packet.extend_from_slice(&start.to_le_bytes());
        packet.extend_from_slice(&size.to_le_bytes());
        packet.extend_from_slice(&name_buf);
        packet
    }

    /// 获取板块元数据
    pub fn get_block_info_meta(&self, block_file: &str) -> Result<BlockInfoMeta> {
        parse_block_info_meta(&self.send_and_recv(&Self::block_info_meta_packet(block_file))?)
    }

    /// 获取板块数据
    pub fn get_block_info(&self, block_file: &str, start: u32, size: u32) -> Result<Vec<u8>> {
        parse_block_info(&self.send_and_recv(&Self::block_info_packet(block_file, start, size))?)
    }

    pub(crate) fn download_stable_block_file(
        &self,
        block_file: &str,
        chunk_size: u32,
        max_size: u32,
    ) -> Result<(BlockInfoMeta, Vec<u8>, BlockInfoMeta)> {
        if chunk_size == 0 || max_size == 0 {
            return Err(TdxError::InvalidData(
                "TDX block download chunk and maximum sizes must be positive".into(),
            ));
        }
        let mut conn = TcpConnection::connect(&self.ip, self.port, self.timeout)?;
        utils::perform_handshake(&mut conn)?;

        let before = parse_block_info_meta(&Self::send_and_recv_on(
            &mut conn,
            &Self::block_info_meta_packet(block_file),
        )?)?;
        if before.size == 0 || before.size > max_size {
            return Err(TdxError::InvalidData(format!(
                "TDX block file {block_file} has invalid source size {}",
                before.size
            )));
        }
        let mut bytes = Vec::with_capacity(before.size as usize);
        let mut offset = 0u32;
        while offset < before.size {
            let requested = chunk_size.min(before.size - offset);
            let chunk = parse_block_info(&Self::send_and_recv_on(
                &mut conn,
                &Self::block_info_packet(block_file, offset, requested),
            )?)?;
            if chunk.len() != requested as usize {
                return Err(TdxError::InvalidData(format!(
                    "TDX block file {block_file} returned a partial chunk at offset {offset}: expected {requested}, received {}",
                    chunk.len()
                )));
            }
            bytes.extend_from_slice(&chunk);
            offset = offset.checked_add(requested).ok_or_else(|| {
                TdxError::InvalidData(format!(
                    "TDX block file {block_file} download offset overflow"
                ))
            })?;
        }
        let after = parse_block_info_meta(&Self::send_and_recv_on(
            &mut conn,
            &Self::block_info_meta_packet(block_file),
        )?)?;
        Ok((before, bytes, after))
    }

    /// 获取并解析板块信息
    pub fn get_and_parse_block_info(
        &self,
        block_file: &str,
    ) -> Result<Vec<crate::reader::block::BlockRecord>> {
        let meta = self.get_block_info_meta(block_file)?;
        let chunk_size: u32 = 0x7530;
        let mut all_data = Vec::new();
        let mut offset = 0u32;

        while offset < meta.size {
            let read_size = chunk_size.min(meta.size - offset);
            let chunk = self.get_block_info(block_file, offset, read_size)?;
            all_data.extend_from_slice(&chunk);
            offset += read_size;
        }

        crate::reader::block::parse_block(&all_data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> TdxDirectClient {
        TdxDirectClient::new("127.0.0.1", 1, 0.01)
    }

    fn source_bar(date: (u32, u32, u32)) -> SecurityBar {
        SecurityBar {
            open: 10.0,
            close: 10.5,
            high: 11.0,
            low: 9.5,
            vol: 100.0,
            amount: 1_000.0,
            year: date.0,
            month: date.1,
            day: date.2,
            hour: 0,
            minute: 0,
            datetime: format!("{:04}-{:02}-{:02}", date.0, date.1, date.2),
        }
    }

    fn source_xdxr(date: (u32, u32, u32), category: u32) -> XdXrInfo {
        XdXrInfo {
            year: date.0,
            month: date.1,
            day: date.2,
            category,
            name: "fixture".into(),
            fenhong: None,
            peigujia: None,
            songzhuangu: None,
            peigu: None,
            suogu: None,
            panqianliutong: None,
            panhouliutong: None,
            qianzongguben: None,
            houzongguben: None,
            fenshu: None,
            xingquanjia: None,
        }
    }

    #[test]
    fn configuration_and_factor_context_boundaries_are_offline() {
        let mut client = client();
        client.set_server("127.0.0.2", 2);
        client.set_timeout(0.02);
        client.set_server("127.0.0.1", 1);
        client.set_timeout(0.01);

        for tier in [
            utils::FqContextTier::Low,
            utils::FqContextTier::Mid,
            utils::FqContextTier::High,
        ] {
            client.set_fq_context_tier(tier);
            assert_eq!(client.fq_context_tier(), tier);
        }

        let recent = source_bar((2026, 7, 25));
        let event = source_xdxr((2025, 7, 25), 1);
        assert!(client
            .fetch_context_for_factors(KLINE_DAILY, 1, "600001", &[], std::slice::from_ref(&event),)
            .unwrap()
            .is_empty());
        assert!(client
            .fetch_context_for_factors(KLINE_DAILY, 1, "600001", std::slice::from_ref(&recent), &[])
            .unwrap()
            .is_empty());
        assert!(client
            .fetch_context_for_factors(
                KLINE_DAILY,
                1,
                "600001",
                std::slice::from_ref(&recent),
                &[source_xdxr((2025, 7, 25), 2)]
            )
            .unwrap()
            .is_empty());
        assert!(client
            .fetch_context_for_factors(
                KLINE_DAILY,
                1,
                "600001",
                &[source_bar((2024, 7, 25))],
                std::slice::from_ref(&event)
            )
            .unwrap()
            .is_empty());
        assert!(client
            .fetch_context_for_factors(KLINE_DAILY, 1, "600001", &[recent], &[event])
            .unwrap()
            .is_empty());
    }

    #[test]
    fn every_direct_request_builder_propagates_local_connection_failure() {
        let client = client();
        assert!(client
            .get_security_bars(KLINE_DAILY, 1, "600001", 0, 5, 0)
            .is_err());
        assert!(client
            .get_security_bars(KLINE_DAILY, 1, "880001", 0, 5, 0)
            .is_err());
        assert!(client
            .get_index_bars(KLINE_DAILY, 1, "000001", 0, 5, 1)
            .is_err());
        assert!(client
            .get_index_bars_inner(KLINE_DAILY, 1, "880001", 0, 5, 1)
            .is_err());
        assert!(client.get_security_quotes(&[(1, "600001")]).is_err());
        assert!(client.get_security_quotes_inner(&[(1, "880001")]).is_err());
        assert!(client.get_security_list(1, 0).is_err());
        assert!(client.get_security_count(1).is_err());
        assert!(client.get_minute_time_data(1, "600001").is_err());
        assert!(client
            .get_history_minute_time_data(1, "600001", 20_260_725)
            .is_err());
        assert!(client.get_transaction_data(1, "600001", 0, 5).is_err());
        assert!(client
            .get_history_transaction_data(1, "600001", 0, 5, 20_260_725)
            .is_err());
        assert!(client.get_finance_info(1, "600001").is_err());
        assert!(client.get_xdxr_info(1, "600001").is_err());
        assert!(client.get_block_info_meta("block.dat").is_err());
        assert!(client.get_block_info("block.dat", 0, 10).is_err());
        assert!(client.get_and_parse_block_info("block.dat").is_err());
    }
}
