# Windows Codex 问题同步解决记录

更新：2026-10-06 23:22 CST。用户直接授权「把 windows codex 的问题同步解决掉」。Mac 工作树为 platform-roadmap-implementation；本记录代码基线 `4ee64e79871893d81d504717cd6989eaf57b507f`，Windows 已提交基线 `f57c114190436e6ec96faa60910c127925ffbcfc`。两个项目的源码身份分别保留。

## 1. 本次完成与仍阻断的项

| 项目 | 实际证据 | 状态与下一步 |
| --- | --- | --- |
| f57c audit 网络失败 | CI 37446154439 的 audit 112328508220 单项重试成功，Mac 前次已核验同 HEAD、原日志与继承 coverage 时间/steps | 这项检查已恢复；原失败保留，不重复整个 workflow |
| 新财报入口测试 | 新11项、模块49、受影响库122、workspace2072通过，0失败、3忽略；原包装器在第9步文档检查崩溃 | fixture错误已改；尚未提交，不宣称整套检查或发布完成 |
| 文档 checker 崩溃 | 原命令0xC0000005，先后停在78与68次存在性检查；冻结输入/LF副本各完成333次 | 根因未定，继续原生异常/父子进程/解释器定位；副本成功不替代原门 |
| 关键代码覆盖率 | f57c 原实测35372/39817＝88.84%，95%未通过；overall84.93%通过80% | 原checker/globs/分母保留；新提交必须取得同HEAD CI，不能以新增测试数代替 |
| Windows→Mac 新交接包 | 科技/财报包31文件、checker诊断20文件、行结束符对照8文件，逐成员长度/SHA/目录集合核验 | 包外ACK已交；只是字节回读，无Mac原生Windows重跑、独立SDK评审或发布资格 |
| Mac→Windows 验收输入 | 两组官方新闻基准和明确窗口、现网最小3只A股、三类报表/披露类别、两个SEC身份和10-K | 已发布并发送；Windows已核验4成员与勘误ACK，真实业务调用仍未执行 |
| Mac 客户端兼容缺口 | 核对当前compat请求/响应入口、公开v2合同和实际操作分支 | 新财报/公告/SEC消费者仍待开发与离线验证；最新新闻筛选不等于历史检索 |

本次直接消息工具返回成功，对方在现有聊天中明确确认收到 Mac 反馈并发布诊断，23:16 CST已在共享目录交回Mac输入包及勘误的字节ACK。23:22 CST确认其上一轮只完成交接，已再次发送继续实际修复和同HEAD CI的明确任务；新任务实际进展需后继回执。跨端读取曾短暂失败，现已恢复；Mac经prlctl只读确认Windows11 running并读AGENTS，未改SDK源、环境或服务。没有另建聊天，也没有仅凭“消息已发送”认定对方已完成修复。

## 2. 共享材料的精确身份

共享根：`/Users/zhangzhen/Desktop/Quant/stock_analysis/client-bundle`；Windows 经原 `\\Mac\Home\Desktop\Quant\stock_analysis\client-bundle` 读取。包本体保持封存，ACK/勘误均写包外。

| 包 | manifest SHA-256 | 本次范围 |
| --- | --- | --- |
| `windows-news-finance-handoff-20261006.1` | `40ced18aa7f58ffb746aa1bf44bf7b6fd0219228a94efef91b610d0cfd759787` | 30成员667689字节，31文件含manifest；WIP和原失败，非release |
| `windows-docs-checker-diagnostic-20261006.1` | `a6199f79a333ec8489fe512aa47209e4efb3f8e600e3cb2989a742c4a4b99a1d` | 19成员620761字节，20文件含manifest；命令/包装器/版本与原日志 |
| `windows-docs-checker-line-endings-20261006.1` | `a9db79a5ec8294dac1b399879b284c657ec701b3068780bec65d3409dc6256ce` | 7成员620510字节，8文件含manifest；单次区分观察 |
| `mac-windows-sync-inputs-20261006.1` | `a8991a272e25cb9ba0bd39f91a6807c8f25019879d1a25b36b9569e330edda4a` | 4成员/5文件；验收输入、客户端缺口、诊断顺序，非live证据 |

Mac输入包外 `CLIENT_GAPS_ERRATA.md` 的SHA为 `8f491579a1f72ae0cf8c483244cab0fbd5adfb867636ef2e8f3a9914733f5db8`：原表述“CompanyFilings可dispatch”过强。当前本地客户端没有该match分支，兜底unimplemented；operation/protobuf定义不能证明ExternalV1方法已准入或存在业务caller。SEC样例是期望验收，消费者/方法准入仍需补齐。勘误已发 Windows，原封存包未覆盖。

## 3. checker 排查的证据边界

- 原正式脚本规范Git LF SHA `6e52d40be74970345c60ee94c964e8e10f4cc9271ee33e37e7dcc7b360858f85`；Windows实际CRLF SHA `5cbc626ce1fbfc2676c8533cd914358984b2ec49538d2462c800b846d1f496d2`。逐字循环一致，但副本同时改变过输入边界/位置/行结束符，不能锁定其中一个为根因。
- 独立 `rg` 生产者返回0、619条；原script使用process substitution并吞掉生产者错误。这是需要区分的边界，不是已证实的本机崩溃原因。冻结输入只证明该份输入可以遍历。
- 原启动路径 `Git\bin\bash.exe` 为47448字节launcher；`Git\usr\bin\bash.exe` 为2456832字节真实解释器。官方说明包装器设置环境并启动真实程序，因此启动PID的返回码还不能直接归因read builtin。下一步记录父子PID/退出码，查现有stackdump/WER或已安装调试工具；保持原源码/原认证，私有内存转储不共享。[Git for Windows 包装器](https://gitforwindows.org/git-wrapper.html)、[官方调试说明](https://gitforwindows.org/debugging-git.html)。
- 当前诊断BASH_ENV/ENV未设置，历史包装器未记录，不能反推历史环境；没有升级Git、关闭安全软件、修改原checker、忽略退出码或重跑整套gate。

## 4. 已补的业务验收范围

**科技主题：** Rubin窗口2026-01-01至01-10，官方基准发布日期01-05；Muse窗口2026-09-20至09-26，官方基准09-23。中英文检索词保留，不将Muse所有主题混成单一产品，也不以近30天空结果否定历史文章。[NVIDIA基准](https://nvidianews.nvidia.com/news/rubin-platform-ai-supercomputer)、[Meta基准](https://research.meta.ai/blog/bringing-your-muse-to-life)。读取公共基准不等于新Provider的采集、保存或展示准入。

**A股：** 从现有生产STOCK_LIST取最小样例605178/002916/688548，分别保留Shanghai/Shenzhen/Shanghai与Equity身份。财报Income/Balance/CashFlow；目标FY2025/H12026只作源期间验收筛选，不在最新20季度接口中捏造历史年份参数。源期间缺失保留null。公告采用逐证券instrument/start/end/limit，范围2026-01-01至09-30，分类年报、半年报、预告和减持计划/进展/完成/回购股份处置。002916半年报/预告的公开CNinfo搜索基准已列，PDF正文未独立读；年报/减持正基准未核实，明确保持缺失。

**SEC元数据：** NVDA CIK0001045810、原10-K accession0001045810-26-000021；META CIK0001326801、原10-K accession0001628280-26-003942。已独立读官方index的form、报告期间与filing date；accepted日期与filing date分开，10-Q不重命名半年报；正文、附件和XBRL不在现有准入范围。[NVDA原index](https://www.sec.gov/Archives/edgar/data/1045810/000104581026000021/0001045810-26-000021-index.htm)、[META原index](https://www.sec.gov/Archives/edgar/data/1326801/000162828026003942/0001628280-26-003942-index.htm)。

**连接材料：** 本轮只核对私有配置中client bundle键/路径存在；未设置独立GRPC_MARKET_ADDR不表示bundle不能连接。未复制任何Token、Provider key、私钥、证书或完整配置；未执行网络/Health/RPC。

## 5. 实施顺序与完成条件

1. Windows完成原生checker定位/修复，只补受影响检查；复用已通过且源码未变的Cargo证据。按本仓库要求评审财报WIP后提交/推送，取得新HEAD原80/95门CI；不重复原f57c审计重试。
2. Mac先在原owner补财报/公告v2纯DTO和证据校验、SEC专用方法/请求身份与错误分类；科技历史检索须有上游检索合同及新源Gate A后接入，不扩全Provider探测。
3. CI实际合格后Windows生成同source/contract/binary绑定release；Mac重绑匹配制品并做受影响兼容验证。旧R2只绑定eea/29a/abf，不能与f57c拼接。
4. 具体候选具备后复核原认证、监听/写入owner、单实例控制、停止方式与回退，并按精确activation人审发布。取得同版真实逐来源新闻/财报/披露及native/R08证据与生产观察，逐项关闭H07–H08。

本轮Mac仅材料、源码阅读与字节核验，没有Rust修改、Cargo、release、安装、重启、真实业务请求或资金/Uncertain裁定。整体H01–H17和M0–M7仍按[整体交接](2026-10-06-platform-remaining-work-handoff.md)验收，不把同步材料完成记成全部上线完成。私有详细执行证据在 `.planning/2026-10-06-windows-sync-resolution/`，不默认随Git交付。

## 6. 后继披露正反基准与原生排查（23:32 CST）

Windows已在 `WINDOWS_MAC_SYNC_RESPONSE_20261006.md` 确认前包可用于需求验收，列出真正缺项：年报/减持正基准、新新闻来源准入、合格同HEAD release与真实业务回执。Mac继续补 `mac-disclosure-reference-inputs-20261006.1`：2成员8322字节、3文件含manifest，manifest SHA `d8b42930f0203ae615df8405c44ef5ef0af2d006ab30a06189b829886c919278`，已逐成员回读并发送Windows。

- 年报/半年报/预告：002916的[2025年报](https://static.cninfo.com.cn/finalpage/2026-03-13/1225006760.PDF)、[2026半年报](https://static.cninfo.com.cn/finalpage/2026-08-27/1225512708.PDF)、[半年预告](https://static.cninfo.com.cn/finalpage/2026-07-14/1225421321.PDF)，已独立读原PDF的相关标题、证券代码和原报告期间。预告签署7月13日与URL日期7月14日保留为不同字段；没有提取财务数字，也不跨Provider填Hithink的期间null。
- 减持计划正基准：605178[2026-005计划](https://static.cninfo.com.cn/finalpage/2026-01-30/1224956431.PDF)，已读原标题/证券身份和计划段落；只能支持计划类型，不能据此认定执行完成。
- 两个实际分类反例：688548[承诺不减持](https://static.cninfo.com.cn/finalpage/2026-07-23/1225437974.PDF)不能按“减持”关键词误分成计划/进展/完成；605178[异动公告](https://static.cninfo.com.cn/finalpage/2026-02-26/1224984774.PDF)引用旧2026-005，不产生新减持事件。均已读相关原段落，原三issuer/2026窗口不变。减持进展/完成及回购股份处置正基准仍未核实。

上述公开原文是有界验收参考，不是RPC命中或正文Provider准入。只保存有限标题/身份/日期/分类条件与URL，未复制PDF全文。URL路径日期只有日精度，未伪造实际发布时间/时区。

Windows新修复turn `01a111cd-de1f-7180-94f8-7e6d315288ee` 实际active，正在使用已有Win32调试API区分进程。第一次调试观测记录launcher、3个真实Bash及rg都退出0，没有抓到访问违例；调试时序可能影响复现，故继续少量无调试器采样取得失败时各进程退出码。仍未提供根因、正式checker修复、新提交95%CI或运行候选。Mac文档前提交 `448cb929d20c6bf134af45e711e64271f1cce94e` 已推送且实际remote匹配，不能由文档提交推定SDK修复完成。
