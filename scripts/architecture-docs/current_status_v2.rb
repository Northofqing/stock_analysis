# frozen_string_literal: true

require 'digest'
require 'json'
require 'open3'
require_relative 'rust_evidence'

module ArchitectureDocs
  # A narrow, immutable source-status delta. It is deliberately separate from
  # the W06 runtime catalog and the older all-source audit manifest.
  module CurrentStatusV2
    JSON_PATH = 'docs/push-system/push-current-capability-status.v2.json'
    MARKDOWN_PATH = 'docs/push-system/push-current-capability-status.v2.md'
    HISTORICAL_PATH = 'docs/push-system/push-capability-catalog.v1.json'
    CURRENT_V1_PATH = 'docs/push-system/push-current-capability-catalog.v1.json'
    SOURCE_PATHS = %w[src/bin/monitor/main.rs src/bin/monitor/push_templates.rs].freeze
    KIND_FACTS = {
      'IndustryChain' => {
        'base_status' => 'STARVED', 'observed_status' => 'ACTIVE', 'new_claim_path' => 'CONDITIONAL',
        'facts' => { 'scheduler_wired' => true, 'account_metrics_complete_required' => true,
                     'historical_decision_resume_independent' => true }
      },
      'PostFixedPriceOrder' => {
        'base_status' => 'STARVED', 'observed_status' => 'STARVED', 'new_claim_path' => 'BLOCKED',
        'facts' => { 'scheduler_wired' => true, 'source_registered' => false }
      },
      'PostFixedPriceFill' => {
        'base_status' => 'STARVED', 'observed_status' => 'STARVED', 'new_claim_path' => 'BLOCKED',
        'facts' => { 'scheduler_wired' => true, 'source_registered' => false,
                     'after_hours_schedule_reachable' => false }
      },
      'BlockTradePriceRange' => {
        'base_status' => 'INACTIVE', 'observed_status' => 'INACTIVE', 'new_claim_path' => 'BLOCKED',
        'facts' => { 'side_route_wired' => true, 'required_range_present' => false }
      }
    }.freeze
    EVIDENCE_IDS_BY_KIND = {
      'IndustryChain' => %w[r03-account-phase r03-banner r03-counted],
      'PostFixedPriceOrder' => %w[trade-schedule trade-source-fetch trade-source-register],
      'PostFixedPriceFill' => %w[trade-schedule trade-source-fetch trade-source-register],
      'BlockTradePriceRange' => %w[block-review-route block-range-guard]
    }.freeze

    module_function

    def load(root)
      path = File.join(root, JSON_PATH)
      return [nil, ['v2_json_path_invalid']] unless regular_file?(path)

      [JSON.parse(File.binread(path)), []]
    rescue JSON::ParserError
      [nil, ['v2_json_invalid']]
    end

    def validate(root, document)
      errors = []
      unless document.is_a?(Hash) && document.keys.sort == %w[
        current_v1_catalog_sha256 evidence historical_catalog_sha256 kinds role schema_version scope source_commit status
      ].sort
        return ['v2_schema_invalid']
      end
      errors << 'v2_header_invalid' unless document['schema_version'] == 2 &&
                                           document['role'] == 'current-source-status-delta' &&
                                           document['status'] == 'PROVISIONAL' &&
                                           nonempty?(document['scope'])
      source_commit = document['source_commit']
      errors << 'v2_source_commit_invalid' unless sha40?(source_commit)
      { 'historical_catalog_sha256' => HISTORICAL_PATH,
        'current_v1_catalog_sha256' => CURRENT_V1_PATH }.each do |field, relative|
        path = File.join(root, relative)
        unless regular_file?(path) && document[field] == Digest::SHA256.file(path).hexdigest
          errors << "v2_base_digest_mismatch field=#{field}"
        end
      end
      return errors unless errors.empty?
      errors.concat(validate_base_catalogs(root, document['kinds']))
      return errors unless errors.empty?

      head, ok = git(root, 'rev-parse', 'HEAD')
      _ancestor, ancestor_ok = git(root, 'merge-base', '--is-ancestor', source_commit, head.strip) if ok
      errors << 'v2_source_commit_not_ancestor' unless ok && ancestor_ok
      errors.concat(validate_kinds(document['kinds']))
      errors.concat(validate_evidence(root, source_commit, document['evidence'], document['kinds']))
      errors.concat(validate_trade_source_absence(root, source_commit)) if errors.empty?
      errors.uniq
    end

    def validate_base_catalogs(root, kinds)
      historical = JSON.parse(File.binread(File.join(root, HISTORICAL_PATH)))
      current = JSON.parse(File.binread(File.join(root, CURRENT_V1_PATH)))
      return ['v2_base_catalog_shape_invalid'] unless [historical, current].all? do |catalog|
        catalog.is_a?(Hash) && catalog['kinds'].is_a?(Array) && catalog['kinds'].length == 65 &&
          catalog['producers'].is_a?(Array) && catalog['producers'].length == 102 &&
          catalog['migration_units'].is_a?(Array) && catalog['migration_units'].length == 52
      end

      historical_kinds = historical['kinds'].to_h { |entry| [entry['kind'], entry['status']] }
      current_kinds = current['kinds'].to_h { |entry| [entry['kind'], entry['status']] }
      return ['v2_base_identity_mismatch'] unless historical_kinds == current_kinds && historical_kinds.length == 65
      return ['v2_base_status_mismatch'] unless kinds.is_a?(Array) && kinds.all? do |entry|
        entry.is_a?(Hash) && current_kinds[entry['kind']] == entry['base_status']
      end

      []
    rescue JSON::ParserError, KeyError, TypeError
      ['v2_base_catalog_shape_invalid']
    end

    def validate_kinds(kinds)
      return ['v2_kinds_invalid'] unless kinds.is_a?(Array) && kinds.length == KIND_FACTS.length

      errors = []
      names = kinds.map { |entry| entry.is_a?(Hash) ? entry['kind'] : nil }
      errors << 'v2_kind_set_mismatch' unless names.all? { |name| nonempty?(name) } &&
                                              names.sort == KIND_FACTS.keys.sort
      kinds.each do |entry|
        next unless entry.is_a?(Hash)

        name = entry['kind']
        expected = KIND_FACTS[name]
        next unless expected

        errors << "v2_kind_shape_invalid kind=#{name}" unless entry.keys.sort == %w[
          base_status evidence_ids facts kind new_claim_path note observed_status
        ].sort && nonempty?(entry['note']) &&
                                                            string_list?(entry['evidence_ids']) && !entry['evidence_ids'].empty?
        expected.each do |field, value|
          errors << "v2_kind_fact_mismatch kind=#{name} field=#{field}" unless entry[field] == value
        end
        errors << "v2_kind_evidence_mismatch kind=#{name}" unless entry['evidence_ids'] == EVIDENCE_IDS_BY_KIND[name]
      end
      errors
    end

    def validate_evidence(root, source_commit, evidence, kinds)
      return ['v2_evidence_invalid'] unless evidence.is_a?(Array) && !evidence.empty?

      errors = []
      ids = evidence.map { |entry| entry.is_a?(Hash) ? entry['id'] : nil }
      errors << 'v2_evidence_ids_invalid' unless ids.all? { |id| nonempty?(id) } && ids.uniq == ids
      references = kinds.is_a?(Array) ? kinds.flat_map { |entry| entry.is_a?(Hash) ? Array(entry['evidence_ids']) : [] } : []
      errors << 'v2_evidence_closure_mismatch' unless ids.all? { |id| nonempty?(id) } &&
                                                    references.all? { |id| nonempty?(id) } &&
                                                    references.uniq.sort == ids.sort
      sources = {}
      views = {}
      evidence.each do |entry|
        next unless entry.is_a?(Hash)

        id = entry['id']
        valid = entry.keys.sort == %w[id path required_fragments symbol symbol_sha256].sort &&
                SOURCE_PATHS.include?(entry['path']) && nonempty?(entry['symbol']) &&
                sha256?(entry['symbol_sha256']) && string_list?(entry['required_fragments']) &&
                !entry['required_fragments'].empty?
        unless valid
          errors << "v2_evidence_shape_invalid id=#{id}"
          next
        end
        path = entry['path']
        unless sources.key?(path)
          bytes, ok = git(root, 'show', "#{source_commit}:#{path}")
          unless ok
            errors << "v2_source_missing path=#{path}"
            next
          end
          sources[path] = bytes
          views[path] = RustEvidence.view(bytes)
        end
        begin
          located = views.fetch(path).locate(entry['symbol'], 'rust_fn')
          errors << "v2_symbol_sha_mismatch id=#{id}" unless located['symbol_sha256'] == entry['symbol_sha256']
          entry['required_fragments'].each do |fragment|
            errors << "v2_fragment_missing id=#{id}" unless located['body'].include?(fragment)
          end
        rescue RustEvidence::Invalid
          errors << "v2_symbol_invalid id=#{id}"
        end
      end
      errors
    end

    def validate_trade_source_absence(root, source_commit)
      output, ok = git(root, 'grep', '-l', '-F', 'register_trade_event_source', source_commit, '--', 'src')
      return ['v2_trade_source_search_failed'] unless ok

      declarations = 0
      calls = 0
      output.lines.map(&:strip).each do |entry|
        prefix = "#{source_commit}:"
        return ['v2_trade_source_search_invalid'] unless entry.start_with?(prefix)

        path = entry.delete_prefix(prefix)
        return ['v2_trade_source_search_invalid'] unless path.match?(/\Asrc\/[A-Za-z0-9_\/-]+\.rs\z/)

        bytes, found = git(root, 'show', "#{source_commit}:#{path}")
        return ['v2_trade_source_search_failed'] unless found

        masked = RustEvidence.mask(bytes)
        declarations += masked.scan(/\bfn\s+register_trade_event_source\s*\(/).length
        calls += masked.scan(/\bregister_trade_event_source\s*\(/).length
      end
      declarations == 1 && calls == 1 ? [] : ['v2_trade_source_registration_found']
    rescue RustEvidence::Invalid
      ['v2_trade_source_search_invalid']
    end

    def render(document, current_v1)
      kinds = current_v1.fetch('kinds').to_h { |entry| [entry.fetch('kind'), entry.fetch('status')] }
      document.fetch('kinds').each { |entry| kinds[entry.fetch('kind')] = entry.fetch('observed_status') }
      counts = kinds.values.each_with_object(Hash.new(0)) { |status, tally| tally[status] += 1 }
      lines = [
        '# 当前源码推送状态增量 v2', '',
        "状态：#{document.fetch('status')}。源码快照：`#{document.fetch('source_commit')}`。", '',
        markdown(document.fetch('scope')), '',
        "历史 catalog SHA-256：`#{document.fetch('historical_catalog_sha256')}`。", '',
        "current v1 catalog SHA-256：`#{document.fetch('current_v1_catalog_sha256')}`。", '',
        "沿用 v1 的其余状态后，65-kind 投影为 ACTIVE #{counts.fetch('ACTIVE', 0)}、INACTIVE #{counts.fetch('INACTIVE', 0)}、STARVED #{counts.fetch('STARVED', 0)}、OPT-IN #{counts.fetch('OPT-IN', 0)}。此处只描述固定源码快照，不更改运行时目录。", '',
        '| kind | v1 状态 | 源码状态 | 新 claim 路径 | 结构化事实 | 说明与证据 |',
        '| --- | --- | --- | --- | --- | --- |'
      ]
      document.fetch('kinds').each do |entry|
        facts = entry.fetch('facts').map { |key, value| "`#{key}=#{value}`" }.join('、')
        evidence = entry.fetch('evidence_ids').map { |id| "[#{id}](##{id})" }.join('、')
        note = markdown(entry.fetch('note'))
        lines << "| `#{entry.fetch('kind')}` | #{entry.fetch('base_status')} | #{entry.fetch('observed_status')} | #{entry.fetch('new_claim_path')} | #{facts} | #{note}<br>#{evidence} |"
      end
      lines.concat(['', '## 源码证据', ''])
      document.fetch('evidence').each do |entry|
        lines << "### #{entry.fetch('id')}"
        lines << ''
        lines << "`#{entry.fetch('path')}::#{entry.fetch('symbol')}`；symbol SHA-256 `#{entry.fetch('symbol_sha256')}`。"
        lines << ''
      end
      lines << 'v2 中的 ACTIVE、STARVED、INACTIVE 是源码能力观察，不是 activation、真实批次、typed receipt 或生产送达证明。'
      lines << ''
      lines.join("\n")
    end

    def regular_file?(path)
      File.file?(path) && !File.symlink?(path) && File.stat(path).nlink == 1
    rescue SystemCallError
      false
    end

    def git(root, *args)
      out, _err, status = Open3.capture3({ 'GIT_OPTIONAL_LOCKS' => '0' }, 'git', '-C', root, *args)
      [out, status.success?]
    end

    def nonempty?(value)
      value.is_a?(String) && !value.strip.empty?
    end

    def string_list?(value)
      value.is_a?(Array) && value.all? { |entry| nonempty?(entry) } && value.uniq == value
    end

    def sha40?(value)
      value.is_a?(String) && value.match?(/\A[0-9a-f]{40}\z/)
    end

    def sha256?(value)
      value.is_a?(String) && value.match?(/\A[0-9a-f]{64}\z/)
    end

    def markdown(value)
      value.to_s.gsub('&', '&amp;').gsub('<', '&lt;').gsub('>', '&gt;').gsub('|', '&#124;').gsub(/\r?\n/, '<br>')
    end
  end
end
