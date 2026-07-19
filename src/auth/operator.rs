//! Registered business rules: BR-074.
//! CLI operator 密码认证 (PAM / 系统用户)
//!
//! monitor / winrate_simulator / live CLI 启动前调用 [`require_monitor_operator_auth`],
//! 通过系统 PAM 验证 operator 密码. 失败 → exit 1, 阻止 monitor 启动.
//!
//! 默认禁用: 单机 single-user 友好, 不强制密码 (与 README + BR-028 保持一致).
//! opt-in 启用: 显式设 `MONITOR_AUTH_REQUIRED=1` 启用 PAM 认证闸.
//!
//! Env config (类似 `NotificationConfig::from_env` 风格, src/notification/config.rs:100-134):
//!   MONITOR_OPERATOR    默认用户名, fallback 到当前 Unix user via whoami
//!   MONITOR_PAM_SERVICE 默认 "login"
//!   MONITOR_AUTH_REQUIRED 默认 unset/0 → 跳过 (认证关闭), 设 "1" → 启用 PAM 认证
//!
//! 失败行为:
//!   - 3 次密码错 → exit 1 (PAM fail_delay 系统级锁定阈值对齐)
//!   - 无 TTY (CI/CD) → exit 1
//!   - PAM service 不存在 / 不可用 → exit 1
//!   - account locked / expired → fail closed (PAM 拒绝)

use std::io::{self, IsTerminal, Write};

pub const MAX_ATTEMPTS: usize = 3;

#[derive(Debug, Clone)]
pub struct OperatorAuthConfig {
    /// 严格匹配 (不是建议): 期望被认证的 Unix username
    pub expected_operator: String,
    /// PAM service 名 (默认 "login")
    pub pam_service: String,
    /// true = 必须通过, false = 仅 log (仅开发用)
    pub required: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum OperatorAuthError {
    #[error("无 TTY, 拒绝启动 monitor (需交互式终端输入密码)")]
    NoTty,
    #[error("认证失败次数过多 (>{n} 次), 拒绝启动", n = MAX_ATTEMPTS)]
    TooManyFailures,
    #[error("PAM 初始化失败: {0}")]
    PamInit(#[from] pam::PamError),
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
}

/// 从 env 加载配置
///
/// 默认禁用: 单机 single-user 不要求密码 (不打扰)
/// opt-in 启用: `MONITOR_AUTH_REQUIRED=1` 启用 PAM 认证闸
pub fn load_auth_config() -> OperatorAuthConfig {
    OperatorAuthConfig {
        expected_operator: std::env::var("MONITOR_OPERATOR")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(whoami::username),
        pam_service: std::env::var("MONITOR_PAM_SERVICE").unwrap_or_else(|_| "login".to_string()),
        // CR-AUTH 默认禁用: 单机 single-user 友好 (不打扰), 生产 cron 显式 MONITOR_AUTH_REQUIRED=1 启用
        // "1" → true (要求 PAM 认证); 其它 (unset / "0" / 其他值) → false (跳过)
        required: std::env::var("MONITOR_AUTH_REQUIRED")
            .map(|v| v == "1")
            .unwrap_or(false),
    }
}

/// CR-AUTH 入口: monitor / CLI 启动前调用, 失败则 exit 1
///
/// 流程:
/// 1. 加载 env 配置 (MONITOR_OPERATOR / MONITOR_PAM_SERVICE / MONITOR_AUTH_REQUIRED)
/// 2. 如果 `MONITOR_AUTH_REQUIRED` 未设或 != "1" → log debug 跳过 (默认禁用, opt-in 启用)
/// 3. 检查 TTY: stdin/stdout 任一非 terminal → 拒绝 (CI/CD 友好)
/// 4. 最多 3 次尝试:
///    - 隐藏式 prompt 读 password
///    - PAM authenticate(expected_operator, password)
///    - 成功 → log + return Ok
///    - 失败 → log warn, 重试
/// 5. 3 次全失败 → return TooManyFailures
pub fn require_monitor_operator_auth() -> Result<(), OperatorAuthError> {
    let cfg = load_auth_config();
    if !cfg.required {
        // CR-AUTH 默认禁用 (单机 single-user 友好). 显式 MONITOR_AUTH_REQUIRED=1 启用.
        // FIX-B: 默认禁用时 eprintln 醒目提示, 让生产 cron 忘了设 env 时可见, 而非仅 log::debug
        eprintln!(
            "[CR-AUTH] ⚠️  operator 认证已禁用 (MONITOR_AUTH_REQUIRED 未设). \
             生产 cron / 共享主机 显式设 MONITOR_AUTH_REQUIRED=1 启用."
        );
        log::debug!("[CR-AUTH] MONITOR_AUTH_REQUIRED 未启用, 跳过认证");
        return Ok(());
    }

    // 必须 TTY (fail closed, 防 CI/CD 静默跳过)
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(OperatorAuthError::NoTty);
    }

    eprintln!(
        "[CR-AUTH] monitor 启动前需 operator 认证: 用户 '{}' (PAM service '{}')",
        cfg.expected_operator, cfg.pam_service
    );

    for attempt in 1..=MAX_ATTEMPTS {
        eprint!("  password: ");
        io::stdout().flush().ok();
        let password = rpassword::read_password()?;

        let result = try_pam_auth(&cfg, &password);

        // 密码 buffer best-effort 擦除 (zeroize crate 限制, 实际效果取决于平台 + 是否真 drop)
        drop(password);

        match result {
            Ok(()) => {
                eprintln!(
                    "[CR-AUTH] operator '{}' 认证通过 via PAM service '{}'",
                    cfg.expected_operator, cfg.pam_service
                );
                return Ok(());
            }
            Err(e) => {
                log::warn!(
                    "[CR-AUTH] 认证失败 (attempt {}/{}): {:?}",
                    attempt,
                    MAX_ATTEMPTS,
                    e
                );
                if attempt < MAX_ATTEMPTS {
                    eprintln!("  [CR-AUTH] 密码错, 还剩 {} 次尝试", MAX_ATTEMPTS - attempt);
                }
            }
        }
    }
    Err(OperatorAuthError::TooManyFailures)
}

/// PAM authenticate 封装 (单独函数便于测试 mock)
#[cfg(unix)]
fn try_pam_auth(cfg: &OperatorAuthConfig, password: &str) -> Result<(), pam::PamError> {
    // pam 0.7 API: Authenticator::with_password(service) 返回 PasswordConv
    // 然后 get_handler().set_credentials(user, pass), 最后 authenticate()
    let mut auth = pam::Authenticator::with_password(&cfg.pam_service)?;
    auth.get_handler()
        .set_credentials(&cfg.expected_operator, password);
    auth.authenticate()
}

/// 非 Unix 平台: 编译通过, 但认证 fail closed
#[cfg(not(unix))]
fn try_pam_auth(_cfg: &OperatorAuthConfig, _password: &str) -> Result<(), pam::PamError> {
    // 真实项目应走此分支, 但 `pam` crate 限定 cfg(unix), 实际不会到这
    Err(pam::PamError::UNKNOWN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial]
    fn config_uses_monitor_operator_when_set() {
        std::env::set_var("MONITOR_OPERATOR", "testuser");
        std::env::remove_var("MONITOR_PAM_SERVICE");
        std::env::remove_var("MONITOR_AUTH_REQUIRED");
        let cfg = load_auth_config();
        assert_eq!(cfg.expected_operator, "testuser");
        // FIX-C: 清理避免污染后续测试 (#[serial] 序列化但 env leak 仍影响顺序)
        std::env::remove_var("MONITOR_OPERATOR");
    }

    #[test]
    #[serial]
    fn config_pam_service_defaults_to_login() {
        std::env::remove_var("MONITOR_PAM_SERVICE");
        let cfg = load_auth_config();
        assert_eq!(cfg.pam_service, "login");
    }

    #[test]
    #[serial]
    fn config_pam_service_uses_env_override() {
        // 显式清空 MONITOR_AUTH_REQUIRED (前一个测试残留) + 设新值
        std::env::remove_var("MONITOR_AUTH_REQUIRED");
        std::env::set_var("MONITOR_PAM_SERVICE", "stock_analysis");
        let cfg = load_auth_config();
        assert_eq!(cfg.pam_service, "stock_analysis");
        // 清理避免污染后续测试
        std::env::remove_var("MONITOR_PAM_SERVICE");
    }

    #[test]
    #[serial]
    fn config_required_defaults_false() {
        // CR-AUTH: 默认禁用, opt-in 启用 (单机 single-user 友好)
        std::env::remove_var("MONITOR_AUTH_REQUIRED");
        std::env::remove_var("MONITOR_PAM_SERVICE");
        let cfg = load_auth_config();
        assert!(!cfg.required, "默认应该禁用认证");
    }

    #[test]
    #[serial]
    fn config_required_one_enables() {
        // 设 "1" 启用 (opt-in)
        std::env::remove_var("MONITOR_PAM_SERVICE");
        std::env::set_var("MONITOR_AUTH_REQUIRED", "1");
        let cfg = load_auth_config();
        assert!(cfg.required, "MONITOR_AUTH_REQUIRED=1 应启用认证");
        // 清理避免污染后续测试
        std::env::remove_var("MONITOR_AUTH_REQUIRED");
    }

    #[test]
    #[serial]
    fn config_required_zero_disables() {
        // 显式 "0" 也禁用
        std::env::remove_var("MONITOR_PAM_SERVICE");
        std::env::set_var("MONITOR_AUTH_REQUIRED", "0");
        let cfg = load_auth_config();
        assert!(!cfg.required);
        // 清理避免污染后续测试
        std::env::remove_var("MONITOR_AUTH_REQUIRED");
    }

    #[test]
    #[serial]
    fn max_attempts_is_three() {
        // 文档化: 3 次失败后 exit 1 (与 PAM 系统级 fail_delay 锁定阈值对齐)
        assert_eq!(MAX_ATTEMPTS, 3);
    }

    #[test]
    #[serial]
    fn empty_env_var_falls_back_to_whoami() {
        std::env::set_var("MONITOR_OPERATOR", ""); // 空字符串应被 filter 掉
        let cfg = load_auth_config();
        // 不空, 来自 whoami (当前 Unix user)
        assert!(!cfg.expected_operator.is_empty());
    }
}
