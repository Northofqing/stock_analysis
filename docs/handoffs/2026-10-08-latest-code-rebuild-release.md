# 最新代码重新构建发布（2026-10-08）

用户明确要求重新用最新代码构建发布。本次源码为 `90017feb0bc8e1d2a397cee7cce1e7661b83d5e9`，已推master并核对远端。相对上次发布，产品源码仅 `backfill_predictions` 入口改为 `DatabaseManager::init_retained_monitor`；验证、交易日、资格及CAS逻辑保持。新提交的21:17日常验证plist与已注册任务一致。

## 制品与验证

Desktop外候选目录冻结1717个Git输入，普通release（1.95.0/clang/SDK26.5、原profile、独立克隆缓存）4制品构建成功，1071.97秒，输入前后精确。monitor SHA `1b8c20d87f4132af23777c459ff3eba8dcfa1f627920c6663412fae2af000739`；activation_prepare `531914c1c5a3340493e69efe76eb9d2d2bed4fab0685b2e6e27600d3374463cc`；backfill_daily `701996a9e2465b94dcab5e3e3f002fb07f559a9318b7af201396b3125c27125c`；backfill_predictions `39f3ee585e3dda56e09047ee9146df0d7a10100ad96c7354dfba6958efc5aa92`。

同制品61 CLI dry-run与独立launchd shadow均通过，external_process_attempted/receipt_audit_appended=0，shadow卸载。原Schema9真实副本重开后93个catalog对象、49政策及所有表行精确。唯一同优化库真实Minute15接收取得300274/48，9/29、9/30、10/8各16根；原bars SHA `1df30a46d0b90be1f5c46953130dbccfe1e7ec55c1795e4a0ae991ba709f159d`保持，batch `tdx-smart:1791467179:3378`，artifact `7fd2b59707832ef31765bc21bb4be5f193e29749cfb7a69bcf31dd763ebd4304`。Windows仍841/abf28a/7407a，Mac client descriptor41db4范围不同；不授予完整30日/PIT/资金资格。

普通release预测CLI在一致私有主库副本运行。副本中只去掉两张原空P05表作为缺席条件，新retained入口未重建它们；原catalog和预测、日线、交易状态及账户/持仓表count/SHA保持。实际CLI exit1：原预测37–40起始在7/11或7/12周末，保持13项deferred/24个窗口deferred、verified0。初始化与保留证据成立，业务回填完成不成立；原失败退出值/日志保留，不改日期或把失败当成功。产品lib/monitor/gateway/合同代码与上次788d精确一致，复用原定向行为测试，未追加全量测试/check/clippy。

## 实际切换

发现每日任务调用 `runtime/bin/` 的旧工具，与上次正式 `target/release/` 制品不同。本次同时更新两个实际路径，SHA均与上述新制品一致；保留原日常脚本和21:17 plist，静态回退包含两套旧工具。21:50旧monitor64021、bridge63046、每日job81270及子81280全部退出后安装，不覆盖数据库、凭据或业务原件。

future activation `2026-10-08T13:54:29.391860Z`（21:54:29 CST），expected_config_hash `5df52cda549244764d8897ea92f61f87c3b05b215c293d3108348170b824b8a8`。桥接原二进制保持；bridge85637于21:55:32实际init/18082就绪，monitor86378于21:56:22启动。monitor86378于21:57:25实际初始化完成，同秒fixed point为progress0/resumed_sink_calls0/foreign0/manual79/hydrate17。21:58:50回读全部5091旧决策字段/envelope精确、79Uncertain精确、原Schema9/catalog49政策/主库catalog及两个原DB inode精确；账户summary41/snapshot39/明细254全部历史记录精确。实际launchctl进程中的证据目录env和0700根身份保持。21:58:58原日常job重新注册并恢复被切换中断的原任务，PID86715，脚本/plist保持、新工具两目录SHA精确；任务业务结果待自然完成，不以注册或PID冒称回填完成。

切换前共5091决策、79Uncertain。原第79条DataMode在19:41发生，晚于前次19:15验收而早于本次发布；没有平台message id，仍需人工裁定。全部既有记录保持，不自动重发/裁定。新来源/旧paper累计超卖及4条周末预测并不因重构建恢复；H08/outcome/monitor范围保持，冻结平台及正式资金seed/cutover不启用。

Health回读仍为unhealthy：monitor/heartbeat/snapshot均fresh，AccountFrozen/DataUnsafe，缺Quote/MoneyFlow/OrderBook；部署验证成立，完整业务资格与旧paper账本修复不成立。没有人工消息测试、旧投递重发、Windows服务更改或资金seed。

私有证据在 `~/.local/share/stock-analysis-candidates/latest-code-rebuild-20261008-2125/`；账户发布前为41 summary/39 snapshot/254明细，真实截图和数额不推Git。
