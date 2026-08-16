//! Bearer token 认证 (合同 §9: 不加密的 metadata 认证)。
//! token 只进 metadata, 不进请求体/URL/日志。
use tonic::metadata::MetadataValue;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum AuthError {
    #[error("GRPC_MARKET_TOKEN 包含非法字符, 无法注入 metadata")]
    InvalidTokenValue,
}

/// 从 GRPC_MARKET_TOKEN 读 token 并注入 authorization metadata。
/// token 未设置 → 不注入 (dev 服务端明文接受; 真实服务端对接时必须设置)。
pub fn attach_bearer<T>(request: &mut tonic::Request<T>) -> Result<(), AuthError> {
    let Ok(token) = std::env::var("GRPC_MARKET_TOKEN") else {
        return Ok(());
    };
    attach_bearer_value(request, &token)
}

/// 注入由客户端实例持有的 token。
///
/// `client-bundle` 的凭据不得写回进程环境；多客户端并存时，共享环境变量也会造成
/// 认证串线。该入口只把调用方提供的值放入当前请求 metadata，不记录 token。
pub fn attach_bearer_value<T>(
    request: &mut tonic::Request<T>,
    token: &str,
) -> Result<(), AuthError> {
    let value = format!("Bearer {token}");
    let metadata =
        MetadataValue::try_from(value.as_str()).map_err(|_| AuthError::InvalidTokenValue)?;
    request.metadata_mut().insert("authorization", metadata);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // 环境变量是进程级的, 而 cargo test 默认并行跑测试 →
    // 所有 env 操作必须串行化, 否则两个测试互相踩 set_var/remove_var。
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_token(token: Option<&str>, f: impl FnOnce()) {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            match token {
                Some(t) => std::env::set_var("GRPC_MARKET_TOKEN", t),
                None => std::env::remove_var("GRPC_MARKET_TOKEN"),
            }
        }
        f();
        unsafe {
            std::env::remove_var("GRPC_MARKET_TOKEN");
        }
    }

    #[test]
    fn injects_bearer_when_token_set() {
        with_token(Some("secret-token"), || {
            let mut req = tonic::Request::new(());
            attach_bearer(&mut req).unwrap();
            let auth = req
                .metadata()
                .get("authorization")
                .unwrap()
                .to_str()
                .unwrap();
            assert_eq!(auth, "Bearer secret-token");
        });
    }

    #[test]
    fn no_op_when_token_unset() {
        with_token(None, || {
            let mut req = tonic::Request::new(());
            attach_bearer(&mut req).unwrap();
            assert!(req.metadata().get("authorization").is_none());
        });
    }

    #[test]
    fn injects_instance_owned_bearer_without_environment_mutation() {
        with_token(None, || {
            let mut req = tonic::Request::new(());
            attach_bearer_value(&mut req, "TEST_CODE_bundle_token").unwrap();
            let auth = req
                .metadata()
                .get("authorization")
                .unwrap()
                .to_str()
                .unwrap();
            assert_eq!(auth, "Bearer TEST_CODE_bundle_token");
            assert!(std::env::var("GRPC_MARKET_TOKEN").is_err());
        });
    }
}
