# frozen_string_literal: true

module ArchitectureDocs
  # Current facts preserve migration identity; their source and reference domains are independent.
  module CurrentAudit
    CATALOG_PATH = 'docs/push-system/push-current-capability-catalog.v1.json'
    MANIFEST_PATH = 'docs/push-system/push-current-evidence-manifest.v1.json'
    MARKDOWN_PATH = 'docs/push-system/push-current-capability-catalog.md'
    BINDINGS = [Catalog::CATALOG_PATH, Catalog::MANIFEST_PATH, CATALOG_PATH].freeze

    module_function

    def render(catalog, manifest)
      text = Catalog.render(catalog, manifest).sub('# 推送能力源审计目录', '# 当前源码审计目录')
      text += "\n## 当前源码审计与历史实施规范\n\n"
      text += "本目录只记录当前源码事实，保持 PROVISIONAL；历史实施规范继续使用原 catalog、manifest、RFC 和 WBS。kind、producer、Unit 身份和状态没有晋级。声明、测试和生产可达性分别判断。\n\n"
      manifest['bindings'].each { |entry| text += "- #{Catalog.markdown(entry['path'])}：#{entry['sha256']}\n" }
      text += "\n## 非迁移架构证据\n\n"
      catalog['architecture'].each do |group|
        text += "### #{Catalog.markdown(group['id'])}\n\n#{Catalog.markdown(group['boundary'])}\n\n"
        text += "声明证据：#{Catalog.markdown(group['evidence_ids'].join('、'))}。\n\n"
        text += "文件级 supporting（不等于符号或生产效果证明）：#{Catalog.markdown(group['supporting_files'].join('、'))}。\n\n"
      end
      text += "## 显式证据依赖\n\n"
      manifest['evidence'].each do |entry|
        entry['dependencies'].each do |dependency|
          text += "- #{Catalog.markdown(entry['id'])} → #{Catalog.markdown(dependency['evidence_id'])}：#{Catalog.markdown(dependency['description'])}。\n"
        end
      end
      text
    end

    def schema_errors(catalog, manifest)
      errors = []
      [catalog, manifest].each do |document|
        errors << 'current_role_invalid' unless document.is_a?(Hash) && document['role'] == 'current-source-audit'
      end
      Catalog.check_object(catalog, { 'architecture' => :objects }, 'current_catalog', errors)
      Catalog.check_object(manifest, { 'bindings' => :objects }, 'current_manifest', errors)
      return errors unless errors.empty?

      manifest['bindings'].each { |entry| Catalog.check_object(entry, { 'path' => :path, 'sha256' => :sha }, 'binding', errors) }
      manifest['files'].each { |entry| Catalog.check_object(entry, { 'architecture_ids' => :strings }, 'current_file', errors) }
      catalog['architecture'].each do |group|
        Catalog.check_object(group, { 'id' => :text, 'evidence_ids' => :references,
                                      'supporting_files' => :references, 'boundary' => :text }, 'architecture', errors)
      end
      manifest['evidence'].each do |entry|
        Catalog.check_object(entry, { 'audit_domains' => :references, 'dependencies' => :objects }, 'current_evidence', errors)
        domains = entry['audit_domains']
        errors << "evidence_domain_invalid id=#{entry['id']}" unless domains.is_a?(Array) && (domains - %w[business architecture]).empty?
        if entry['dependencies'].is_a?(Array)
          entry['dependencies'].each { |dependency| Catalog.check_object(dependency, { 'evidence_id' => :text, 'description' => :text }, 'dependency', errors) }
        end
      end
      errors
    end

    def reference_errors(root, catalog, manifest)
      errors = Catalog.reference_errors(catalog, manifest)
      architecture = catalog['architecture']
      ids = manifest['evidence'].map { |entry| entry['id'] }
      files = manifest['files'].map { |entry| entry['path'] }
      referenced = architecture.flat_map { |group| group['evidence_ids'] }.uniq
      # Only architecture-exclusive entries may be absent from business references.
      exclusive = manifest['evidence'].select { |entry| entry['audit_domains'] == ['architecture'] }.map { |entry| entry['id'] }
      errors.reject! { |error| exclusive.any? { |id| error == "evidence_unreferenced id=#{id}" } }
      (exclusive - referenced).each { |id| errors << "architecture_evidence_unreferenced id=#{id}" }
      (referenced - ids).each { |id| errors << "architecture_reference_missing id=#{id}" }
      manifest['evidence'].each do |entry|
        if entry['audit_domains'].include?('architecture') && !referenced.include?(entry['id'])
          errors << "architecture_evidence_unreferenced id=#{entry['id']}"
        end
        targets = entry['dependencies'].map { |dependency| dependency['evidence_id'] }
        errors << "duplicate_dependency id=#{entry['id']}" unless targets.uniq == targets
        (targets - ids).each { |id| errors << "evidence_reference_missing id=#{id}" }
      end
      business_lists = []
      catalog['kinds'].each { |kind| business_lists << ["kind:#{kind['kind']}", kind['evidence_ids']] }
      catalog['producers'].each do |producer|
        business_lists << ["producer:#{producer['id']}", producer['evidence_ids']]
        %w[trigger source authority policy].each { |field| business_lists << ["producer:#{producer['id']}:#{field}", producer[field]['evidence_ids']] }
      end
      business_lists.each { |context, list| errors.concat(closure_errors(manifest, list, 'business', context)) }
      architecture.each { |group| errors.concat(closure_errors(manifest, group['evidence_ids'], 'architecture', group['id'])) }
      SourceCatalog.duplicate_values(architecture, 'id').each { |id| errors << "duplicate_architecture id=#{id}" }
      manifest['files'].each do |file|
        owners = architecture.select { |group| group['supporting_files'].include?(file['path']) }.map { |group| group['id'] }.sort
        errors << "architecture_file_ownership_mismatch path=#{file['path']}" unless owners == file['architecture_ids'].sort
      end
      architecture.each do |group|
        required = manifest['evidence'].select { |entry| group['evidence_ids'].include?(entry['id']) }.map { |entry| entry['path'] }.uniq
        (required - group['supporting_files']).each { |path| errors << "architecture_file_incomplete id=#{group['id']} path=#{path}" }
        (group['supporting_files'] - files).each { |path| errors << "architecture_file_missing id=#{group['id']} path=#{path}" }
      end
      errors << 'current_bindings_mismatch' unless manifest['bindings'].map { |entry| entry['path'] }.sort == BINDINGS.sort
      manifest['bindings'].each do |entry|
        next unless BINDINGS.include?(entry['path'])
        path = SourceCatalog.safe_path(root, entry['path'])
        unless path && File.file?(path) && Digest::SHA256.file(path).hexdigest == entry['sha256']
          errors << "binding_sha_mismatch path=#{entry['path']}"
        end
      end
      historical_path = SourceCatalog.safe_path(root, Catalog::CATALOG_PATH)
      if historical_path && File.file?(historical_path)
        begin
          historical = JSON.parse(File.binread(historical_path))
          if historical.is_a?(Hash)
            { 'kinds' => %w[kind primary_phase status producer_ids],
              'producers' => %w[id kinds phase_epics occurrence_family completion_owner migration_unit_id],
              'migration_units' => %w[id producer_ids completion_owner occurrence_families phase_epics],
              'excluded_worktree_additions' => %w[kind reason] }.each do |collection, fields|
              expected = historical[collection]
              next unless expected.is_a?(Array) && expected.all? { |entry| entry.is_a?(Hash) }
              identity = proc { |entries| entries.map { |entry| fields.map { |field| entry[field] } }.sort_by { |row| row.first.to_s } }
              errors << "current_identity_mismatch collection=#{collection}" unless identity.call(expected) == identity.call(catalog[collection])
            end
          end
        rescue JSON::ParserError
          # Historical parsing reports independently; its byte binding still fails here.
        end
      end
      errors
    end

    def closure_errors(manifest, ids, domain, context)
      errors = []
      manifest['evidence'].each do |entry|
        next unless ids.include?(entry['id'])
        errors << "evidence_domain_mismatch context=#{context} id=#{entry['id']}" unless entry['audit_domains'].include?(domain)
        entry['dependencies'].each do |dependency|
          unless ids.include?(dependency['evidence_id'])
            errors << "evidence_dependency_missing context=#{context} id=#{entry['id']} dependency=#{dependency['evidence_id']}"
          end
        end
      end
      errors
    end
  end
end
