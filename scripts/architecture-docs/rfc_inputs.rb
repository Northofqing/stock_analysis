# frozen_string_literal: true

require 'digest'
require 'json'
require 'pathname'

module ArchitectureDocs
  module RfcInputs
    MANIFEST_PATH = 'docs/push-system/rfc-input-manifest.v1.json'
    TOP_FIELDS = %w[schema_version status source_workspace captured_date inputs].freeze
    INPUT_FIELDS = %w[id path bytes sha256 role authority conflicts].freeze

    module_function

    def validate(root)
      root = File.expand_path(root)
      return ["root_missing path=#{root}"] unless File.exist?(root)
      return ["root_invalid path=#{root}"] unless File.directory?(root)

      root = File.realpath(root)
      manifest_bytes, problem = read_bytes(root, MANIFEST_PATH)
      return ["manifest_#{problem} path=#{MANIFEST_PATH}"] if problem

      manifest = JSON.parse(manifest_bytes)
      errors = manifest_errors(manifest)
      return errors unless errors.empty?

      %w[id path].each do |field|
        manifest['inputs'].group_by { |input| input[field] }.each do |value, entries|
          errors << "duplicate_input_#{field} #{field}=#{value}" if entries.length > 1
        end
      end
      manifest.fetch('inputs').each do |input|
        bytes, problem = read_bytes(root, input['path'])
        if problem
          errors << "input_#{problem} id=#{input['id']} path=#{input['path']}"
          next
        end
        unless bytes.bytesize == input['bytes']
          errors << "input_bytes_mismatch id=#{input['id']} path=#{input['path']} expected=#{input['bytes']} actual=#{bytes.bytesize}"
        end
        actual = Digest::SHA256.hexdigest(bytes)
        unless actual == input['sha256']
          errors << "input_sha_mismatch id=#{input['id']} path=#{input['path']} expected=#{input['sha256']} actual=#{actual}"
        end
      end
      errors
    rescue JSON::ParserError
      ["manifest_json_invalid path=#{MANIFEST_PATH}"]
    end

    def manifest_errors(manifest)
      return ['manifest_structure_invalid'] unless manifest.is_a?(Hash)

      errors = []
      errors << 'manifest_fields_invalid' unless manifest.keys.sort == TOP_FIELDS.sort
      [
        ['schema_version', 1, 'schema'],
        ['status', 'PROVISIONAL', 'status'],
        ['source_workspace', 'root-worktree-snapshot', 'source_workspace'],
        ['captured_date', '2026-09-06', 'captured_date']
      ].each do |field, expected, code|
        unless manifest[field].eql?(expected)
          errors << "manifest_#{code}_invalid expected=#{expected.inspect} actual=#{manifest[field].inspect}"
        end
      end

      inputs = manifest['inputs']
      unless inputs.is_a?(Array) && !inputs.empty?
        return errors + ['manifest_inputs_invalid']
      end

      inputs.each_with_index do |input, index|
        unless input.is_a?(Hash)
          errors << "input_structure_invalid index=#{index}"
          next
        end
        errors << "input_fields_invalid index=#{index}" unless input.keys.sort == INPUT_FIELDS.sort
        strings = %w[id path role authority].all? { |field| nonempty_string?(input[field]) }
        size = input['bytes'].is_a?(Integer) && input['bytes'] >= 0
        sha = input['sha256'].is_a?(String) && input['sha256'].match?(/\A[0-9a-f]{64}\z/)
        conflicts = input['conflicts'].is_a?(Array) && input['conflicts'].all? { |value| nonempty_string?(value) }
        errors << "input_field_invalid index=#{index}" unless strings && size && sha && conflicts
      end
      errors
    end

    def nonempty_string?(value)
      value.is_a?(String) && !value.strip.empty?
    end

    def read_bytes(root, relative_path)
      full_path, problem = checked_path(root, relative_path)
      return [nil, problem] if problem

      [File.binread(full_path), nil]
    rescue SystemCallError
      [nil, 'unreadable']
    end

    # Check every component before resolving or reading: in-root and dangling
    # symlinks are rejected too, not merely links that escape the repository.
    def checked_path(root, relative_path)
      if relative_path.include?("\0") || Pathname.new(relative_path).absolute?
        return [nil, 'path_invalid']
      end
      components = relative_path.split('/', -1)
      return [nil, 'path_invalid'] if components.any? { |part| ['', '.', '..'].include?(part) }

      full_path = root
      components.each do |part|
        full_path = File.join(full_path, part)
        return [nil, 'path_invalid'] if File.lstat(full_path).symlink?
      end
      real_path = File.realpath(full_path)
      return [nil, 'path_invalid'] unless real_path.start_with?(root + File::SEPARATOR)
      return [nil, 'not_regular'] unless File.file?(real_path)

      [real_path, nil]
    rescue Errno::ENOENT
      [nil, 'missing']
    rescue Errno::ENOTDIR, Errno::ELOOP
      [nil, 'path_invalid']
    end
  end
end
