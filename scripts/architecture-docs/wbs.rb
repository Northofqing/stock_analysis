# frozen_string_literal: true

require 'json'
require 'digest'
require 'bigdecimal'
require_relative 'rfc_inputs'

module ArchitectureDocs
  module Wbs
    PATH = 'docs/push-system/push-system-wbs.v1.json'
    CATALOG = 'docs/push-system/push-capability-catalog.v1.json'
    RFC = 'docs/push-system/push-system-implementation-rfc.md'
    BEGIN_MARKER = '<!-- RFC-WBS-BEGIN -->'
    END_MARKER = '<!-- RFC-WBS-END -->'
    TOP_FIELDS = %w[schema_version status catalog_sha256 assumptions contingency engineering_totals trading_totals calendar_scenarios critical_path foundation_work_packages migration_units].freeze
    ROW_FIELDS = %w[id name scope lineage evidence_or_catalog_refs dependencies risk_class optimistic_hours most_likely_hours pessimistic_hours pert_hours engineer_count external_wait_business_days calendar_constraints acceptance_gates rationale].freeze
    UNIT_FIELDS = %w[completion_owner phase_epics producer_ids catalog_unit_sha256 changes_physical_owner approved_promotion_rank promotion_sessions observation_sessions].freeze
    CORE_DEPENDENCIES = %w[W01 W02 W03 W06 W07 W08 W09 W10 W11 W12 W16 W17 W18 W19 W20].freeze
    SHARED_GATES = %w[unit failure crash shadow dedup rollback].freeze
    RECOVERY = %w[recover_existing_facts no_new_occurrence no_duplicate_send uncertain_no_blind_resend].freeze
    INACTIVE = %w[remain_inactive missing_dependencies_visible zero_messages_not_pass].freeze
    REPLAY = %w[real_send_audit original_and_replay_identity dry_run_zero_send force_explicit_authorization].freeze
    WAVES = [
      %w[MU-cli-single MU-cli-summary], %w[MU-chain-preopen], %w[MU-chain-post-close],
      %w[MU-attribution-daily], %w[MU-g5b-attribution], %w[MU-intraday-market],
      %w[MU-auction-candidates], %w[MU-limit-boards],
      %w[MU-review-r04 MU-review-r07 MU-review-r08 MU-review-r09 MU-review-r11 MU-review-r13 MU-review-a10],
      %w[MU-paper-review-noon MU-paper-review-daily]
    ].map { |g| g.sort.freeze }.freeze
    FOUNDATION_NAMES = [
      '身份、业务日、occurrence 与 source-contract 基础合同',
      '应用 DeliveryResult 与现有 durable 类型适配',
      'CompletionPolicy、ReasonCode 与 RetryPolicy',
      'RunContext 与 PreparedFacts 单次取数',
      'SemanticProjection、PreparedPush 与 exact bytes 绑定',
      'catalog/Unit/completion-owner 运行时注册表',
      'business intent schema 与迁移',
      'append-only transition/outbox',
      'VerifiedTerminalRef 构造与重验证',
      '通用 finalizer 与业务 CAS',
      'reconciler、lease/fence 与启动恢复',
      'transport authority port 与通用 adapter',
      'P01/N02 专用 conformance adapter',
      'PhaseScheduler 与 occurrence catch-up',
      'readiness/operational snapshot 与 deploy probe',
      'activation manifest、generation CAS 与 owner fence',
      'shadow harness 与 typed diff',
      'operator inspect/reconcile/resolve/promote/rollback',
      '指标、SLA、保留期与安全审计',
      'fault/replay/dedup/rollback 回归 harness',
      '发布编排、N/N-1 兼容和逐 Unit/tail-cleanup 门禁工具'
    ].freeze
    class Invalid < StandardError; end
    class UniqueObject < Hash
      def []=(key, value)
        raise Invalid, "wbs_duplicate_json_key key=#{key}" if key?(key)
        super
      end
    end
    module_function

    def parse(bytes)
      JSON.parse(bytes, object_class: UniqueObject, decimal_class: BigDecimal)
    end

    def read(root, path)
      bytes, problem = RfcInputs.read_bytes(File.realpath(root), path)
      raise Invalid, "wbs_document_#{problem} path=#{path}" if problem
      bytes.force_encoding(Encoding::UTF_8)
      raise Invalid, "wbs_encoding_invalid path=#{path}" unless bytes.valid_encoding?
      bytes
    end

    def validate(root, freshness: true)
      wbs = parse(read(root, PATH))
      catalog_bytes = read(root, CATALOG)
      errors = data_errors(wbs, parse(catalog_bytes), catalog_bytes)
      return errors.uniq.sort unless errors.empty?
      text = read(root, RFC)
      expected = replace(text, render(wbs))
      errors << 'wbs_rfc_stale' if freshness && text != expected
      errors.uniq.sort
    rescue Invalid => error
      [error.message]
    rescue JSON::ParserError
      ['wbs_json_invalid']
    rescue SystemCallError, ArgumentError
      ['wbs_path_invalid']
    end

    def data_errors(w, catalog, bytes)
      return ['wbs_structure_invalid'] unless w.is_a?(Hash)
      errors = []
      errors << 'wbs_fields_invalid' unless w.keys.sort == TOP_FIELDS.sort
      errors << 'wbs_schema_invalid' unless w['schema_version'] == 1
      errors << 'wbs_status_invalid' unless w['status'] == 'PROVISIONAL'
      errors << 'wbs_catalog_sha_mismatch' unless w['catalog_sha256'] == Digest::SHA256.hexdigest(bytes)
      unless catalog.is_a?(Hash) && %w[migration_units producers kinds].all? { |k| objects?(catalog[k]) }
        return (errors + ['wbs_catalog_invalid']).sort
      end
      unless catalog['migration_units'].all? { |r| text?(r['id']) && text?(r['completion_owner']) && strings?(r['phase_epics']) && strings?(r['producer_ids']) } &&
             catalog['producers'].all? { |r| text?(r['id']) } && catalog['kinds'].all? { |r| strings?(r['producer_ids']) }
        return (errors + ['wbs_catalog_invalid']).sort
      end
      fs, us = w.values_at('foundation_work_packages', 'migration_units')
      return (errors + ['wbs_rows_invalid']).sort unless objects?(fs) && objects?(us)
      [['foundation_work_packages', fs], ['migration_units', us]].each do |path, rows|
        rows.each_with_index do |row, index|
          errors << "wbs_id_invalid path=#{path}[#{index}].id" unless text?(row['id'])
        end
      end
      return errors.uniq.sort if errors.any? { |error| error.start_with?('wbs_id_invalid ') }
      fids = (1..21).map { |i| format('W%02d', i) }
      cids = catalog['migration_units'].map { |r| r['id'] }
      errors << 'wbs_foundation_set_invalid' unless fs.map { |r| r['id'] }.sort_by(&:to_s) == fids
      errors << 'wbs_unit_set_invalid' unless us.length == 52 && cids.length == 52 && cids.uniq.length == 52 && us.map { |r| r['id'] }.sort_by(&:to_s) == cids.sort
      assumptions = w['assumptions']
      expected_assumptions = {
        'engineering_day_hours'=>8, 'engineer_count'=>1, 'lineage'=>'reconstructed_2026-09-06',
        'historical_foundation_range_hours'=>[98,142], 'historical_usage'=>'comparison_only_not_fit_or_recovered',
        'review_and_repair'=>'included_in_three_point_estimates', 'phase_epics'=>%w[盘前 集合竞价 盘中 盘后],
        'nonbaseline'=>%w[PaperBuy Watchdog]
      }
      unless assumptions.is_a?(Hash) && expected_assumptions.all? { |k,v| assumptions[k] == v } &&
             %w[unit_hash_encoding recent_evidence].all? { |k| text?(assumptions[k]) } &&
             assumptions.keys.sort == (expected_assumptions.keys + %w[unit_hash_encoding recent_evidence]).sort
        errors << 'wbs_assumptions_invalid'
      end
      contingency = w['contingency']
      unless contingency.is_a?(Hash) && contingency.keys.sort == %w[percent applications basis excludes rationale].sort &&
             positive?(contingency['percent']) && contingency['percent'] <= 100 && contingency['applications'] == 1 &&
             contingency['basis'] == 'sum_stored_line_pert_hours' &&
             contingency['excludes'] == %w[external_wait promotion_sessions observation_sessions] && text?(contingency['rationale'])
        errors << 'wbs_contingency_invalid'
      end
      (fs + us).each do |r|
        id = r['id']
        unit = us.include?(r)
        fields = ROW_FIELDS + (unit ? UNIT_FIELDS : [])
        errors << "wbs_row_fields_invalid id=#{id}" unless r.keys.sort == fields.sort
        errors << "wbs_lineage_invalid id=#{id}" unless r['lineage'] == 'reconstructed_2026-09-06'
        %w[id name scope rationale].each { |k| errors << "wbs_field_invalid id=#{id} field=#{k}" unless text?(r[k]) }
        %w[evidence_or_catalog_refs calendar_constraints].each { |k| errors << "wbs_field_invalid id=#{id} field=#{k}" unless strings?(r[k]) && !r[k].empty? }
        errors << "wbs_field_invalid id=#{id} field=risk_class" unless %w[medium high critical].include?(r['risk_class'])
        errors << "wbs_field_invalid id=#{id} field=engineer_count" unless r['engineer_count'] == 1
        errors << "wbs_field_invalid id=#{id} field=external_wait_business_days" unless natural?(r['external_wait_business_days'])
        estimates = r.values_at('optimistic_hours','most_likely_hours','pessimistic_hours')
        if estimates.all? { |n| positive?(n) } && estimates == estimates.sort
          o,m,p = estimates.map { |n| decimal(n) }
          errors << "wbs_pert_mismatch id=#{id}" unless r['pert_hours'] == rounded((o+4*m+p)/6)
        else
          errors << "wbs_estimate_invalid id=#{id}"
        end
        errors << "wbs_estimate_invalid id=#{id}" unless positive?(r['pert_hours'])
        unless strings?(r['dependencies']) && (r['dependencies'] - fids).empty?
          errors << "wbs_dependency_invalid id=#{id}"
        end
        if unit
          errors << "wbs_unit_dependencies_incomplete id=#{id}" unless strings?(r['dependencies']) && (CORE_DEPENDENCIES-r['dependencies']).empty?
          c = catalog['migration_units'].find { |x| x['id'] == id }
          if c
            %w[completion_owner phase_epics producer_ids].each do |key|
              expected = key == 'completion_owner' ? c[key] : c[key].sort
              errors << "wbs_catalog_snapshot_mismatch id=#{id} field=#{key}" unless r[key] == expected
            end
            errors << "wbs_catalog_unit_sha_mismatch id=#{id}" unless r['catalog_unit_sha256'] == Digest::SHA256.hexdigest(JSON.generate(canonical(c)))
          end
          rank = WAVES.index { |wave| wave.include?(id) }
          errors << "wbs_rank_invalid id=#{id}" unless r.key?('approved_promotion_rank') && r['approved_promotion_rank'] == (rank && rank+1)
          changing = r['changes_physical_owner']
          p, o = r.values_at('promotion_sessions','observation_sessions')
          valid_sessions = [true,false].include?(changing) && natural?(p) && natural?(o)
          if valid_sessions
            valid_sessions &&= changing ? p >= 1 && o >= (%w[high critical].include?(r['risk_class']) ? 2 : 1) : p == 0 && o == 0
          end
          errors << "wbs_sessions_invalid id=#{id}" unless valid_sessions
        else
          index = fids.index(id)
          errors << "wbs_foundation_name_invalid id=#{id}" unless index && r['name'] == FOUNDATION_NAMES[index]
        end
        errors.concat(gate_errors(r, unit, catalog))
        refs = ['Q:62','Q:102','Q:84','Q:100'] + cids.map { |x| 'unit:'+x } + catalog['producers'].map { |x| 'producer:'+x['id'] }
        errors << "wbs_reference_invalid id=#{id}" unless strings?(r['evidence_or_catalog_refs']) && (r['evidence_or_catalog_refs']-refs).empty?
      end
      statements = (fs+us).flat_map { |r| g=r['acceptance_gates']; g.is_a?(Hash) && g['specific'].is_a?(Array) ? g['specific'].select { |x| x.is_a?(Hash) }.map { |x| x['statement'] } : [] }
      errors << 'wbs_gate_duplicate' unless statements.uniq.length == statements.length
      return errors.uniq.sort unless errors.empty?
      errors.concat(graph_errors(fs, us))
      return errors.uniq.sort unless errors.empty?
      errors.concat(total_errors(w))
      errors.uniq.sort
    end

    def text?(v)
      v.is_a?(String) && !v.strip.empty?
    end

    def objects?(v)
      v.is_a?(Array) && v.all? { |x| x.is_a?(Hash) }
    end

    def strings?(v)
      v.is_a?(Array) && v.all? { |x| text?(x) } && v.uniq == v
    end

    def natural?(v)
      v.is_a?(Integer) && v >= 0
    end

    def positive?(v)
      v.is_a?(Numeric) && v.finite? && v > 0
    end

    def canonical(v)
      case v
      when Hash then v.keys.sort.to_h { |k| [k, canonical(v[k])] }
      when Array then v.map { |x| canonical(x) }
      else v
      end
    end

    def gate_errors(r, unit, catalog)
      id, g = r.values_at('id','acceptance_gates')
      unless g.is_a?(Hash) && g.keys.sort == %w[shared specific] &&
             g['shared'] == (unit ? SHARED_GATES : []) && objects?(g['specific']) && !g['specific'].empty?
        return ["wbs_gates_invalid id=#{id}"]
      end
      errors = []
      g['specific'].each do |gate|
        unless gate.keys.sort == %w[id producer_ids requirements statement] && text?(gate['id']) &&
               gate['id'].start_with?(id.to_s+'-') && text?(gate['statement']) && strings?(gate['producer_ids']) && strings?(gate['requirements'])
          errors << "wbs_gates_invalid id=#{id}"
        end
      end
      return errors unless errors.empty?
      producers = g['specific'].flat_map { |x| x['producer_ids'] }.uniq.sort
      errors << "wbs_gate_binding_invalid id=#{id}" unless producers == (unit ? r['producer_ids'] : [])
      requirements = g['specific'].flat_map { |x| x['requirements'] }.uniq
      if id == 'MU-cli-replay-force' && !(REPLAY - requirements).empty?
        errors << "wbs_replay_gate_invalid id=#{id}"
      end
      if unit && Array(r['producer_ids']).any? { |p| text?(p) && p.start_with?('startup-resume') } && !(RECOVERY-requirements).empty?
        errors << "wbs_startup_gate_invalid id=#{id}"
      end
      inactive_producers = catalog['kinds'].select { |k| %w[STARVED OPT-IN].include?(k['status']) }.flat_map { |k| k['producer_ids'] }
      if unit && !(Array(r['producer_ids']) & inactive_producers).empty?
        unless (INACTIVE-requirements).empty? && r['changes_physical_owner'] == false
          errors << "wbs_inactive_gate_invalid id=#{id}"
        end
      end
      errors
    end

    def graph_errors(fs, us)
      rows = (fs+us).to_h { |r| [r['id'], r] }
      visiting, done = {}, {}
      visit = lambda do |id|
        return false if visiting[id]
        return true if done[id]
        visiting[id] = true
        return false unless rows[id]['dependencies'].all? { |dep| visit.call(dep) }
        visiting.delete(id)
        done[id] = true
        true
      end
      rows.keys.sort.all? { |id| visit.call(id) } ? [] : ['wbs_dependency_cycle']
    end

    def longest_path(rows)
      by_id = rows.to_h { |r| [r['id'],r] }
      memo = {}
      visit = lambda do |id|
        return memo[id] if memo.key?(id)
        r = by_id.fetch(id)
        previous = r['dependencies'].map { |d| visit.call(d) }.sort_by { |p| [-p['hours'],p['ids'].join(',')] }.first || {'ids'=>[],'hours'=>0}
        memo[id] = {'ids'=>previous['ids']+[id], 'hours'=>rounded(decimal(previous['hours'])+decimal(r['pert_hours']))}
      end
      rows.map { |r| visit.call(r['id']) }.sort_by { |p| [-p['hours'],p['ids'].join(',')] }.first
    end

    def total_errors(w)
      fs, us = w.values_at('foundation_work_packages','migration_units')
      rows = fs + us
      errors = []
      base = sum(rows,'pert_hours')
      buffer = rounded(decimal(base)*decimal(w['contingency']['percent'])/100)
      buffered = rounded(decimal(base)+decimal(buffer))
      e = {'foundation_pert_hours'=>sum(fs,'pert_hours'),'migration_pert_hours'=>sum(us,'pert_hours'),
           'baseline_hours'=>base,'baseline_engineering_days'=>rounded(decimal(base)/8),
           'contingency_hours'=>buffer,'buffered_hours'=>buffered,'buffered_engineering_days'=>rounded(decimal(buffered)/8)}
      errors << 'wbs_engineering_totals_mismatch' unless w['engineering_totals'] == e
      p, o = sum(us,'promotion_sessions').to_i, sum(us,'observation_sessions').to_i
      t = {'physical_owner_units'=>us.count { |r| r['changes_physical_owner'] }, 'promotion_sessions'=>p,'observation_sessions'=>o,
           'serialized_rollout_sessions'=>p+o, 'promotion_business_day_lower_bound'=>p,
           'session_policy'=>'one_unit_per_business_date; observation_serialized_after_each_promotion','unranked_requires_new_approval'=>true}
      errors << 'wbs_trading_totals_mismatch' unless w['trading_totals'] == t
      c = w['calendar_scenarios']
      external = sum(rows,'external_wait_business_days').to_i
      engineering_days = (decimal(buffered)/8).ceil
      business = engineering_days+p+o+external
      expected = {'planning_start'=>'hypothetical_monday_not_a_committed_date','workdays_per_week'=>5,
        'external_wait_business_days_serial_sum'=>external, 'buffered_engineering_business_days'=>engineering_days,
        'serialized_business_days'=>business, 'weekdays_only_calendar_days'=>((business-1)/5)*7+(business-1)%5+1,
        'additional_exchange_holidays'=>'add_authoritative_calendar_closures','approval_and_sample_delay_days'=>nil,'upper_bound_calendar_days'=>nil}
      unless c.is_a?(Hash) && c.keys.sort == (expected.keys+['scope']).sort && expected.all? { |k,v| c[k] == v } && text?(c['scope'])
        errors << 'wbs_calendar_mismatch'
      end
      cp = w['critical_path']
      return errors + ['wbs_critical_path_invalid'] unless cp.is_a?(Hash) && cp.keys.sort == %w[engineering trading_rollout]
      eng = cp['engineering']
      first = us.select { |r| (1..3).include?(r['approved_promotion_rank']) }
      first_ids = first.map { |r| r['id'] }.sort
      longest = longest_path(rows)
      expected = {'resource_policy'=>'one_engineer_serial','serialized_hours'=>base,
        'dependency_longest_path'=>longest['ids'],'dependency_path_hours'=>longest['hours'],
        'first_batch_unit_ids'=>first_ids,'first_batch_hours'=>sum(fs+first,'pert_hours')}
      order = eng.is_a?(Hash) ? eng['serial_order'] : nil
      valid_order = strings?(order) && order.sort == rows.map { |r| r['id'] }.sort &&
                    rows.all? { |r| r['dependencies'].all? { |d| order.index(d) < order.index(r['id']) } } &&
                    order.take(fs.length+first.length).sort == (fs.map { |r| r['id'] }+first_ids).sort
      unless eng.is_a?(Hash) && eng.keys.sort == (expected.keys+['serial_order']).sort &&
             expected.all? { |k,v| eng[k] == v } && valid_order
        errors << 'wbs_critical_path_invalid'
      end
      expected = {'wave_groups'=>WAVES.each_with_index.map { |ids,i| {'rank'=>i+1,'unit_ids'=>ids} },
        'within_wave_order'=>'operator_approval_required', 'unranked_unit_ids'=>us.select { |r| r['approved_promotion_rank'].nil? }.map { |r| r['id'] }.sort,
        'first_batch_unit_ids'=>first_ids,'first_batch_sessions'=>sum(first,'promotion_sessions')+sum(first,'observation_sessions')}
      errors << 'wbs_trading_path_invalid' unless cp['trading_rollout'] == expected
      errors
    end

    def replace(text, body)
      unless text.scan(BEGIN_MARKER).length == 1 && text.scan(END_MARKER).length == 1 &&
             text.index(BEGIN_MARKER) < text.index(END_MARKER)
        raise Invalid, 'wbs_markers_invalid'
      end
      from = text.index(BEGIN_MARKER) + BEGIN_MARKER.length
      to = text.index(END_MARKER)
      text[0...from] + "\n" + body + "\n" + text[to..-1]
    end

    def cell(value)
      value = value.to_s('F') if value.is_a?(BigDecimal)
      value.to_s.gsub('|', '&#124;').gsub('[', '&#91;').gsub(']', '&#93;').gsub("\n", '<br>')
    end

    def table(headers, rows)
      ([headers, headers.map { '---' }] + rows).map { |row| '| ' + row.map { |v| cell(v) }.join(' | ') + ' |' }.join("\n") + "\n"
    end

    def render(w)
      w = render_values(w)
      e, t, c, cp = w.values_at('engineering_totals', 'trading_totals', 'calendar_scenarios', 'critical_path')
      fs, us = w.values_at('foundation_work_packages', 'migration_units')
      text = "## WBS 确定性摘要（PROVISIONAL）\n\n"
      text += "事实源为 [push-system-wbs.v1.json](push-system-wbs.v1.json)。本区间仅为生成视图；修改事实源后运行 render-wbs.rb --write。\n\n"
      text += "PROVISIONAL：规格非实现、非部署、非生产验收。旧 W01--W21 合计 98--142h 仅为历史对照，无法恢复旧逐项表；本表 lineage=reconstructed_2026-09-06，不拟合旧范围。\n\n"
      text += "catalog SHA256：`#{w['catalog_sha256']}`。Unit hash 对原catalog对象递归排序键后编码无空格/换行UTF-8 JSON，数组保持原顺序；这是对象快照hash，不是新增运行时身份合同。\n\n"
      text += "### Foundation：恰好 W01--W21\n\n"
      text += table(%w[ID 工作包 O/M/P小时 PERT小时 依赖 风险 验收], fs.map { |r| [r['id'], r['name'], %w[optimistic_hours most_likely_hours pessimistic_hours].map { |k| r[k] }.join('/'), r['pert_hours'], r['dependencies'].join(','), r['risk_class'], r['acceptance_gates']['specific'].map { |g| g['statement'] }.join('；')] })
      text += "\nW21只交付发布编排与清理门禁工具；逐Unit cutover准备、验证及tail-cleanup资格核验在Unit估算中。保留期届满后的生产删除不属于本次规格交付。\n\n### 四 Epic / 52 Unit\n\n"
      text += table(%w[Epic 关联Unit数 关联PERT小时], w['assumptions']['phase_epics'].map { |phase| members = us.select { |r| r['phase_epics'].include?(phase) }; [phase, members.length, sum(members, 'pert_hours')] })
      text += "\n跨Epic Unit在关联行重复展示，不能累加Epic行作为总数；去重后 #{us.length} Unit，#{e['migration_pert_hours']} 小时。\n\n### Q44 十波映射\n\n"
      text += table(%w[rank CatalogUnit physical-owner晋级session 观察session], cp['trading_rollout']['wave_groups'].map { |g| rows=us.select { |r| g['unit_ids'].include?(r['id']) }; [g['rank'], g['unit_ids'].join(', '), sum(rows, 'promotion_sessions'), sum(rows, 'observation_sessions')] })
      text += "\n同rank不代表有内部先后顺序：仍逐Unit逐交易日，同波内顺序须操作员另批。其他Unit rank=null，未经新批准不能追加为第十一波或按流量排序。rank1仅含default CLI单股/汇总的typed BestEffort结果；CLI产业链报告的历史批准范围有歧义，MU-cli-chain保持rank=null，纳入波次需要另行产品裁决；replay-force独立。rank6覆盖15:05所属共享owner的四入口；rank9仅七个ACTIVE ReviewTask，R03三owner rank=null。rank10是PaperReview保持STARVED的conformance，不授予物理owner。\n\n### 可复算时间与首批关键路径\n\n"
      text += "O/M/P包含实现、评审和修复。逐行 PERT=round-half-up((O+4M+P)/6,2)，总计仅加保存的逐行PERT。Foundation #{e['foundation_pert_hours']}h + Unit #{e['migration_pert_hours']}h = #{e['baseline_hours']}h / 8 = #{e['baseline_engineering_days']}工程日。\n\n"
      text += "单开发者串行；缓冲只在总PERT上应用一次 #{w['contingency']['percent']}%=#{e['contingency_hours']}h。工程区间为baseline #{e['baseline_hours']}h至含缓冲 #{e['buffered_hours']}h，即 #{e['baseline_engineering_days']}至#{e['buffered_engineering_days']}个8小时工程日。外部等待/交易观察/同一风险不重复进入工时。\n\n"
      text += "工程DAG最长依赖路径：#{cp['engineering']['dependency_longest_path'].join(' → ')} = #{cp['engineering']['dependency_path_hours']}h；这不是单开发者总历时。完整资源串行顺序存于JSON，可检查每条依赖。首批工程是全部Foundation加rank1--3的 #{cp['engineering']['first_batch_unit_ids'].join(', ')}，共#{cp['engineering']['first_batch_hours']}h（无缓冲）。\n\n"
      text += "交易独立计算：#{t['physical_owner_units']}个owner-changing Unit，#{t['promotion_sessions']}次晋级 + #{t['observation_sessions']}次独立观察 = #{t['serialized_rollout_sessions']}个串行eligible session；单日全局最多晋级一个Unit，下限#{t['promotion_business_day_lower_bound']}个晋级交易日。观察按每Unit晋级后串行保守场景；高风险/业务副作用至少两观察session，纯shadow不占名额。首批rank1--3至少#{cp['trading_rollout']['first_batch_sessions']}个session，同rank排列须另批。\n\n"
      text += "自然日场景从假设周一开始且不承诺日期：ceil(#{e['buffered_engineering_days']})=#{c['buffered_engineering_business_days']}工程工作日 + #{t['serialized_rollout_sessions']}交易session + #{c['external_wait_business_days_serial_sum']}外部等待工作日 = #{c['serialized_business_days']}个串行业务日；只排周末时 7*floor((N-1)/5)+(N-1)%5+1 = #{c['weekdays_only_calendar_days']}自然日。该保守无重叠场景须另加交易所休市、人工批准和真实样本延迟，上限为null；非承诺，亦非把交易日直接当自然日。STARVED/OPT-IN激活及至少90天/更严留存届满等待均不在此场景，未排序Unit须新批准。\n\n"
      text += "近期08-31--09-04仅影响设计、回放与预修复，门禁引用RFC既有业务样本；PaperBuy/Watchdog仅nonbaseline反例，不新增第53/54 Unit。\n\n### 完整 52 Unit 附录\n\n"
      us.each do |r|
        text += "#### #{r['id']} — #{r['name']}\n\n"
        text += "owner：#{cell(r['completion_owner'])}。Epic：#{r['phase_epics'].join('/')}；producer：#{r['producer_ids'].join(', ')}。快照SHA：`#{r['catalog_unit_sha256']}`。\n\n"
        text += "#{r['scope']} O/M/P=#{r['optimistic_hours']}/#{r['most_likely_hours']}/#{r['pessimistic_hours']}h；PERT=#{r['pert_hours']}h；风险=#{r['risk_class']}；外部等待=#{r['external_wait_business_days']}工作日；owner change=#{r['changes_physical_owner']}；rank=#{r['approved_promotion_rank'] || 'null'}；晋级/观察=#{r['promotion_sessions']}/#{r['observation_sessions']} session。\n\n"
        text += "依赖：#{r['dependencies'].join(', ')}。日历：#{r['calendar_constraints'].join('；')}。估算依据：#{r['rationale']}。\n\n"
        text += "六类共享门禁：#{r['acceptance_gates']['shared'].join(', ')}（每Unit/build重新取证）；专属门禁：#{r['acceptance_gates']['specific'].map { |g| g['statement'] }.join('；')}\n\n"
      end
      text + "发布边界：draft校验只证明文档一致性；strict必须返回 wbs_status_provisional，不能用本WBS宣称实现、部署或真实接收完成。\n"
    end

    def decimal(n)
      n.is_a?(Rational) ? n : BigDecimal(n.to_s).to_r
    end

    def rounded(n)
      # Keep the raw JSON decimal exact through division and half-up rounding.
      value = decimal(n)
      cents = (value.abs * 100 + Rational(1, 2)).floor * (value < 0 ? -1 : 1)
      BigDecimal(cents.to_s) / 100
    end

    def sum(rows, key)
      rounded(rows.reduce(Rational(0)) { |n, r| n + decimal(r.fetch(key)) })
    end

    def render_values(value)
      case value
      when BigDecimal then value.to_s('F')
      when Hash then value.to_h { |key, item| [key, render_values(item)] }
      when Array then value.map { |item| render_values(item) }
      else value
      end
    end
  end
end
