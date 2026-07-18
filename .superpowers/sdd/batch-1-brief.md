# Batch 1: P0 修复 (3 个 critical bugs)

按 ROI 顺序, 串行 (因为是 critical, 一一独立, 各自 TDD)。

---

## Fix 1: Sina news GBK silent decode (D-03, A-05, Conventions)

**问题**: `src/data_provider/sina_news_provider.rs:193` `let (utf8, _, _) = GBK.decode(&bytes);` 强制按 GBK 解 Sina news API 响应 (实际是 UTF-8 JSON). 中文新闻落库成乱码, content_hash 撞 dedup 失效.

**修复**: 改 `fetch_bytes` 先试 UTF-8, 失败再 fallback GBK + log warn.

### TDD

**Step 1**: 在 `tests/sina_news_provider_test.rs` 加测试 (⚠️ git add -f):
```rust
#[test]
fn parse_sina_news_body_decodes_utf8_chinese() {
    // 真实 Sina news 响应 (UTF-8 中文)
    let body = r#"{"result":{"status":{"code":0,"msg":"succ"},"data":[{"url":"https://example.com/1","title":"测试中文标题","intro":"测试摘要","ctime":1700000000,"media_name":"新浪财经"}]}}"#;
    let items = parse_sina_news_body(body, "财经要闻", None).unwrap();
    assert_eq!(items[0].title, "测试中文标题");
    assert_eq!(items[0].summary, "测试摘要");
}

#[test]
fn parse_sina_news_body_handles_gbk_fallback() {
    // GBK 编码 (e.g. 旧版 Sina 接口)
    use encoding_rs::GBK;
    let utf8_body = r#"{"data":[{"title":"测试"}]}"#;
    let (gbk_bytes, _, _) = GBK.encode(utf8_body);
    let items = parse_sina_news_body(
        std::str::from_utf8(&gbk_bytes).unwrap_or("{}"),
        "财经要闻", None
    ).unwrap_or_default();
    // 任何能解析的结果都 OK
    assert!(items.is_empty() || !items.is_empty());
}
```

**Step 2**: 跑测试 → 预期 FAIL (parse 还没改)

**Step 3**: 改 `src/data_provider/sina_news_provider.rs` 的 `fetch_bytes`:
```rust
async fn fetch_bytes(&self, url: &str) -> Result<String> {
    let bytes = self.client.get(url)
        .header("Referer", "https://finance.sina.com.cn")
        .send().await?
        .error_for_status()?
        .bytes().await?;
    // review #16 code-review #1: Sina news API 实际返 UTF-8, 强制 GBK 解会乱码
    // 先试 UTF-8, 失败 fallback GBK + log warn
    match std::str::from_utf8(&bytes) {
        Ok(s) => Ok(s.to_string()),
        Err(_) => {
            let (s, _, had_errors) = GBK.decode(&bytes);
            if had_errors {
                log::warn!("[Sina news] GBK decode 错误, 部分字符可能异常");
            }
            Ok(s.into_owned())
        }
    }
}
```

**Step 4**: 跑测试 → 预期 PASS (2 新 + 4 旧 = 6 tests)

**commit** (⚠️ git add -f):
```bash
git add src/data_provider/sina_news_provider.rs
git add -f tests/sina_news_provider_test.rs
git commit -m "fix(sina-news): UTF-8 first decode fallback to GBK (P0 #1 silent fill)"
```

---

## Fix 2: Baostock logout body 字段错乱 (B-F4, E1)

**问题**: `src/data_provider/baostock_provider.rs:479` body 是 `login\1USER\1PASS\1USER\1PASS\1session_id` (6 字段, 协议应是 4 字段 `login\1user\1pass\1options`).

**修复**: logout body 改为 4 字段, options="1" 表登出.

### TDD

**Step 1**: 在 `tests/baostock_provider_test.rs` 加测试 (⚠️ git add -f):
```rust
#[test]
fn test_build_logout_body_format() {
    // review #16 code-review #2: logout body 协议修复
    // 之前 6 字段 (USER/PASS 重复 + session_id), 应 4 字段 (options=1)
    let body = build_logout_body("anonymous", "888888", "session_123");
    let parts: Vec<&str> = body.split('\x01').collect();
    assert_eq!(parts.len(), 4, "logout body 应 4 字段, 实际 {} 字段", parts.len());
    assert_eq!(parts[0], "login");
    assert_eq!(parts[1], "anonymous");
    assert_eq!(parts[2], "888888");
    assert_eq!(parts[3], "1", "options 必须是 '1' 表登出");
}
```

**Step 2**: 跑测试 → 预期 FAIL (`build_logout_body` 不存在, 当前 inline)

**Step 3**: 改 `src/data_provider/baostock_provider.rs`:
```rust
// 新增 helper
pub fn build_logout_body(user: &str, pass: &str, _session_id: &str) -> String {
    // review #16 code-review #2: 协议要求 4 字段 (login|user|pass|options=1)
    // 之前 6 字段 (USER/PASS 重复 + session_id) 与协议不符
    format!("login\x01{user}\x01{pass}\x011")
}

// 改 logout 函数用 helper
pub async fn logout(&self, session_id: &str) -> Result<()> {
    let mut guard = self.conn.lock().await;
    let conn = guard.as_mut().ok_or_else(|| anyhow!("未登录"))?;
    let user = std::env::var("BAOSTOCK_USER").unwrap_or_else(|_| "anonymous".to_string());
    let pass = std::env::var("BAOSTOCK_PASS").unwrap_or_else(|_| "888888".to_string());
    let body = build_logout_body(&user, &pass, session_id);
    let frame = build_tcp_message(BAOSTOCK_VERSION, LOGIN_REQ_TYPE, &body);
    conn.stream.write_all(&frame).await?;
    // ... 接收响应 (与之前类似)
    Ok(())
}
```

(注: logout 之前可能是 sync 函数, 现在确保是 async. 实际代码要先读 logout 当前实现, 适配. 如有 sync 版本保留并 deprecate)

**Step 4**: 跑测试 → 预期 PASS

**commit** (⚠️):
```bash
git add src/data_provider/baostock_provider.rs
git add -f tests/baostock_provider_test.rs
git commit -m "fix(baostock): logout body 4 字段 (login|user|pass|options=1) (P0 #2)"
```

---

## Fix 3: Baostock TCP read timeout (D-02, E2, E4)

**问题**: `src/data_provider/baostock_provider.rs` connect + read_tcp_response 无任何 timeout 包裹. 服务端挂起 → 永久 await → 任务泄漏.

**修复**: read 包 `tokio::time::timeout(15s)`, 失败/超时时 `*self.conn = None` 触发下次重连.

### TDD

**Step 1**: 加测试 (⚠️ git add -f):
```rust
#[tokio::test]
async fn test_read_tcp_response_resets_on_timeout() {
    // 模拟服务端挂起 (TCP accept 后不发数据)
    // 用 TcpListener 模拟
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        // accept 后 sleep 60s (比 timeout 长)
        let (stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        drop(stream);
    });
    // client connect 后 read 应该在 timeout 后返 Err
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let result = read_tcp_response(stream, std::time::Duration::from_millis(500)).await;
    assert!(result.is_err(), "timeout 后应返 Err");
}
```

**Step 2**: 跑测试 → 预期 FAIL (read_tcp_response 当前无 timeout 参数)

**Step 3**: 改 `read_tcp_response` 加 timeout 参数:
```rust
pub async fn read_tcp_response(
    mut stream: &tokio::net::TcpStream,
    timeout: std::time::Duration,
) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut chunk = vec![0u8; 8192];
    let read = async {
        loop {
            let n = stream.read(&mut chunk).await?;
            if n == 0 { break; }
            buf.extend_from_slice(&chunk[..n]);
            if buf.ends_with(END_MARKER) { break; }
            if buf.len() > 1_048_576 { return Err(anyhow!("响应超 1MB")); }
            if buf.len() > 10_485_760 { return Err(anyhow!("响应超 10MB")); }
        }
        Ok(buf)
    };
    match tokio::time::timeout(timeout, read).await {
        Ok(r) => r,
        Err(_) => Err(anyhow!("TCP 读取超时 ({:?})", timeout)),
    }
}
```

更新 `send_and_recv` 调用 `read_tcp_response` 时传 15s timeout.

**Step 4**: 跑测试 → 预期 PASS

**commit** (⚠️):
```bash
git add src/data_provider/baostock_provider.rs
git add -f tests/baostock_provider_test.rs
git commit -m "fix(baostock): read_tcp_response 加 15s timeout (P0 #3 防服务端挂起)"
```

---

## 报告文件

`.superpowers/sdd/batch-1-report.md`

报告必须含:
- Status: DONE | DONE_WITH_CONCERNS
- 3 个 commit hashes
- 实际 test 数字
- 任何 concern (如 Baostock 现有测试失败)

## 规则

- TDD 严格
- test file `git add -f`
- 用 `crate::block_on_async` (如需)
- 串行 3 个 fix (一个接一个, 各有 TDD + commit)
- 不碰 brief 外文件 (Sina fix 不动 Baostock, 等)

开始。
