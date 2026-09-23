use crate::error::Result;
use crate::error_codes::ErrorCode;

/// Response header: 16 bytes little-endian <IIIHH
/// (seq, method, _, zip_size, unzip_size)
#[derive(Debug, Clone)]
pub struct ResponseHeader {
    pub seq: u32,
    pub method: u32,
    pub zip_size: u32,
    pub unzip_size: u32,
}

pub const RSP_HEADER_LEN: usize = 16;

impl ResponseHeader {
    pub fn parse(buf: &[u8]) -> Result<Self> {
        if buf.len() < RSP_HEADER_LEN {
            return Err(ErrorCode::RESPONSE_HEADER_INVALID.err(format!(
                "expected {} bytes, got {}",
                RSP_HEADER_LEN,
                buf.len()
            )));
        }
        // <IIIHH: seq(u32), method(u32), _(u32), zip_size(u16), unzip_size(u16)
        let seq = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let method = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let _ = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
        let zip_size = u16::from_le_bytes([buf[12], buf[13]]) as u32;
        let unzip_size = u16::from_le_bytes([buf[14], buf[15]]) as u32;
        Ok(Self {
            seq,
            method,
            zip_size,
            unzip_size,
        })
    }
}

#[cfg(test)]
#[path = "../../tests/internal/net_packet.rs"]
mod tests;
