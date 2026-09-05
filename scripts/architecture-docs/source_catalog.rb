# frozen_string_literal: true

require 'digest'
require 'json'
require 'pathname'

module ArchitectureDocs
  module SourceCatalog
    CATALOG_PATH = 'design-source-catalog.v1.json'

    module_function

    def validate(root)
      root = File.realpath(File.expand_path(root))
      catalog_path = safe_path(root, CATALOG_PATH)
      return ["catalog_path_invalid path=#{CATALOG_PATH}"] unless catalog_path

      catalog = JSON.parse(File.binread(catalog_path))
      errors = catalog_errors(catalog)
      return errors unless errors.empty?

      duplicate_values(catalog.fetch('sources', []), 'id').each do |id|
        errors << "duplicate_source_id id=#{id}"
      end
      duplicate_values(catalog.fetch('sources', []), 'path').each do |path|
        errors << "duplicate_source_path path=#{path}"
      end

      catalog.fetch('sources', []).each do |source|
        path = source['path']
        full_path = safe_path(root, path)
        unless full_path
          errors << "source_path_invalid path=#{path} id=#{source['id']}"
          next
        end
        unless File.file?(full_path)
          errors << "source_missing path=#{path} id=#{source['id']}"
          next
        end

        actual = Digest::SHA256.file(full_path).hexdigest
        expected = source['sha256']
        errors << "source_sha_mismatch path=#{path} id=#{source['id']} expected=#{expected} actual=#{actual}" unless actual == expected
      end

      decisions = catalog.fetch('approved_decisions')
      decision_path = safe_path(root, decisions.fetch('path'))
      if decision_path
        if File.file?(decision_path)
          decision_bytes = File.binread(decision_path)
          actual = Digest::SHA256.hexdigest(decision_bytes)
          expected = decisions.fetch('sha256')
          errors << "approved_decisions_sha_mismatch path=#{decisions['path']} expected=#{expected} actual=#{actual}" unless actual == expected
          errors.concat(question_errors(decision_bytes, decisions['path']))
        else
          errors << "approved_decisions_missing path=#{decisions['path']}"
        end
      else
        errors << "approved_decisions_path_invalid path=#{decisions['path']}"
      end

      errors
    rescue Errno::ENOENT => error
      ["catalog_missing path=#{CATALOG_PATH} detail=#{error.message}"]
    rescue JSON::ParserError => error
      ["catalog_json_invalid path=#{CATALOG_PATH} detail=#{error.message}"]
    rescue TypeError => error
      ["catalog_structure_invalid path=#{CATALOG_PATH} detail=#{error.message}"]
    end

    TOP_FIELDS = %w[schema_version status provenance approved_decisions sources].freeze
    DECISION_FIELDS = %w[path sha256 question_count].freeze
    SOURCE_FIELDS = %w[id path sha256 title self_version self_status ruling conflicts superseded_by].freeze

    def catalog_errors(catalog)
      return ['catalog_structure_invalid path=design-source-catalog.v1.json'] unless catalog.is_a?(Hash)

      errors = missing_fields(catalog, TOP_FIELDS).map { |field| "catalog_field_missing field=#{field}" }
      errors << "catalog_schema_invalid expected=1 actual=#{catalog['schema_version'].inspect}" unless catalog['schema_version'] == 1
      errors << "catalog_status_invalid expected=PROVISIONAL actual=#{catalog['status'].inspect}" unless catalog['status'] == 'PROVISIONAL'
      errors << "catalog_provenance_invalid expected=user_workspace_snapshot actual=#{catalog['provenance'].inspect}" unless catalog['provenance'] == 'user_workspace_snapshot'

      decisions = catalog['approved_decisions']
      if decisions.is_a?(Hash)
        errors.concat(missing_fields(decisions, DECISION_FIELDS).map { |field| "approved_decisions_field_missing field=#{field}" })
        errors << "approved_decisions_question_count_invalid expected=108 actual=#{decisions['question_count'].inspect}" unless decisions['question_count'] == 108
        unless nonempty_string?(decisions['path']) && sha256?(decisions['sha256'])
          errors << 'approved_decisions_field_invalid field=path_or_sha256'
        end
      elsif catalog.key?('approved_decisions')
        errors << 'approved_decisions_invalid'
      end

      sources = catalog['sources']
      if sources.is_a?(Array)
        sources.each_with_index do |source, index|
          unless source.is_a?(Hash)
            errors << "source_invalid index=#{index}"
            next
          end
          errors.concat(missing_fields(source, SOURCE_FIELDS).map { |field| "source_field_missing field=#{field} id=#{source['id']}" })
          valid_strings = %w[id path title ruling].all? { |field| nonempty_string?(source[field]) }
          valid_nullable = %w[self_version self_status].all? { |field| source[field].nil? || nonempty_string?(source[field]) }
          valid_arrays = %w[conflicts superseded_by].all? do |field|
            source[field].is_a?(Array) && source[field].all? { |value| nonempty_string?(value) }
          end
          errors << "source_field_invalid id=#{source['id']}" unless valid_strings && valid_nullable && valid_arrays && sha256?(source['sha256'])
        end
      elsif catalog.key?('sources')
        errors << 'catalog_sources_invalid'
      end
      errors
    end

    def missing_fields(object, fields)
      fields.reject { |field| object.key?(field) }
    end

    def nonempty_string?(value)
      value.is_a?(String) && !value.empty?
    end

    def sha256?(value)
      value.is_a?(String) && value.match?(/\A[0-9a-f]{64}\z/)
    end

    def duplicate_values(entries, key)
      entries.map { |entry| entry[key] }.group_by(&:itself).select { |_value, values| values.length > 1 }.keys
    end

    def safe_path(root, relative_path)
      return nil unless relative_path.is_a?(String)
      return nil if relative_path.include?("\0")

      path = Pathname.new(relative_path)
      return nil if path.absolute? || path.cleanpath.to_s == '..' || path.cleanpath.to_s.start_with?('../')

      full_path = File.expand_path(relative_path, root)
      existing = full_path
      existing = File.dirname(existing) until File.exist?(existing) || existing == File.dirname(existing)
      real_existing = File.realpath(existing)
      return nil unless real_existing == root || real_existing.start_with?(root + File::SEPARATOR)

      File.exist?(full_path) ? File.realpath(full_path) : full_path
    rescue Errno::ENOENT, Errno::ELOOP
      nil
    end

    def question_errors(bytes, path)
      text = bytes.force_encoding(Encoding::UTF_8)
      return ["approved_decisions_encoding_invalid path=#{path}"] unless text.valid_encoding?

      first = text[/^## Q1--Q55.*?(?=^## Q56--Q108)/m].to_s
      second = text[/^## Q56--Q108.*?(?=^## |\z)/m].to_s
      questions = (first + second).scan(/^\|\s*(\d+)\s*\|/).flatten.map(&:to_i)
      counts = questions.group_by(&:itself)
      errors = []
      (1..108).each do |number|
        count = counts.fetch(number, []).length
        errors << "approved_question_missing question=#{number}" if count.zero?
        errors << "approved_question_duplicate question=#{number} count=#{count}" if count > 1
      end
      errors
    end
  end
end
