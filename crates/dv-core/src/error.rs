//! 统一错误类型。
//!
//! 刻意不依赖 `reqwest` / `rusqlite`：底层实现把各自的错误转成这里的变体，
//! 这样 `dv-core` 保持零 IO 依赖，测试替身也不用引入 HTTP 栈。

use std::fmt;

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("网络错误: {0}")]
    Network(String),

    /// B 站业务错误（`code != 0`）。`retryable` 由调用方按错误码判定。
    #[error("接口错误 {code}: {msg}")]
    Api { code: i32, msg: String },

    #[error("解析失败: {0}")]
    Parse(String),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("数据库错误: {0}")]
    Db(String),

    #[error("需要登录：cookie 缺失或已失效")]
    Auth,

    #[error("被限流或风控拦截（-412）")]
    RateLimited,

    #[error("非法 ID: {0}")]
    InvalidId(String),

    #[error("数据损坏: {0}")]
    Corrupt(String),

    #[error("{0}")]
    Other(String),
}

impl VaultError {
    /// 是否值得重试：网络抖动、5xx、限流类。
    pub fn is_retryable(&self) -> bool {
        match self {
            VaultError::Network(_) | VaultError::RateLimited => true,
            VaultError::Api { code, .. } => matches!(code, -412 | -509 | -500 | -503 | -504),
            _ => false,
        }
    }

    /// 是否属于「这个视频/分 P 没救了，跳过别再试」。
    pub fn is_permanent_skip(&self) -> bool {
        match self {
            VaultError::Api { code, .. } => matches!(code, -404 | 62002 | 62004 | 62012),
            _ => false,
        }
    }
}

impl From<std::fmt::Error> for VaultError {
    fn from(value: fmt::Error) -> Self {
        VaultError::Other(value.to_string())
    }
}

pub type Result<T, E = VaultError> = std::result::Result<T, E>;
