#!/usr/bin/env ruby
# 只读取证：Ruby 标准库 + sqlite3；不调用业务 binary/provider/sink，不修改数据库。
# 输出默认 stdout；--write 仅生成本目录的审计 JSON（生成产物，不是产品配置）。
require 'json'
require 'digest'
require 'open3'
require 'date'
require 'time'
require 'base64'

ROOT = File.expand_path('../..', __dir__)
Dir.chdir(ROOT)
FROM = '2026-08-31'
TO = '2026-09-05' # 包括周六负对照；闭区间，非“到周六零点”。
QUERIES = {}
def command(*args)
  out, err, status = Open3.capture3(*args)
  raise "#{args.first}: #{err}" unless status.success?
  out
end
def sql(key, db, query)
  QUERIES[key] = { 'db' => db, 'sql' => query }
  out = command('sqlite3', '-readonly', '-json', db, query)
  out.strip.empty? ? [] : JSON.parse(out)
end
def sha(path)
  Digest::SHA256.file(path).hexdigest
end
def local_time(value)
  # SQLite 裸时间只在调用方确认其为 UTC 后显式附 Z；日志裸时间另行补 +08。
  Time.iso8601(value).getlocal('+08:00')
end
def counts(rows, *keys)
  groups = Hash.new(0)
  rows.each { |r| groups[keys.map { |k| r[k] }] += 1 }
  groups.sort_by { |k, _| k.map(&:to_s) }.map do |values, n|
    Hash[keys.zip(values)].merge('n' => n)
  end
end
def phase(t)
  hm = t.strftime('%H:%M')
  return ['盘前', '盘前'] if hm < '09:15'
  return ['集合竞价', '集合竞价'] if hm < '09:25'
  return ['集合竞价', '竞价后间隙'] if hm < '09:30'
  return ['盘中', '早盘'] if hm < '11:30'
  return ['盘中', '午休'] if hm < '13:00'
  return ['盘中', '午盘'] if hm < '15:00'
  ['盘后', '盘后']
end
def jsonl(path)
  return [] unless File.file?(path)
  File.foreach(path).with_index(1).map do |line, n|
    begin
      yield JSON.parse(line), n
    rescue JSON::ParserError => e
      raise "invalid JSONL #{path}:#{n}: #{e.message}"
    end
  end.compact
end

started = Time.now.utc.iso8601
baseline = command('git', 'status', '--porcelain=v1')
conflicts = command('git', 'diff', '--name-only', '--diff-filter=U').lines.map(&:strip)
md_path = 'docs/Project_Architecture_Blueprint.md'
html_path = 'docs/Project_Architecture_Blueprint.html'
md = File.read(md_path)
html = File.read(html_path)
embedded = html[/<script[^>]*id="blueprint-data"[^>]*>(.*?)<\/script>/m, 1]
enum_source = File.read('src/bin/monitor/notify.rs')
enum_body = enum_source[/pub enum PushKind \{(.*?)^\}/m, 1] or raise 'PushKind not found'
kinds = enum_body.scan(/^    ([A-Za-z][A-Za-z0-9]*),/).flatten
raise 'duplicate PushKind' unless kinds.uniq == kinds
historical_rows = {}
current_phase = nil
md.each_line.with_index(1) do |line, n|
  current_phase = {'3'=>'盘前', '4'=>'集合竞价', '5'=>'盘中', '6'=>'盘后'}[$1] if line =~ /^### 24\.([3-6]) /
  if current_phase && line =~ /^\| `([A-Za-z0-9]+)` \/ `([A-Z-]+)` \| (.*?) \| (.*?) \|$/
    historical_rows[$1] = { 'phase_epic' => current_phase, 'historical_status' => $2,
      'historical_logic' => $3, 'historical_evidence' => $4, 'blueprint_line' => n }
  end
end
raise 'historical inventory changed' unless historical_rows.size == 65
presentation = File.read('src/bin/monitor/presentation_registry.rs').split('const PRODUCTION_PRESENTATION_DESCRIPTORS:', 2)[1].split('\n];', 2)[0]
presentation = presentation.split(/^\];/, 2)[0]
presentation_kinds = presentation.scan(/PushKind::([A-Za-z0-9]+)/).flatten
source_paths = %w[src/bin/monitor/main.rs src/bin/monitor/notify.rs src/bin/monitor/push_templates.rs src/bin/monitor/p01.rs src/bin/monitor/review_batch.rs src/bin/monitor/presentation_registry.rs src/bin/monitor/news_aggregator_init.rs src/bin/monitor/news_ai_shadow.rs src/bin/monitor/v14_adapter.rs src/bin/monitor/v17_sources.rs src/database/news_ai.rs src/database/watchdog_deadline.rs src/monitor/attribution_deep.rs src/monitor/alert_log.rs src/monitor/news_ai.rs src/trading/paper_sell.rs src/decision/intraday_monitor.rs src/durable_delivery/model.rs src/app/modes.rs src/notification/service.rs src/calendar.rs]
source_meta = source_paths.map do |p|
  { 'path' => p, 'sha256' => sha(p), 'lines' => File.foreach(p).count,
    'unmerged' => conflicts.include?(p),
    'git_status' => baseline.lines.map(&:chomp).find { |l| l[3..-1] == p } || 'clean' }
end
inventory = kinds.map do |kind|
  old = historical_rows[kind] || { 'phase_epic' => kind == 'Watchdog' ? '盘前' : '盘中', 'historical_status' => 'NOT_IN_65' }
  refs = source_paths.flat_map do |p|
    File.foreach(p).with_index(1).map { |line, n| "#{p}:#{n}" if line.include?("PushKind::#{kind}") && line =~ /PushKind::#{kind}\b/ }.compact
  end
  old.merge('kind' => kind, 'current_enum_line' => enum_source.lines.index { |l| l.strip == "#{kind}," } + 1,
    'presentation_registered' => presentation_kinds.include?(kind),
    'reference_locator_only' => refs) # 引用不是 caller 证明；也可能是测试/metadata。
end

legacy = sql('analytics_rows', 'data/push_analytics.db', "SELECT id,event_id,template_id,ts,pushed,sink_name,governance_decision,validation_status FROM push_analytics WHERE substr(ts,1,10) BETWEEN '#{FROM}' AND '#{TO}' ORDER BY id")
legacy.each { |r| r['day'] = r['ts'][0,10] }
durable = sql('sink_results', 'data/durable_delivery.sqlite3', "SELECT r.result_event_identity,r.decision_identity,r.attempt_identity,r.result_kind,r.observed_at,r.accepted_at,r.authoritative_for_state,r.channel,r.provider,d.business_date,d.push_kind,d.state FROM sink_results r JOIN delivery_decisions d USING(decision_identity) WHERE date(r.observed_at,'+8 hours') BETWEEN '#{FROM}' AND '#{TO}' ORDER BY r.observed_at")
accepted = durable.select { |r| r['result_kind'] == 'Accepted' && r['authoritative_for_state'] == 1 }
decisions = sql('decisions', 'data/durable_delivery.sqlite3', "SELECT business_date,push_kind,state,count(*) n FROM delivery_decisions WHERE business_date BETWEEN '#{FROM}' AND '#{TO}' GROUP BY 1,2,3 ORDER BY 1,2,3")
news = sql('news_delivered', 'data/stock_analysis.db', "SELECT e.event_id,e.assessment_id,e.created_at,e.delivery_audit_event_id,a.source_provider,a.source_batch_id,a.source_item_id,a.target_code,a.analysis_version,a.content_hash,a.normalized_prompt_sha256 FROM news_ai_delivery_event e JOIN news_ai_assessment a USING(assessment_id) WHERE e.state='delivered' AND date(e.created_at,'+8 hours') BETWEEN '#{FROM}' AND '#{TO}' ORDER BY e.created_at")
news.each { |r| r['day'] = local_time(r['created_at']).strftime('%F') }
trades = sql('paper_trades', 'data/stock_analysis.db', "SELECT id,plan_id,code,direction,quantity,price,fill_price,status,ts FROM paper_trades WHERE date(ts,'+8 hours') BETWEEN '#{FROM}' AND '#{TO}' ORDER BY ts,id")
trades.each { |r| r['day'] = local_time(r['ts'].tr(' ', 'T') + 'Z').strftime('%F') }
audit_path = 'data/event_audit/2026.jsonl'
audit_events = jsonl(audit_path) do |r, n|
  e = r['envelope'] || {}
  if e['ts'] && e['ts'][0,10] >= FROM && e['ts'][0,10] <= TO
    e.merge('_line' => n)
  end
end
flashes = audit_events.select { |e| e.fetch('payload', {})['news_flash_remote_receipt'] && e['payload']['news_flash_transaction_stage'] == 'Accepted' }
flash_rows = flashes.map do |e|
  p = e['payload']; receipt = p['news_flash_remote_receipt']
  { 'event_id' => e['id'], 'audit_line' => e['_line'], 'business_date' => p['news_flash_business_date'],
    'decision_key' => p['news_flash_decision_key'], 'accepted_at' => receipt['accepted_at'],
    'kind' => p['kind'], 'provider' => receipt['provider'],
    'receipt_sha256' => p['news_flash_remote_receipt_sha256'] }
end
daily = (Date.parse(FROM)..Date.parse(TO)).map do |day|
  ds = day.to_s
  l = legacy.select { |r| r['day'] == ds }
  a = accepted.select { |r| local_time(r['accepted_at']).strftime('%F') == ds }
  f = flash_rows.select { |r| local_time(r['accepted_at']).strftime('%F') == ds }
  { 'day' => ds, 'legacy_reported_true' => l.count { |r| r['pushed'] == 1 },
    'legacy_false_not_all_failure' => l.count { |r| r['pushed'] == 0 },
    'n02_analytics_subset' => l.count { |r| r['pushed'] == 1 && r['template_id'] == 'news_flash_aggregated' },
    'durable_accepted_at_send_date' => a.size, 'n02_typed_audit_at_send_date' => f.size,
    'durable_delivered_for_business_date' => decisions.select { |r| r['business_date'] == ds && r['state'] == 'Delivered' }.inject(0) { |s,r| s+r['n'] } }
end
# 仅诊断时间分布：legacy analytics.ts 可能是事实/采样时间，不保证物理发送时间。
phase_rows = legacy.select { |r| r['pushed'] == 1 && r['template_id'] != 'news_flash_aggregated' }.map do |r|
  t = local_time(r['ts']); epic, session = phase(t)
  { 'day'=>t.strftime('%F'), 'phase_epic'=>epic, 'exact_session'=>session, 'strength'=>'legacy_reported', 'kind'=>r['template_id'] }
end
(accepted + flash_rows.map { |r| r.merge('push_kind'=>'NewsFlashAggregated') }).each do |r|
  t = local_time(r['accepted_at']); epic, session = phase(t)
  phase_rows << {'day'=>t.strftime('%F'), 'phase_epic'=>epic, 'exact_session'=>session, 'strength'=>'typed_accepted', 'kind'=>r['push_kind']}
end
dispatch = []; review = []; logs = []; g5b = []
(Date.parse(FROM)..Date.parse(TO)).each do |day|
  ds = day.to_s
  dispatch.concat(jsonl("data/dispatcher_log/#{ds}.jsonl") { |r,n| r.merge('day'=>ds,'line'=>n) })
  review.concat(jsonl("data/review_audit/#{ds}.jsonl") { |r,n| r.fetch('payload').merge('day'=>ds,'line'=>n) })
  g5b.concat(jsonl("data/g5b/#{ds}.jsonl") { |r,n| {'day'=>ds,'line'=>n,'code'=>r.dig('record','code'),'category'=>r.dig('record','category'),'triggered_at'=>r.dig('record','triggered_at')} })
  Dir.glob("data/push_log/#{ds}/*.md").sort.each do |p|
    body = File.read(p)
    tags = ['TEST_CODE','Frozen','Unsafe','时间不可信','缺乏数据','系统哨兵'].select { |s| body.include?(s) }
    logs << {'path'=>p,'sha256'=>sha(p),'day'=>ds,'tags'=>tags,'first_line'=>body.lines.first.to_s.strip} unless tags.empty?
  end
end
# 匹配 paper 卡的 code+方向+数量+分价与账本，文件名时间是发送前保存，不是远端 Accepted。
paper_matches = []
(Date.parse(FROM)..Date.parse(TO)).each do |day|
  Dir.glob("data/push_log/#{day}/*.md").sort.each do |p|
    body = File.read(p)
    m = body.match(/\[虚拟盘(买入|卖出)\].*?\((\d{6})\) (?:买入|卖出)(\d+)股 @([\d.]+)/)
    next unless m
    direction = m[1] == '买入' ? 'buy' : 'sell'
    matches = trades.select { |r| r['day'] == day.to_s && r['direction'] == direction && r['status'] == 'Filled' && r['code'] == m[2] && r['quantity'].to_i == m[3].to_i && ((r['fill_price'] || r['price']).to_f * 100).round == (m[4].to_f * 100).round }
    row = {'path'=>p,'day'=>day.to_s,'direction'=>direction,'matches'=>matches.map { |r| r['id'] }}
    if matches.size == 1
      file_time = Time.strptime("#{day} #{File.basename(p)[0,6]} +0800", '%F %H%M%S %z')
      row['fill_to_presend_log_seconds'] = (file_time - local_time(matches.first['ts'].tr(' ', 'T')+'Z')).round
    end
    paper_matches << row
  end
end
design_sources = (Dir.glob('docs/v18.x/*.md') + Dir.glob('docs/v19.x/*.md')).sort.map do |p|
  tracked = Open3.capture3('git','ls-files','--error-unmatch','--',p)[2].success?
  ignored = Open3.capture3('git','check-ignore','--',p)[2].success?
  {'path'=>p,'sha256'=>sha(p),'tracked'=>tracked,'ignored'=>ignored}
end
output = {
  'schema'=>'push.reanalysis.evidence.v1', 'status'=>'PROVISIONAL', 'collected_started_at'=>started,
  'window'=>{'from'=>FROM,'through'=>TO,'timezone'=>'Asia/Shanghai'},
  'limitations'=>['Three live databases read sequentially, not an atomic snapshot.', 'Local Accepted is not recipient read confirmation.', 'Hash capture is not a full authority-chain verification.', 'Reference locator includes metadata/tests; it is not a producer or deployment proof.'],
  'head'=>command('git','rev-parse','HEAD').strip, 'unmerged_count'=>conflicts.size,
  'code_sources'=>source_meta,'design_sources'=>design_sources,
  'blueprint'=>{'md_sha256'=>sha(md_path),'html_sha256'=>sha(html_path),'embedded_md_equal'=>embedded && Base64.decode64(embedded).b == md.b,'external_scripts'=>html.scan(/<script[^>]*src="([^"]+)"/).flatten},
  'inventory'=>inventory,'presentation'=>{'tuples'=>presentation_kinds.size,'unique_kinds'=>presentation_kinds.uniq.size,'missing_kinds'=>kinds-presentation_kinds},
  'daily'=>daily,'analytics_by_kind'=>counts(legacy,'day','template_id','pushed','sink_name'),
  'phase_distribution'=>counts(phase_rows,'day','phase_epic','exact_session','strength'),
  'durable_results'=>durable,'durable_decisions'=>decisions,'news_flash_typed_receipts'=>flash_rows,
  'news_ai_days'=>news.group_by { |r| r['day'] }.map { |day,rs| {'day'=>day,'delivered'=>rs.size,'items'=>rs.map { |r| [r['source_provider'],r['source_item_id']] }.uniq.size,'source_target_pairs'=>rs.map { |r| [r['source_provider'],r['source_item_id'],r['target_code']] }.uniq.size,'batches'=>rs.map { |r| r['source_batch_id'] }.uniq.size} },
  'news_ai_repeat_candidates'=>news.group_by { |r| [r['day'],r['source_provider'],r['source_item_id'],r['target_code']] }.select { |_,rs| rs.size>1 }.map { |k,rs| {'key'=>k,'n'=>rs.size,'assessment_ids'=>rs.map { |r| r['assessment_id'] },'batch_count'=>rs.map { |r| r['source_batch_id'] }.uniq.size,'prompt_hash_count'=>rs.map { |r| r['normalized_prompt_sha256'] }.uniq.size} },
  'paper_trade_counts'=>counts(trades,'day','direction','status'),'paper_message_matches'=>paper_matches,
  'dispatcher_counts'=>counts(dispatch,'day','kind','success'),
  'dispatcher_reason_counts'=>counts(dispatch.map { |r| r.merge('reason_token'=>r['error'].to_s[/reason_code=([^\s;]+)/,1] || r['error'].to_s[/reason=([^\s;]+)/,1] || r['error'].to_s[/diagnostic_code=([^\s;]+)/,1] || r['error'].to_s) },'day','kind','success','reason_token'),
  'review_transitions'=>counts(review,'day','task','status','reason_code','retryable'),
  'g5b_events'=>g5b,'message_samples'=>logs,
  'account_snapshot_refs'=>sql('account_snapshot_refs','data/stock_analysis.db',"SELECT id,effective_at,item_count FROM user_position_snapshot ORDER BY id DESC LIMIT 2"),
  'account_summary_refs'=>sql('account_summary_refs','data/stock_analysis.db',"SELECT id,effective_at,round(total_assets-securities_market_value-available_cash,2) AS unclassified_asset_gap FROM user_account_summary ORDER BY id DESC LIMIT 2"),
  'real_account_refs'=>sql('real_account_refs','data/stock_analysis.db',"SELECT id,snapshot_date,account_ref_status FROM real_account_snapshot ORDER BY id DESC LIMIT 2"),
  'watchdog'=>sql('watchdog','data/stock_analysis.db',"SELECT * FROM watchdog_deadline WHERE business_date BETWEEN '#{FROM}' AND '#{TO}' ORDER BY business_date,family"),
  'hydration'=>sql('hydration','data/durable_delivery.sqlite3',"SELECT d.business_date,d.push_kind,t.hydration_state,t.hydrated_at,p.created_at AS disposition_created_at FROM task_transition_payloads t JOIN delivery_decisions d USING(decision_identity) JOIN delivery_disposition_payloads p ON p.disposition_identity=t.disposition_identity WHERE d.business_date BETWEEN '#{FROM}' AND '#{TO}' ORDER BY d.business_date,d.push_kind"),
  'queries'=>QUERIES
}
output['collected_finished_at'] = Time.now.utc.iso8601
encoded = JSON.pretty_generate(output) + "\n"
if ARGV == ['--write']
  path = File.join(__dir__, 'recent-push-evidence-2026-09-05.json')
  File.write(path, encoded) # 机械生成审计产物。
  notes = {
    'NewsToIdea'=>'存在第二条 NewsAI producer：main::news_monitor_loop → news_ai_shadow::schedule_from_same_tick → assessment/delivery event owner。五日 212 条 NewsAI delivered；不能只用 D-01 的 Top1/20min 规则解释。见总报告 F03。',
    'PaperSell'=>'共 408 张卖出卡与 Filled 成交按日期/证券/方向/数量/分价唯一匹配；批处理后才逐条发卡，9/1 最长 1130 秒到发送前日志。不是据数量认定重复。见 F04。',
    'PaperBuy'=>'新项，main.rs:8970 起只遍历 report.fills；report 的 Filled 收集实现位于 decision/intraday_monitor.rs；工作树存在不代表该制品已验证部署。9/4 有 29 张卡及 29 笔唯一匹配 Filled。无持久通知恢复；不补发旧日期买入卡。见 F04。',
    'Watchdog'=>'新项，主归属盘前/运行健康，跨四阶段；review scheduler 每60秒查 deadline。三个 family 为 news_first_wave/attribution_1505/review_evening，发送无条件 mark_fired，依赖原任务注册。见 F06。',
    'IndustryChain'=>'R-03 是固定 LegacyAccountGate，当前无条件返回 account_metrics_incomplete。不能归因于用户今天未补数据，也不能将独立09:05/15:30报告视为该 kind。见 F07。',
    'EventCalendar'=>'最近五日259条 dispatcher失败，其中256条invalid_evidence、3条no_verified_batch；R-08 review102条失败均retryable。CFFEX强制依赖失败被字符串化再升级为可重试，非单纯网络抖动。见 F08。',
    'NewsFlashAggregated'=>'五日8个带typed receipt的Accepted窗口。analytics.ts可能是源事实时间，按receipt.accepted_at重建实际发送时段。不能把08-31 04:23解析成凌晨物理推送。见 F01/F10。',
    'G5bAttribution'=>'08-31有两份TEST_CODE深链结果和同分钟feishu pushed=true记录；namespace污染已成立，远端实际阅读未证明。top_events_for_deep只排序截断，不做事件聚合。见 F02/F05。',
    'AttributionDaily'=>'15:05链已改日K取收盘价，但仍在任何push outcome后推进日完成并satisfy哨兵；报表文件不能替代回执。五日analytics无该类true记录。见 F05。',
    'CandidateBoard'=>'当前dispatcher在确认空批时先return，未进入失效diff；非空时又先保存diff基线再发卡。应分别验证“全部消失”和“来源失败”，不能二者混用。见F05。',
    'CandidateInvalidated'=>'失效推送结果被丢弃；本轮候选全空时上层提前return导致不会逐票失效。和CandidateBoard共享completion owner，不应独立切换。见F05。',
    'IntradayMarket'=>'同kind至少三个producer语义：盘中资金概览、09:10预检、15:05快照提醒；必须分别定义occurrence/completion owner。见 F07。'
  }
  appendix = ["# 全量推送审计视图（67 项，2026-09-05）", '',
    '> PROVISIONAL：由取证脚本从当前 enum 与蓝图 §24.3–§24.6 机械生成，不是新的生产 capability catalog。', '',
    '阅读顺序：[总报告](comprehensive-reanalysis-2026-09-05.md) → 本清单 → [证据快照](recent-push-evidence-2026-09-05.json)。', '',
    '“原业务逻辑”是冻结蓝图的被审计断言，不代表本轮将冲突源码逐行验证通过。“校正”优先于原断言；其余仍须在干净源码基线复核可达性。当前 enum 定义行与文件 SHA 可由 JSON 查询；历史代码行号只供追溯，不能当作当前锚点。', '',
    '原65项状态是 ACTIVE37 / INACTIVE24 / STARVED2 / OPT-IN2；新增PaperBuy/Watchdog均有本地运行记录，但这不证明当前混合工作树就是部署制品。', '']
  ['盘前','集合竞价','盘中','盘后'].each do |ph|
    rs = inventory.select { |r| r['phase_epic'] == ph }
    appendix += ["## #{ph}（#{rs.size}项；Watchdog跨时段）", '', '| PushKind / 原状态 | 原业务逻辑及来源 | 本轮校正 |', '| --- | --- | --- |']
    rs.each do |r|
      old = r['historical_logic'] || '旧65项蓝图未收录；以本轮新增实现和日志为准。'
      link = r['blueprint_line'] ? "蓝图源第#{r['blueprint_line']}行" : "notify.rs第#{r['current_enum_line']}行"
      note = notes[r['kind']] || '保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。'
      appendix << "| `#{r['kind']}` / #{r['historical_status']} | #{old}（#{link}） | #{note} |"
    end
    appendix << ''
  end
  appendix += ['## enum 外路径与多 producer', '',
    '09:05/15:30产业链报告、CLI单股/汇总报告均走NotificationService；AlertManager无生产caller证据，不能按第五条已运行消息算量。NewsAI虽复用NewsToIdea，仍有独立分析和完成owner；PaperSell盘中/盘后、IntradayMarket三处以及MarketActionAlert两处也需逐producer核对。参见总报告§3及蓝图§24.7。', '']
  File.write(File.join(__dir__, 'all-push-kinds-2026-09-05.md'), appendix.join("\n"))
  puts "#{path}: #{encoded.bytesize} bytes; kinds=#{kinds.size}; unmerged=#{conflicts.size}"
elsif ARGV.empty?
  puts encoded
else
  abort 'Usage: ruby docs/push-system/reanalyse-recent-pushes.rb [--write]'
end
