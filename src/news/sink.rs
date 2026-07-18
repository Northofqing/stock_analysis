//! v15.3 D6 wire: NewsSink — mpsc channel 桥接 sync sink ↔ async push_loop

use crate::news::dispatcher::NewsPush;
use tokio::sync::mpsc;

/// 全局 sender (lib 内 sink.send() 用)
static PUSH_TX: std::sync::Mutex<Option<mpsc::UnboundedSender<NewsPush>>> =
    std::sync::Mutex::new(None);

/// 安装并返回 receiver 给 push_loop
pub fn install() -> mpsc::UnboundedReceiver<NewsPush> {
    let (tx, rx) = mpsc::unbounded_channel();
    if let Ok(mut g) = PUSH_TX.lock() {
        *g = Some(tx);
    }
    rx
}

pub fn try_push(np: &NewsPush) -> bool {
    match PUSH_TX.lock() {
        Ok(g) => match g.as_ref() {
            Some(tx) => tx.send(np.clone()).is_ok(),
            None => {
                log::debug!("[news::sink] PUSH_TX 未注入");
                false
            }
        },
        Err(p) => {
            log::warn!("[news::sink] PUSH_TX lock poisoned, push dropped");
            drop(p.into_inner());
            false
        }
    }
}

pub fn is_installed() -> bool {
    PUSH_TX.lock().map(|g| g.is_some()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::market_event::Direction;

    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_install_creates_pair() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let _rx = install();
        assert!(is_installed());
    }

    #[test]
    fn test_try_push_succeeds() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let _rx = install();
        let ok = try_push(&NewsPush {
            text: "abc".into(),
            headline: "h".into(),
            code: Some("TEST_CODE_000001".into()),
            score: 80.0,
            direction: Direction::Bull,
        });
        assert!(ok);
    }
}
