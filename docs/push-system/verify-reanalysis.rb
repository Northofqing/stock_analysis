#!/usr/bin/env ruby
# 验证已捕获证据与本文数字，不重新扫描运行数据库或发送消息。
require 'json'
require 'digest'
require 'time'

ROOT = File.expand_path('../..', __dir__)
Dir.chdir(ROOT)
x = JSON.parse(File.read(File.join(__dir__, 'recent-push-evidence-2026-09-05.json')))
checks = 0
check = lambda do |condition, label|
  abort "FAIL: #{label}" unless condition
  checks += 1
end
enum = File.read('src/bin/monitor/notify.rs')[/pub enum PushKind \{(.*?)^\}/m,1].scan(/^    (\w+),/).flatten
check.call(x['inventory'].map { |r| r['kind'] }.sort == enum.sort, 'enum exact coverage')
check.call(enum.size == 67 && enum.uniq.size == 67, '67 unique kinds')
check.call(x['presentation']['tuples']==58 && x['presentation']['unique_kinds']==54, 'presentation 58/54')
check.call(x['blueprint']['embedded_md_equal'], 'HTML embedded Markdown bytes')
x['code_sources'].each do |s|
  check.call(Digest::SHA256.file(s['path']).hexdigest == s['sha256'], "source SHA: #{s['path']}")
end
x['design_sources'].each do |s|
  check.call(Digest::SHA256.file(s['path']).hexdigest == s['sha256'], "design SHA: #{s['path']}")
end
check.call(Digest::SHA256.file('docs/Project_Architecture_Blueprint.md').hexdigest == x['blueprint']['md_sha256'], 'blueprint SHA')
days = x['daily'].select { |r| r['day'] < '2026-09-05' }
check.call(days.map { |r|r['legacy_reported_true'] } == [44,410,160,74,111], 'daily analytics counts')
check.call(days.map { |r|r['durable_accepted_at_send_date'] } == [24,31,24,21,19], 'daily receipt counts')
check.call(days.map { |r|r['durable_delivered_for_business_date'] } == [21,31,24,21,19], 'business-date counts')
check.call(days.inject(0) { |s,r| s+r['legacy_false_not_all_failure'] } == 310, 'analytics false')
check.call(x['news_flash_typed_receipts'].size==8, 'N02 receipts')
check.call(x['news_flash_typed_receipts'].map { |r|r['event_id'] }.uniq.size==8, 'N02 unique event')
check.call(x['durable_results'].size==119, 'durable results')
check.call(x['durable_results'].all? { |r|r['result_kind']=='Accepted' && r['authoritative_for_state']==1 }, 'result strength')
check.call(x['durable_results'].map { |r|r['attempt_identity'] }.uniq.size==119, 'unique attempts')
rej = x['durable_decisions'].select { |r| r['state']=='RejectedDurable' && r['business_date']<'2026-09-05' }.inject(0) { |s,r|s+r['n'] }
check.call(rej==443, 'rejected decisions, not sink failures')
pm = x['paper_message_matches']
check.call(pm.size==437 && pm.all? { |r|r['matches'].size==1 }, 'paper exact match counts')
check.call(pm.flat_map { |r|r['matches'] }.uniq.size==437, 'no same fill matched twice')
check.call(pm.select { |r|r['day']=='2026-09-01' }.map { |r|r['fill_to_presend_log_seconds'] }.max==1130, 'paper max latency')
check.call(x['news_ai_days'].map { |r|r['delivered'] }==[120,6,48,38], 'NewsAI counts')
check.call(x['hydration'].size==30 && x['hydration'].all? { |r|r['hydration_state']=='Applied' }, 'legacy hydration, not proposed finalizer')
check.call(x['account_snapshot_refs'].first['id']==25 && x['account_summary_refs'].first['id']==27, 'account snapshot references')
view = File.read(File.join(__dir__, 'all-push-kinds-2026-09-05.md'))
rows = view.scan(/^\| `([A-Za-z0-9]+)` \//).flatten
check.call(rows.sort==enum.sort, 'generated Markdown exact coverage')
docs = %w[README.md comprehensive-reanalysis-2026-09-05.md all-push-kinds-2026-09-05.md]
docs.each do |name|
  path = File.join(__dir__, name)
  body = File.read(path)
  check.call(!body.gsub(/  $/, '').match?(/[ \t]+$/), "no unexpected trailing whitespace (Markdown hard breaks allowed): #{name}")
  body.scan(/\]\(([^)]+)\)/).flatten.each do |target|
    next if target.start_with?('#','http:','https:')
    file, fragment = target.split('#',2)
    resolved = File.expand_path(file, __dir__)
    check.call(File.file?(resolved), "link exists: #{target}")
    if fragment && fragment =~ /^L(\d+)$/
      check.call($1.to_i <= File.foreach(resolved).count, "line in range: #{target}")
    end
  end
end
puts "PASS: #{checks} checks; enum67, receipt119+8, paper437 unique; source hashes and document links."
puts 'NOT CHECKED: compiled Rust, deployment identity, remote inbox, full historical hash-chain integrity, formal RFC readiness.'
