# frozen_string_literal: true

require 'fileutils'
require 'json'
require 'open3'
require 'rbconfig'
require 'tmpdir'

module DocumentCheckFixture
  SOURCE_ROOT = File.expand_path('../../../..', __dir__)
  BASELINE = '07781bf386aafdf202851ae928efee8920387058'

  module_function

  def with_fixture
    Dir.mktmpdir('document-check-') do |root|
      clone(fixture_template, root)
      yield root
    end
  end

  def fixture_template
    return @fixture_template if @fixture_template

    @fixture_template = Dir.mktmpdir('document-check-template-')
    at_exit { FileUtils.remove_entry(@fixture_template) if @fixture_template && File.exist?(@fixture_template) }
    clone(SOURCE_ROOT, @fixture_template, no_checkout: true)
    git(@fixture_template, 'checkout', '-q', '--detach', BASELINE)
    overlay_current_documents(@fixture_template)
    run_ruby(@fixture_template, 'scripts/architecture-docs/render-catalog.rb', '--root', @fixture_template, '--write')
    run_ruby(@fixture_template, 'scripts/architecture-docs/build.rb', 'rfc', '--root', @fixture_template, '--draft')
    git(@fixture_template, 'add', '-f', 'docs', 'scripts/architecture-docs',
        'design-source-catalog.v1.json', '.github/workflows/ci.yml')
    git(@fixture_template, '-c', 'user.name=Document Check Test', '-c', 'user.email=document-check@example.invalid',
        'commit', '-qm', 'historical code aligned document fixture')
    %w[docs/push-system/push-capability-catalog.md
       docs/push-system/push-system-implementation-rfc.html
       scripts/architecture-docs/assets/mermaid.min.js].each do |path|
      git(@fixture_template, 'ls-files', '--error-unmatch', path)
    end
    raise 'fixture template is dirty' unless git(@fixture_template, 'status', '--porcelain', '--untracked-files=all').empty?

    @fixture_template
  rescue StandardError
    FileUtils.remove_entry(@fixture_template) if @fixture_template && File.exist?(@fixture_template)
    @fixture_template = nil
    raise
  end

  def clone(source, root, no_checkout: false)
    arguments = ['git', 'clone', '--shared', '-q']
    arguments << '--no-checkout' if no_checkout
    arguments.concat([source, root])
    _out, err, status = Open3.capture3(*arguments)
    raise err unless status.success?
  end

  def overlay_current_documents(root)
    FileUtils.rm_rf(File.join(root, 'docs'))
    source_catalog = JSON.parse(File.binread(File.join(SOURCE_ROOT, 'design-source-catalog.v1.json')))
    input_manifest = JSON.parse(File.binread(File.join(SOURCE_ROOT, 'docs/push-system/rfc-input-manifest.v1.json')))
    document_paths = [
      'docs/push-system/push-capability-catalog.v1.json',
      'docs/push-system/push-evidence-manifest.v1.json',
      'docs/push-system/push-system-implementation-rfc.md',
      'docs/push-system/push-system-foundation.v1.sql',
      'docs/push-system/push-system-wbs.v1.json',
      'docs/push-system/rfc-input-manifest.v1.json',
      source_catalog.fetch('approved_decisions').fetch('path')
    ]
    document_paths.concat(source_catalog.fetch('sources').map { |source| source.fetch('path') })
    document_paths.concat(input_manifest.fetch('inputs').map { |input| input.fetch('path') })
    document_paths.uniq.each { |path| copy_path(root, path) }

    FileUtils.rm_rf(File.join(root, 'scripts/architecture-docs'))
    %w[build.rb catalog.rb check.rb html_builder.rb markdown_renderer.rb render-catalog.rb
       rfc_inputs.rb rfc_spec.rb rust_evidence.rb source_catalog.rb wbs.rb].each do |name|
      copy_path(root, File.join('scripts/architecture-docs', name))
    end
    %w[templates/document.html.erb assets/mermaid-manifest.v1.json assets/mermaid.min.js
       assets/mermaid.LICENSE].each do |path|
      copy_path(root, File.join('scripts/architecture-docs', path))
    end
    FileUtils.cp(File.join(SOURCE_ROOT, 'design-source-catalog.v1.json'), root)
    FileUtils.mkdir_p(File.join(root, '.github/workflows'))
    FileUtils.cp(File.join(SOURCE_ROOT, '.github/workflows/ci.yml'), File.join(root, '.github/workflows/ci.yml'))
  end

  def copy_path(root, relative)
    destination = File.join(root, relative)
    FileUtils.mkdir_p(File.dirname(destination))
    FileUtils.copy_file(File.join(SOURCE_ROOT, relative), destination)
  end

  def run_ruby(root, relative, *arguments)
    out, err, status = Open3.capture3(RbConfig.ruby, File.join(root, relative), *arguments)
    raise out + err unless status.success?

    out
  end

  def git(root, *arguments)
    out, err, status = Open3.capture3('git', '-C', root, *arguments)
    raise out + err unless status.success?

    out.strip
  end
end
