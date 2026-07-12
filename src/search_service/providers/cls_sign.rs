//! 财联社 (CLS) 请求签名。
//!
//! BR-037: CLS/KCB 签名与快讯池同源配额规则。
//! 参照 RSSHub `/cls` route (MIT, github.com/DIYgod/RSSHub, lib/routes/cls/utils.ts)：
//! 1) 固定追加 appName=CailianpressWeb, os=web, sv=<版本>
//! 2) 全部参数按 key 字典序排序并做 querystring 编码
//! 3) sign = MD5_hex(SHA1_hex(querystring))
//! 4) 追加 sign 参数

use md5::{Digest as Md5Digest, Md5};
use sha1::Sha1;

/// CLS web 版本号（改版时只改这里）。
pub const CLS_SV: &str = "8.7.9";

/// 对给定业务参数生成 CLS 完整签名参数（含 app/os/sv/sign）。
///
/// 返回值是按 key 排序后的参数数组，调用方可直接 `.query(&params)`。
pub fn build_signed_params(business: &[(&str, String)]) -> Vec<(String, String)> {
    let mut params: Vec<(String, String)> = business
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();

    params.push(("appName".to_string(), "CailianpressWeb".to_string()));
    params.push(("os".to_string(), "web".to_string()));
    params.push(("sv".to_string(), CLS_SV.to_string()));
    params.sort_by(|a, b| a.0.cmp(&b.0));

    let query_string = serde_urlencoded::to_string(&params).unwrap_or_default();

    let sha1_hex = {
        let mut hasher = Sha1::new();
        hasher.update(query_string.as_bytes());
        hex::encode(hasher.finalize())
    };

    let sign = {
        let mut hasher = Md5::new();
        hasher.update(sha1_hex.as_bytes());
        hex::encode(hasher.finalize())
    };

    params.push(("sign".to_string(), sign));
    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_is_deterministic_and_appended() {
        let p1 = build_signed_params(&[("name", "telegraph".to_string())]);
        let p2 = build_signed_params(&[("name", "telegraph".to_string())]);

        let s1 = p1
            .iter()
            .find(|(k, _)| k == "sign")
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        let s2 = p2
            .iter()
            .find(|(k, _)| k == "sign")
            .map(|(_, v)| v.clone())
            .unwrap_or_default();

        assert_eq!(s1, s2);
        assert_eq!(s1.len(), 32);
        assert!(s1.chars().all(|c| c.is_ascii_hexdigit()));

        assert!(p1
            .iter()
            .any(|(k, v)| k == "appName" && v == "CailianpressWeb"));
        assert!(p1.iter().any(|(k, v)| k == "os" && v == "web"));
        assert!(p1.iter().any(|(k, _)| k == "sv"));
    }

    #[test]
    fn params_are_sorted_before_sign() {
        let p = build_signed_params(&[("zzz", "1".to_string()), ("aaa", "2".to_string())]);
        let keys: Vec<&str> = p
            .iter()
            .map(|(k, _)| k.as_str())
            .filter(|k| *k != "sign")
            .collect();

        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
    }
}
