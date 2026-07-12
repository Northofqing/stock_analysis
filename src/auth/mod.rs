//! CLI operator 密码认证
//!
//! monitor / winrate_simulator / live CLI 启动前调用 [`operator::require_monitor_operator_auth`],
//! 通过系统 PAM 验证 operator 密码. 失败 → exit 1, 阻止 monitor 启动.

pub mod operator;
