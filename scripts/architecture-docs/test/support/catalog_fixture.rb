# frozen_string_literal: true

require 'digest'
require 'fileutils'
require 'json'
require 'open3'
require 'tmpdir'
require_relative '../../catalog'

module CatalogFixture
  def install_current(root, locate: true)
    catalog = JSON.parse(File.binread(File.join(root, ArchitectureDocs::Catalog::CATALOG_PATH)))
    manifest = JSON.parse(File.binread(File.join(root, ArchitectureDocs::Catalog::MANIFEST_PATH)))
    [catalog, manifest].each do |document|
      document['baseline_commit'] = git(root, 'rev-parse', 'HEAD')
      document['role'] = 'current-source-audit'
    end
    catalog['architecture'] = []
    manifest['files'].each do |file|
      file['sha256'] = Digest::SHA256.file(File.join(root, file['path'])).hexdigest
      file['architecture_ids'] = []
    end
    manifest['evidence'].each do |entry|
      if locate
        located = ArchitectureDocs::RustEvidence.locate(File.binread(File.join(root, entry['path'])), entry['symbol'], entry['kind'])
        %w[symbol_sha256 start_line end_line].each { |field| entry[field] = located[field] }
      end
      entry['audit_domains'] = ['business']
      entry['dependencies'] = []
    end
    json(root, 'docs/push-system/push-current-capability-catalog.v1.json', catalog)
    manifest['bindings'] = [ArchitectureDocs::Catalog::CATALOG_PATH, ArchitectureDocs::Catalog::MANIFEST_PATH,
                            'docs/push-system/push-current-capability-catalog.v1.json'].map do |path|
      { 'path' => path, 'sha256' => Digest::SHA256.file(File.join(root, path)).hexdigest }
    end
    json(root, 'docs/push-system/push-current-evidence-manifest.v1.json', manifest)
  end

  def bind_current(root)
    mutate(root, 'push-current-evidence-manifest.v1.json') do |manifest|
      manifest['bindings'].each { |entry| entry['sha256'] = Digest::SHA256.file(File.join(root, entry['path'])).hexdigest }
    end
  end

  def mutate(root, filename)
    path = 'docs/push-system/' + filename
    data = JSON.parse(File.binread(File.join(root, path)))
    yield data
    json(root, path, data)
  end

  def git(root, *args)
    out, err, result = Open3.capture3('git', '-C', root, *args)
    raise out + err unless result.success?

    out.strip
  end

  def json(root, path, object)
    File.binwrite(File.join(root, path), JSON.pretty_generate(object) + "\n")
  end

  def with_fixture(parent = nil)
    Dir.mktmpdir('push-catalog-', parent) do |root|
      FileUtils.mkdir_p(File.join(root, 'docs/push-system'))
      FileUtils.mkdir_p(File.join(root, 'src'))
      source = "pub enum PushKind {\n    One,\n}\n"
      File.binwrite(File.join(root, 'src/notify.rs'), source)
      decisions = "## Q1--Q55\n" + (1..55).map { |n| "| #{n} | yes |\n" }.join +
                  "## Q56--Q108\n" + (56..108).map { |n| "| #{n} | yes |\n" }.join
      File.binwrite(File.join(root, 'docs/decisions.md'), decisions)
      json(root, 'design-source-catalog.v1.json', {
        'schema_version' => 1, 'status' => 'PROVISIONAL', 'provenance' => 'user_workspace_snapshot',
        'approved_decisions' => { 'path' => 'docs/decisions.md', 'sha256' => Digest::SHA256.hexdigest(decisions), 'question_count' => 108 },
        'sources' => []
      })
      git(root, 'init', '-q')
      git(root, 'add', '.')
      git(root, '-c', 'user.name=Catalog Test', '-c', 'user.email=catalog@example.invalid', 'commit', '-qm', 'fixture baseline')
      baseline = git(root, 'rev-parse', 'HEAD')
      json(root, 'docs/push-system/push-evidence-manifest.v1.json', {
        'schema_version' => 1, 'status' => 'PROVISIONAL', 'baseline_commit' => baseline,
        'files' => [{ 'path' => 'src/notify.rs', 'sha256' => Digest::SHA256.hexdigest(source) }],
        'evidence' => [{ 'id' => 'enum', 'path' => 'src/notify.rs', 'symbol' => 'PushKind', 'kind' => 'rust_enum',
                         'symbol_sha256' => Digest::SHA256.hexdigest(source), 'start_line' => 1, 'end_line' => 3 }]
      })
      json(root, 'docs/push-system/push-capability-catalog.v1.json', {
        'schema_version' => 1, 'status' => 'PROVISIONAL', 'baseline_commit' => baseline,
        'scope' => '隔离代码基线；源码审计≠部署证明；NOT CHECKED：完整RFC/WBS/离线HTML/CI/运行时Foundation/部署/真实接收/工期。',
        'enum_evidence_id' => 'enum',
        'kinds' => [{ 'kind' => 'One', 'primary_phase' => '盘中', 'status' => 'INACTIVE',
                     'producer_ids' => [], 'evidence_ids' => ['enum'], 'note' => 'fixture 无生产 caller。' }],
        'producers' => [], 'migration_units' => [],
        'excluded_worktree_additions' => [
          { 'kind' => 'PaperBuy', 'reason' => '仅原混合工作树存在，本分支未移入' },
          { 'kind' => 'Watchdog', 'reason' => '仅原混合工作树存在，本分支未移入' }
        ]
      })
      install_current(root)
      yield root
    end
  end
end
