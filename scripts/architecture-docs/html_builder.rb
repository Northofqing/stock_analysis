# frozen_string_literal: true

require 'base64'
require 'cgi'
require 'digest'
require 'erb'
require 'json'
require 'tempfile'
require_relative 'markdown_renderer'
require_relative 'rfc_inputs'

module ArchitectureDocs
  module HtmlBuilder
    class Invalid < StandardError; end

    SOURCE = 'docs/push-system/push-system-implementation-rfc.md'
    OUTPUT = 'docs/push-system/push-system-implementation-rfc.html'
    TEMPLATE = 'scripts/architecture-docs/templates/document.html.erb'
    ASSET_DIRECTORY = 'scripts/architecture-docs/assets'
    MERMAID_MANIFEST = File.join(ASSET_DIRECTORY, 'mermaid-manifest.v1.json')
    MERMAID_TOP_FIELDS = %w[files license name package_integrity schema_version source_url version].freeze
    MERMAID_FILE_FIELDS = %w[bytes package_path path sha256].freeze
    MERMAID_INTEGRITY = 'sha512-V6K3C8EBdEsPFZXSKMJe6ppQOENxuHARr9GvHX4hh47lAbhMRD9qf4oEK7LoaRQxULMa80/qt5gHO73aCleBBg=='.freeze
    MERMAID_FILES = {
      'mermaid.min.js' => {
        'package_path' => 'package/dist/mermaid.min.js',
        'bytes' => 3_572_661,
        'sha256' => '581ed7d74bd9048d0e3a91363927d72ef22942d7722546b27f7cc29e35390eb8'
      },
      'mermaid.LICENSE' => {
        'package_path' => 'package/LICENSE',
        'bytes' => 1_089,
        'sha256' => 'ec9fb67dcb25eccc416ed56e1aab819222c805a2a4bfe4cb19e7556bf2ffde80'
      }
    }.freeze

    module_function

    def check(root, target = 'rfc')
      build(root, target, check: true)
    end

    def build(root, target, check: false)
      raise Invalid, 'target_unknown' unless target == 'rfc'

      root = canonical_root(root)
      problems = RfcInputs.validate(root)
      raise Invalid, problems.first unless problems.empty?

      mermaid = load_mermaid(root)
      source = read_checked(root, SOURCE, 'html_source')
      template = read_checked(root, TEMPLATE, 'html_template')
      source_utf8 = source.dup.force_encoding(Encoding::UTF_8)
      raise Invalid, 'html_source_encoding_invalid' unless source_utf8.valid_encoding?
      template_utf8 = template.dup.force_encoding(Encoding::UTF_8)
      raise Invalid, 'html_template_encoding_invalid' unless template_utf8.valid_encoding?
      source_sha256 = Digest::SHA256.hexdigest(source)
      source_base64 = Base64.strict_encode64(source)
      template_sha256 = Digest::SHA256.hexdigest(template)
      implementation_sha256 = %w[build.rb html_builder.rb markdown_renderer.rb].each_with_object({}) do |name, hashes|
        hashes[name] = Digest::SHA256.file(File.join(__dir__, name)).hexdigest
      end
      mermaid_script = utf8_asset(mermaid.fetch(:files).fetch('mermaid.min.js'), 'mermaid.min.js')
      mermaid_license = utf8_asset(mermaid.fetch(:files).fetch('mermaid.LICENSE'), 'mermaid.LICENSE')
      mermaid_metadata = mermaid.fetch(:manifest)
      document = MarkdownRenderer.render_document(source_utf8)
      body_html = document.html
      navigation_html = render_navigation(document.headings)
      document_title = document.headings.empty? ? 'RFC' : document.headings.first.text
      build_metadata_json = metadata_json(
        source, source_sha256, template, template_sha256,
        implementation_sha256, mermaid_metadata, document.diagram_count
      )
      mermaid_license_html = CGI.escapeHTML(mermaid_license.force_encoding(Encoding::UTF_8))
      html = render_template(template_utf8, binding)
      output = File.join(root, OUTPUT)
      output_exists = validate_output(root, output)
      if output_exists
        return 'current' if File.binread(output) == html
        raise Invalid, 'html_stale target=rfc' if check
      elsif check
        raise Invalid, 'html_missing target=rfc'
      end

      atomic_write(output, html)
      'written'
    rescue Errno::ENOENT
      raise Invalid, 'html_input_missing'
    rescue SystemCallError
      raise Invalid, 'html_build_failed'
    end

    def atomic_write(path, bytes)
      Tempfile.create(['rfc-html-', '.tmp'], File.dirname(path)) do |file|
        file.binmode
        file.write(bytes)
        file.flush
        file.fsync
        File.rename(file.path, path)
      end
    end

    def canonical_root(root)
      path = File.expand_path(root)
      stat = File.lstat(path)
      raise Invalid, 'html_root_path_invalid' if stat.symlink?
      raise Invalid, 'html_root_invalid' unless stat.directory?

      File.realpath(path)
    rescue Errno::ENOENT
      raise Invalid, 'html_root_missing'
    rescue ArgumentError, Errno::ENOTDIR, Errno::ELOOP
      raise Invalid, 'html_root_invalid'
    end

    def render_template(template, context)
      ERB.new(template).result(context).b
    rescue StandardError, ScriptError
      raise Invalid, 'html_template_render_failed'
    end

    def validate_output(root, output)
      current = root
      File.dirname(OUTPUT).split('/').each do |part|
        current = File.join(current, part)
        stat = File.lstat(current)
        raise Invalid, 'html_output_parent_path_invalid' if stat.symlink? || !stat.directory?
      end
      parent = File.realpath(File.dirname(output))
      unless parent.start_with?(root + File::SEPARATOR)
        raise Invalid, 'html_output_parent_path_invalid'
      end

      begin
        stat = File.lstat(output)
      rescue Errno::ENOENT
        return false
      end
      raise Invalid, 'html_output_path_invalid' if stat.symlink?
      raise Invalid, 'html_output_not_regular' unless stat.file?
      raise Invalid, 'html_output_hardlink_invalid' unless stat.nlink == 1
      true
    rescue Errno::ENOENT
      raise Invalid, 'html_output_parent_missing'
    rescue Errno::ENOTDIR, Errno::ELOOP
      raise Invalid, 'html_output_parent_path_invalid'
    end

    def render_navigation(headings)
      items = headings.map do |heading|
        id = CGI.escapeHTML(heading.id)
        text = CGI.escapeHTML(heading.text)
        %(<li class="toc-level-#{heading.level}" data-heading-level="#{heading.level}"><a href="##{id}">#{text}</a></li>)
      end
      "<ol>#{items.join}</ol>"
    end

    def metadata_json(source, source_sha256, template, template_sha256, implementation_sha256, mermaid, diagram_count)
      metadata = {
        'schema_version' => 1,
        'status' => 'PROVISIONAL',
        'target' => 'rfc',
        'source' => {
          'path' => SOURCE,
          'bytes' => source.bytesize,
          'sha256' => source_sha256
        },
        'template' => {
          'path' => TEMPLATE,
          'bytes' => template.bytesize,
          'sha256' => template_sha256
        },
        'implementation_sha256' => implementation_sha256,
        'mermaid' => mermaid,
        'diagram_count' => diagram_count,
        'publication_boundary' => 'PROVISIONAL: not Implementation-Ready; strict publication is out of scope for this batch'
      }
      JSON.generate(metadata).gsub('&', '\\u0026').gsub('<', '\\u003c').gsub('>', '\\u003e')
    end

    def utf8_asset(bytes, name)
      text = bytes.dup.force_encoding(Encoding::UTF_8)
      raise Invalid, "mermaid_asset_encoding_invalid path=#{name}" unless text.valid_encoding?

      text
    end

    def load_mermaid(root)
      manifest_bytes = read_checked(root, MERMAID_MANIFEST, 'mermaid_manifest')
      manifest = JSON.parse(manifest_bytes)
      validate_mermaid_manifest(manifest)

      files = manifest.fetch('files').each_with_object({}) do |entry, loaded|
        bytes = read_checked(root, File.join(ASSET_DIRECTORY, entry.fetch('path')), 'mermaid_asset')
        unless bytes.bytesize == entry.fetch('bytes')
          raise Invalid, "mermaid_asset_bytes_mismatch path=#{entry.fetch('path')}"
        end
        unless Digest::SHA256.hexdigest(bytes) == entry.fetch('sha256')
          raise Invalid, "mermaid_asset_sha_mismatch path=#{entry.fetch('path')}"
        end
        loaded[entry.fetch('path')] = bytes
      end
      { manifest: manifest, files: files }
    rescue JSON::ParserError
      raise Invalid, 'mermaid_manifest_json_invalid'
    end

    def validate_mermaid_manifest(manifest)
      unless manifest.is_a?(Hash) && manifest.keys.sort == MERMAID_TOP_FIELDS.sort
        raise Invalid, 'mermaid_manifest_structure_invalid'
      end
      expected = {
        'schema_version' => 1,
        'name' => 'mermaid',
        'version' => '11.17.2',
        'license' => 'MIT',
        'source_url' => 'https://registry.npmjs.org/mermaid/-/mermaid-11.17.2.tgz',
        'package_integrity' => MERMAID_INTEGRITY
      }
      expected.each do |field, value|
        raise Invalid, "mermaid_manifest_#{field}_invalid" unless manifest[field].eql?(value)
      end
      files = manifest['files']
      unless files.is_a?(Array) && files.length == MERMAID_FILES.length
        raise Invalid, 'mermaid_manifest_files_invalid'
      end
      seen = {}
      files.each do |entry|
        unless entry.is_a?(Hash) && entry.keys.sort == MERMAID_FILE_FIELDS.sort
          raise Invalid, 'mermaid_manifest_file_structure_invalid'
        end
        name = entry['path']
        expected = MERMAID_FILES[name]
        valid = expected && entry['package_path'] == expected['package_path']
        valid &&= entry['bytes'].eql?(expected['bytes'])
        valid &&= entry['sha256'].eql?(expected['sha256'])
        valid &&= !seen[name]
        raise Invalid, 'mermaid_manifest_file_invalid' unless valid
        seen[name] = true
      end
      raise Invalid, 'mermaid_manifest_files_invalid' unless seen.keys.sort == MERMAID_FILES.keys.sort
    end

    def read_checked(root, relative_path, label)
      path, problem = RfcInputs.checked_path(root, relative_path)
      raise Invalid, "#{label}_#{problem}" if problem

      File.binread(path)
    rescue SystemCallError
      raise Invalid, "#{label}_unreadable"
    end
  end
end
