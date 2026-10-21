# push2 族网络不可达证据（2026-10-18）

范围：`push2.eastmoney.com`、`push2delay.eastmoney.com` 及其兄弟宿主。
用途：向 Mac 侧说明为什么 BoardFlows/PostCloseFlows/MarketRankings/Popularity/
ProviderTopN 与 FundFlowSeries 分钟线拿不到，以及为什么这不是本仓能修的缺陷。

## 1. 实测（Windows 侧，2026-10-18 16:2x Asia/Shanghai）

| 目标 | DNS | TLS/HTTP 结果 |
| --- | --- | --- |
| push2.eastmoney.com | 101.226.30.206（CNAME → push2ipv6.trafficmanager.cn） | curl 56 `schannel: server closed abruptly (missing close_notify)`，code=000 |
| push2delay.eastmoney.com | 61.152.229.217 | 同上，code=000 |
| push2.eastmoney.com（`--resolve 101.226.30.206`） | 强制真实 IP | 同样 code=000，排除纯 DNS 因素 |
| push2his.eastmoney.com | 61.129.129.199 | **HTTP 200**（同族对照组可达） |

命令（原样复现）：

```bash
nslookup push2.eastmoney.com
curl -sS -m 12 -o /dev/null -w '%{http_code} %{remote_ip}\n' \
  'https://push2.eastmoney.com/api/qt/clist/get?pn=1&pz=5&po=1&np=1&fltt=2&invt=2&fid=f3&fs=m:90+t:2&fields=f12,f14'
curl -sS -m 12 --resolve push2.eastmoney.com:443:101.226.30.206 -o /dev/null \
  -w '%{http_code}\n' 'https://push2.eastmoney.com/api/qt/clist/get?pn=1&pz=5'
curl -sS -m 12 -o /dev/null -w '%{http_code} %{remote_ip}\n' \
  'https://push2his.eastmoney.com/api/qt/stock/kline/get?secid=1.600519&fields1=f1&fields2=f51&klt=101&fqt=1&lmt=5'
```

## 2. 结论修正

- 2026-10-16 观察到的 `198.18.49.0` 是**本机 DNS/代理路径的 fake-IP**，不是东财侧固定解析；
  2026-10-18 已解析到真实 IP，但 TLS 仍在握手阶段被对端断开。
- 因此故障点在 **TLS/边缘策略或本机到该边缘的网络路径**，不是 URL、不是 UA、不是本仓
  HTTP 客户端配置。伪装 UA/绕证书不在仓规允许范围内，不做。
- 同族 `push2his` 正常，说明不是整个 `eastmoney.com` 被拒。

## 3. 影响面（与现有 op 对应）

| 能力 | 受影响 op |
| --- | --- |
| BoardFlows | 36 |
| PostCloseFlows | 42 |
| MarketRankings | 49 |
| Popularity | 51 |
| ProviderTopN | 54 |
| FundFlowSeries 分钟线 | 相关资金流路径 |
| R-09 资金流证据链 | — |

不影响的对照组：`push2his`（历史 K 线）、`push2ex`、`datacenter-web`、`gitee`
此前均 200；本次只复测 `push2his`。

## 4. 下一步（需要跨机配合）

1. 在 Mac 侧对同一 URL 做一次同样的 curl，判定是"本机路径"还是"东财对该出口的策略"。
2. 若 Mac 侧可达，可评估把该族请求放到 Mac 侧执行，或为本机换一条出口；
   这属于部署拓扑变更，不在本仓代码范围内。
3. 在该族恢复前，消费方应按"能力不可用"处理，不得用其它族数据补值。

本文件只记录实测与归属，不承诺修复时间。
