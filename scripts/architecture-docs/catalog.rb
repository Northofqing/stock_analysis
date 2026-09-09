# frozen_string_literal: true

require 'json'
require 'open3'
require_relative 'source_catalog'
require_relative 'rust_evidence'

module ArchitectureDocs
  module Catalog
    class GitBatchInvalid < StandardError; end

    CATALOG_PATH = 'docs/push-system/push-capability-catalog.v1.json'
    MANIFEST_PATH = 'docs/push-system/push-evidence-manifest.v1.json'
    PHASES = %w[盘前 集合竞价 盘中 盘后].freeze
    STATUSES = %w[ACTIVE INACTIVE STARVED OPT-IN].freeze
    EVIDENCE_KINDS = %w[rust_fn rust_enum rust_impl rust_mod].freeze

    module_function

    def validate(root, strict: true)
      errors = SourceCatalog.validate(root)
      errors.concat(validate_pair(root, strict: strict, current: false))
      errors.concat(validate_pair(root, strict: strict, current: true))
      if strict
        status, success = git(root, 'status', '--porcelain', '--untracked-files=all')
        errors << 'git_status_failed' unless success
        errors << 'worktree_dirty' unless status.empty?
      end
      errors.uniq
    end

    def validate_pair(root, strict:, current:)
      pair = current ? 'current' : 'historical'
      pair_errors(root, strict: strict, current: current).map { |error| "#{error} pair=#{pair}" }
    end

    def pair_errors(root, strict:, current:)
      root = File.realpath(File.expand_path(root))
      return ['push_root_invalid'] unless File.directory?(root)
      errors = []
      paths = current ? [CurrentAudit::CATALOG_PATH, CurrentAudit::MANIFEST_PATH] : [CATALOG_PATH, MANIFEST_PATH]
      documents = paths.map do |path|
        safe = regular_path(root, path)
        unless safe
          errors << "push_path_invalid path=#{path}"
          next nil
        end
        unless File.file?(safe)
          errors << "push_document_missing path=#{path}"
          next nil
        end
        begin
          document = JSON.parse(File.binread(safe))
        rescue JSON::ParserError => error
          errors << "push_json_invalid path=#{path} detail=#{error.message}"
          next nil
        end
        errors << "push_schema_invalid path=#{path}" unless document.is_a?(Hash) && document['schema_version'] == 1
        errors << "provisional path=#{path}" if strict && document.is_a?(Hash) && document['status'] == 'PROVISIONAL'
        document
      end
      catalog, manifest = documents
      structure = schema_errors(catalog, manifest)
      structure.concat(CurrentAudit.schema_errors(catalog, manifest)) if current && structure.empty?
      errors.concat(structure)
      return errors unless structure.empty?
      errors.concat(current ? CurrentAudit.reference_errors(root, catalog, manifest) : reference_errors(catalog, manifest))
      errors.concat(git_errors(root, catalog, manifest, workspace: current))
      return errors unless current
      current_sources = {}
      manifest['files'].each do |file|
        safe = regular_path(root, file['path'])
        unless safe && File.file?(safe)
          errors << "file_path_invalid path=#{file['path']}"
          next
        end
        bytes = File.binread(safe)
        current_sources[file['path']] = bytes
        errors << "file_sha_mismatch path=#{file['path']}" unless Digest::SHA256.hexdigest(bytes) == file['sha256']
      end
      current_views = {}
      manifest['evidence'].each do |evidence|
        begin
          safe = regular_path(root, evidence['path'])
          unless safe && File.file?(safe)
            errors << "evidence_path_invalid id=#{evidence['id']}"
            next
          end
          source = current_sources[evidence['path']] ||= File.binread(safe)
          view = current_views[evidence['path']] ||= RustEvidence.view(source)
          located = view.locate(evidence['symbol'], evidence['kind'])
          errors << "symbol_sha_mismatch id=#{evidence['id']}" unless located['symbol_sha256'] == evidence['symbol_sha256']
          errors << "symbol_lines_mismatch id=#{evidence['id']}" unless %w[start_line end_line].all? { |field| located[field] == evidence[field] }
        rescue RustEvidence::Invalid => error
          errors << "#{error.message} id=#{evidence['id']}"
        end
      end
      entry = manifest['evidence'].find { |evidence| evidence['id'] == catalog['enum_evidence_id'] }
      if entry && entry['kind'] == 'rust_enum' && (safe = SourceCatalog.safe_path(root, entry['path'])) && File.file?(safe)
        begin
          source = current_sources[entry['path']] ||= File.binread(safe)
          view = current_views[entry['path']] ||= RustEvidence.view(source)
          item = view.locate(entry['symbol'], entry['kind'])
          actual = RustEvidence.enum_variants(item)
          expected = catalog['kinds'].map { |kind| kind['kind'] }
          errors << 'enum_coverage_mismatch' unless actual.sort == expected.sort && actual.uniq == actual
        rescue RustEvidence::Invalid => error
          errors << "#{error.message} id=#{entry['id']}"
        end
      else
        errors << 'enum_evidence_missing'
      end
      errors.uniq
    rescue RustEvidence::Invalid => error
      errors + [error.message]
    rescue Errno::ENOENT => error
      (errors || []) + ["push_document_missing detail=#{error.message}"]
    rescue JSON::ParserError => error
      errors + ["push_json_invalid detail=#{error.message}"]
    rescue SystemCallError => error
      (errors || []) + ["push_io_error detail=#{error.message}"]
    end

    def schema_errors(catalog, manifest)
      errors = []
      check_object(catalog, {
        'schema_version' => :version, 'status' => :provisional, 'baseline_commit' => :commit,
        'scope' => :text, 'enum_evidence_id' => :text, 'kinds' => :objects,
        'producers' => :objects, 'migration_units' => :objects, 'excluded_worktree_additions' => :objects
      }, 'catalog', errors)
      check_object(manifest, {
        'schema_version' => :version, 'status' => :provisional, 'baseline_commit' => :commit,
        'files' => :objects, 'evidence' => :objects
      }, 'manifest', errors)
      return errors unless errors.empty?
      catalog['kinds'].each do |kind|
        check_object(kind, { 'kind' => :text, 'primary_phase' => :phase, 'status' => :status,
                            'producer_ids' => :strings, 'evidence_ids' => :references, 'note' => :text }, 'kind', errors)
      end
      catalog['producers'].each do |producer|
        check_object(producer, {
          'id' => :text, 'kinds' => :strings, 'phase_epics' => :phases, 'occurrence_family' => :text,
          'completion_owner' => :text, 'migration_unit_id' => :text, 'trigger' => :relation,
          'source' => :relation, 'authority' => :relation, 'policy' => :relation,
          'evidence_ids' => :references, 'known_gaps' => :strings
        }, 'producer', errors)
      end
      catalog['migration_units'].each do |unit|
        check_object(unit, { 'id' => :text, 'producer_ids' => :references, 'completion_owner' => :text,
                             'occurrence_families' => :references, 'phase_epics' => :phases, 'note' => :text }, 'unit', errors)
      end
      catalog['excluded_worktree_additions'].each do |entry|
        check_object(entry, { 'kind' => :text, 'reason' => :text }, 'excluded', errors)
      end
      manifest['files'].each { |file| check_object(file, { 'path' => :path, 'sha256' => :sha }, 'file', errors) }
      manifest['evidence'].each do |entry|
        check_object(entry, { 'id' => :text, 'path' => :path, 'symbol' => :text, 'kind' => :evidence_kind,
                              'symbol_sha256' => :sha, 'start_line' => :line, 'end_line' => :line }, 'evidence', errors)
      end
      errors
    end

    def check_object(object, fields, context, errors)
      unless object.is_a?(Hash)
        errors << "push_structure_invalid context=#{context}"
        return
      end
      fields.each do |field, type|
        value = object[field]
        valid = case type
                when :version then value.is_a?(Integer) && value == 1
                when :provisional then value == 'PROVISIONAL'
                when :text then SourceCatalog.nonempty_string?(value)
                when :sha then SourceCatalog.sha256?(value)
                when :commit then value.is_a?(String) && value.match?(/\A[0-9a-f]{40}\z/)
                when :path then relative_path?(value)
                when :phase then PHASES.include?(value)
                when :status then STATUSES.include?(value)
                when :evidence_kind then EVIDENCE_KINDS.include?(value)
                when :line then value.is_a?(Integer) && value > 0
                when :objects then value.is_a?(Array) && value.all? { |entry| entry.is_a?(Hash) }
                when :strings, :references, :phases
                  value.is_a?(Array) && value.all? { |entry| SourceCatalog.nonempty_string?(entry) } &&
                    value.uniq == value && (type == :strings || !value.empty?) &&
                    (type != :phases || (value - PHASES).empty?)
                when :relation
                  value.is_a?(Hash) && SourceCatalog.nonempty_string?(value['description']) &&
                    value['evidence_ids'].is_a?(Array) && !value['evidence_ids'].empty? &&
                    value['evidence_ids'].all? { |id| SourceCatalog.nonempty_string?(id) } &&
                    value['evidence_ids'].uniq == value['evidence_ids']
                end
        unless valid
          code = type == :commit ? 'baseline_commit_invalid' : 'push_field_invalid'
          errors << "#{code} context=#{context} field=#{field}"
        end
      end
    end

    def relative_path?(value)
      SourceCatalog.nonempty_string?(value) && !value.include?("\0") && !value.start_with?('/') &&
        !value.include?('\\') && value.split('/').none? { |part| ['..', '.', ''].include?(part) }
    end

    def regular_path(root, relative)
      safe = SourceCatalog.safe_path(root, relative)
      expected = File.join(root, relative)
      return nil unless safe == expected && !File.symlink?(expected)
      return nil if File.exist?(safe) && (!File.file?(safe) || File.stat(safe).nlink != 1)

      safe
    end

    def reference_errors(catalog, manifest)
      errors = []
      [['kind', catalog['kinds'], 'kind'], ['producer', catalog['producers'], 'id'],
       ['unit', catalog['migration_units'], 'id'], ['evidence', manifest['evidence'], 'id'],
       ['file', manifest['files'], 'path']].each do |label, entries, field|
        SourceCatalog.duplicate_values(entries, field).each { |id| errors << "duplicate_#{label} id=#{id}" }
      end
      evidence_ids = manifest['evidence'].map { |entry| entry['id'] }
      manifest['evidence'].group_by { |entry| [entry['path'], entry['kind'], entry['symbol']] }.each do |locator, entries|
        errors << "duplicate_locator path=#{locator[0]} symbol=#{locator[2]}" if entries.length > 1
      end
      producers = catalog['producers'].map { |entry| entry['id'] }
      catalog['kinds'].each do |kind|
        errors << "producer_missing kind=#{kind['kind']}" if kind['status'] != 'INACTIVE' && kind['producer_ids'].empty?
        (kind['producer_ids'] - producers).each { |id| errors << "producer_reference_missing id=#{id}" }
      end
      referenced = [catalog['enum_evidence_id']]
      catalog['kinds'].each { |kind| referenced.concat(kind['evidence_ids']) }
      catalog['producers'].each do |producer|
        referenced.concat(producer['evidence_ids'])
        %w[trigger source authority policy].each { |field| referenced.concat(producer[field]['evidence_ids']) }
      end
      (referenced.uniq - evidence_ids).each { |id| errors << "evidence_reference_missing id=#{id}" }
      (evidence_ids - referenced.uniq).each { |id| errors << "evidence_unreferenced id=#{id}" }
      kinds = catalog['kinds']
      units = catalog['migration_units']
      catalog['producers'].each do |producer|
        reverse = kinds.select { |kind| kind['producer_ids'].include?(producer['id']) }.map { |kind| kind['kind'] }
        errors << "producer_kind_mismatch id=#{producer['id']}" unless reverse.sort == producer['kinds'].sort
        unit = units.find { |entry| entry['id'] == producer['migration_unit_id'] }
        errors << "unit_reference_missing id=#{producer['id']}" unless unit
        if unit
          errors << "completion_owner_mismatch id=#{producer['id']}" unless producer['completion_owner'] == unit['completion_owner']
          errors << "unit_producer_mismatch id=#{producer['id']}" unless unit['producer_ids'].include?(producer['id'])
        end
        nested = %w[trigger source authority policy].flat_map { |field| producer[field]['evidence_ids'] }.uniq
        errors << "producer_evidence_incomplete id=#{producer['id']}" unless (nested - producer['evidence_ids']).empty?
        if producer['kinds'].empty? && producer['known_gaps'].empty?
          errors << "enum_external_reason_missing id=#{producer['id']}"
        end
      end
      units.each do |unit|
        members = catalog['producers'].select { |producer| producer['migration_unit_id'] == unit['id'] }
        errors << "unit_producer_mismatch id=#{unit['id']}" unless members.map { |producer| producer['id'] }.sort == unit['producer_ids'].sort
        occurrences = members.map { |producer| producer['occurrence_family'] }.uniq.sort
        phases = members.flat_map { |producer| producer['phase_epics'] }.uniq.sort
        errors << "unit_occurrence_mismatch id=#{unit['id']}" unless occurrences == unit['occurrence_families'].sort
        errors << "unit_phase_mismatch id=#{unit['id']}" unless phases == unit['phase_epics'].sort
      end
      catalog['producers'].group_by { |producer| producer['completion_owner'] }.each do |owner, members|
        errors << "shared_owner_split owner=#{owner}" if members.map { |producer| producer['migration_unit_id'] }.uniq.length > 1
      end
      catalog['excluded_worktree_additions'].each do |entry|
        errors << "excluded_kind_in_enum kind=#{entry['kind']}" if kinds.any? { |kind| kind['kind'] == entry['kind'] }
      end
      excluded = catalog['excluded_worktree_additions'].map { |entry| entry['kind'] }.sort
      errors << 'excluded_worktree_additions_mismatch' unless excluded == %w[PaperBuy Watchdog]
      errors
    end

    def git(root, *args)
      out, _err, status = Open3.capture3({ 'GIT_OPTIONAL_LOCKS' => '0' }, 'git', '-C', root, *args)
      [out, status.success?]
    end

    def markdown(value)
      value.to_s.gsub('&', '&amp;').gsub('<', '&lt;').gsub('>', '&gt;').gsub('|', '&#124;').gsub(/\r?\n/, '<br>').gsub('`', '&#96;')
    end

    def render(catalog, manifest)
      evidence = manifest['evidence'].each_with_object({}) { |entry, map| map[entry['id']] = entry }
      lines = [
        '# 推送能力源审计目录', '',
        '状态：PROVISIONAL；ACTIVE 仅表示源码接线，不表示 Ready、已部署或已接收。', '',
        "代码基线：`#{catalog['baseline_commit']}`。", '', catalog['scope'], '',
        'NOT CHECKED：完整 RFC / WBS / 离线 HTML / CI / 运行时 Foundation / 部署 / 真实接收；不推导迁移顺序或工期。', '',
        '本文件由 JSON 目录生成。四时段是 Epic，MigrationUnit 按 occurrence 与 completion owner 归属。', ''
      ]
      PHASES.each do |phase|
        lines.concat(["## #{phase}", '', '| kind | 状态 | producer | Unit | 证据符号 | 说明 |', '| --- | --- | --- | --- | --- | --- |'])
        catalog['kinds'].select { |kind| kind['primary_phase'] == phase }.sort_by { |kind| kind['kind'] }.each do |kind|
          producers = catalog['producers'].select { |producer| kind['producer_ids'].include?(producer['id']) }
          symbols = kind['evidence_ids'].map { |id| evidence[id]['path'] + '::' + evidence[id]['symbol'] }
          cells = [kind['kind'], kind['status'], kind['producer_ids'].sort.join(', '),
                   producers.map { |producer| producer['migration_unit_id'] }.uniq.sort.join(', '), symbols.sort.join('; '), kind['note']]
          lines << '| ' + cells.map { |cell| markdown(cell) }.join(' | ') + ' |'
        end
        lines << ''
      end
      lines.concat(['## enum 外生产路径', ''])
      catalog['producers'].select { |producer| producer['kinds'].empty? }.sort_by { |producer| producer['id'] }.each do |producer|
        lines << "- #{markdown(producer['id'])}：#{markdown(producer['known_gaps'].join('；'))}（Unit #{markdown(producer['migration_unit_id'])}）"
      end
      lines.concat(['', '## producer 与完成边界', ''])
      catalog['producers'].sort_by { |producer| producer['id'] }.each do |producer|
        lines.concat(["### #{markdown(producer['id'])}", '',
                      "时段：#{markdown(producer['phase_epics'].join('、'))}；occurrence：#{markdown(producer['occurrence_family'])}；Unit：#{markdown(producer['migration_unit_id'])}。", '',
                      "completion owner：#{markdown(producer['completion_owner'])}。", ''])
        { 'trigger' => '触发', 'source' => '输入', 'authority' => '权威事实', 'policy' => '策略' }.each do |field, label|
          relation = producer[field]
          symbols = relation['evidence_ids'].map { |id| evidence[id]['path'] + '::' + evidence[id]['symbol'] }
          lines.concat(["#{label}：#{markdown(relation['description'])} 证据：#{markdown(symbols.join('；'))}。", ''])
        end
        lines.concat(["已知缺口：#{markdown(producer['known_gaps'].join('；'))}", '']) unless producer['known_gaps'].empty?
      end
      lines.concat(['## MigrationUnit', ''])
      catalog['migration_units'].sort_by { |unit| unit['id'] }.each do |unit|
        lines.concat(["- #{markdown(unit['id'])}：#{markdown(unit['producer_ids'].sort.join('、'))}；owner #{markdown(unit['completion_owner'])}。#{markdown(unit['note'])}"])
      end
      lines.concat(['', '## 原工作树未移入项', ''])
      catalog['excluded_worktree_additions'].sort_by { |entry| entry['kind'] }.each do |entry|
        lines << "- #{markdown(entry['kind'])}：#{markdown(entry['reason'])}。"
      end
      lines.concat(['', '## 稳定证据索引', '',
                    'Rust 符号按词法声明定位；impl 使用去掉 impl 和左花括号后的完整头，仅折叠空白，保留泛型、trait for 和 where。重复声明失败。模块必须有内联主体；无主体声明及不能可靠定位的语法不猜测。哈希包含声明行至闭合行的原始字节及实际行结束符。', '',
                    '| evidence | 文件 | 符号 | 行 | SHA256 |', '| --- | --- | --- | --- | --- |'])
      manifest['evidence'].sort_by { |entry| entry['id'] }.each do |entry|
        cells = [entry['id'], entry['path'], entry['symbol'], "#{entry['start_line']}–#{entry['end_line']}", entry['symbol_sha256']]
        lines << '| ' + cells.map { |cell| markdown(cell) }.join(' | ') + ' |'
      end
      lines.join("\n") + "\n"
    end

    def code_path?(path)
      (path.start_with?('src/') && path.end_with?('.rs')) || %w[Cargo.toml Cargo.lock].include?(path)
    end

    def parse_git_batch(objects, output)
      offset = 0
      blobs = {}
      objects.each do |expected|
        line_end = output.index("\n", offset)
        raise GitBatchInvalid, 'baseline_batch_truncated' unless line_end

        header = output.byteslice(offset, line_end - offset)
        object, type, length = header.split(' ', -1)
        unless object && object.match?(/\A[0-9a-f]{40}(?:[0-9a-f]{24})?\z/) && type && length &&
               header.split(' ', -1).length == 3
          raise GitBatchInvalid, 'baseline_batch_header_invalid'
        end
        raise GitBatchInvalid, 'baseline_batch_object_mismatch' unless object == expected
        raise GitBatchInvalid, 'baseline_batch_type_invalid' unless type == 'blob'
        raise GitBatchInvalid, 'baseline_batch_length_invalid' unless length.match?(/\A(?:0|[1-9][0-9]*)\z/)

        size = length.to_i
        body_start = line_end + 1
        body_end = body_start + size
        raise GitBatchInvalid, 'baseline_batch_truncated' if body_end >= output.bytesize
        raise GitBatchInvalid, 'baseline_batch_terminator_invalid' unless output.getbyte(body_end) == 10

        blobs[object] = output.byteslice(body_start, size)
        offset = body_end + 1
      end
      raise GitBatchInvalid, 'baseline_batch_extra' unless offset == output.bytesize

      blobs
    end

    def git_blobs(root, objects)
      objects = objects.uniq
      input = objects.empty? ? ''.b : (objects.join("\n") + "\n").b
      output, _error, status = Open3.capture3('git', '-C', root, 'cat-file', '--batch', stdin_data: input)
      raise GitBatchInvalid, 'baseline_batch_failed' unless status.success?

      parse_git_batch(objects, output.b)
    end

    def git_errors(root, catalog, manifest, workspace: true)
      errors = []
      baseline = manifest['baseline_commit']
      errors << 'baseline_mismatch' unless baseline == catalog['baseline_commit']
      type, exists = git(root, 'cat-file', '-t', baseline)
      unless exists && type.strip == 'commit'
        return errors + ["baseline_commit_invalid commit=#{baseline}"]
      end
      _output, ancestor = git(root, 'merge-base', '--is-ancestor', baseline, 'HEAD')
      errors << "baseline_not_ancestor commit=#{baseline}" unless ancestor
      tree, success = git(root, 'ls-tree', '-r', '-z', baseline)
      return errors + ['baseline_tree_failed'] unless success
      baseline_objects = {}
      baseline_paths = []
      tree.split("\0").reject(&:empty?).each do |entry|
        metadata, path = entry.split("\t", 2)
        fields = metadata && metadata.split(' ', 3)
        unless path && fields && fields.length == 3 && fields[2].match?(/\A[0-9a-f]{40}(?:[0-9a-f]{24})?\z/)
          return errors + ['baseline_tree_invalid']
        end
        if code_path?(path)
          baseline_paths << path
          baseline_objects[path] = fields[2] if fields[1] == 'blob' && %w[100644 100755].include?(fields[0])
        end
      end
      baseline_paths.sort!
      current_paths = Dir.glob(File.join(root, 'src/**/*.rs'), File::FNM_DOTMATCH).select { |path| File.file?(path) }.map { |path| path.delete_prefix(root + '/') }
      %w[Cargo.toml Cargo.lock].each { |path| current_paths << path if File.file?(File.join(root, path)) }
      frozen_paths = manifest['files'].map { |file| file['path'] }.sort
      errors << 'file_set_mismatch' unless baseline_paths == frozen_paths && (!workspace || current_paths.sort == frozen_paths)
      requested = manifest['files'].map { |file| baseline_objects[file['path']] }.compact
      blobs = git_blobs(root, requested)
      baseline_bytes = {}
      manifest['files'].each do |file|
        object = baseline_objects[file['path']]
        unless object
          errors << "baseline_file_missing path=#{file['path']}"
          next
        end
        bytes = blobs.fetch(object)
        baseline_bytes[file['path']] = bytes
        errors << "file_sha_mismatch origin=baseline path=#{file['path']}" unless Digest::SHA256.hexdigest(bytes) == file['sha256']
      end
      baseline_views = {}
      manifest['evidence'].each do |entry|
        bytes = baseline_bytes[entry['path']]
        unless bytes
          errors << "evidence_file_missing id=#{entry['id']}"
          next
        end
        begin
          view = baseline_views[entry['path']] ||= RustEvidence.view(bytes)
          located = view.locate(entry['symbol'], entry['kind'])
          errors << "symbol_sha_mismatch origin=baseline id=#{entry['id']}" unless located['symbol_sha256'] == entry['symbol_sha256']
          errors << "symbol_lines_mismatch origin=baseline id=#{entry['id']}" unless %w[start_line end_line].all? { |field| located[field] == entry[field] }
        rescue RustEvidence::Invalid => error
          errors << "#{error.message} origin=baseline id=#{entry['id']}"
        end
      end
      entry = manifest['evidence'].find { |item| item['id'] == catalog['enum_evidence_id'] }
      if entry && entry['kind'] == 'rust_enum' && baseline_bytes[entry['path']]
        begin
          view = baseline_views[entry['path']] ||= RustEvidence.view(baseline_bytes[entry['path']])
          actual = RustEvidence.enum_variants(view.locate(entry['symbol'], entry['kind']))
          errors << 'enum_coverage_mismatch origin=baseline' unless actual.sort == catalog['kinds'].map { |kind| kind['kind'] }.sort && actual.uniq == actual
        rescue RustEvidence::Invalid => error
          errors << "#{error.message} origin=baseline id=#{entry['id']}"
        end
      else
        errors << 'enum_evidence_missing origin=baseline'
      end
      errors
    rescue GitBatchInvalid => error
      errors + [error.message]
    end
  end
end

require_relative 'current_audit'
