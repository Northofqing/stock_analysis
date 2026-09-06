# frozen_string_literal: true

require 'minitest/autorun'
require 'open3'
require 'rbconfig'
require 'tmpdir'
require 'fileutils'
require 'json'
require 'digest'

class WbsTest < Minitest::Test
  ROOT = File.expand_path('../../..', __dir__)
  CLI = File.join(ROOT, 'scripts/architecture-docs/render-wbs.rb')
  WBS = 'docs/push-system/push-system-wbs.v1.json'
  RFC = 'docs/push-system/push-system-implementation-rfc.md'
  CATALOG = 'docs/push-system/push-capability-catalog.v1.json'

  def test_public_check_accepts_complete_reconstructed_wbs
    out, err, status = Open3.capture3(RbConfig.ruby, CLI, '--root', ROOT, '--check')
    assert_equal 0, status.exitstatus, out + err
    assert_equal "wbs_current\n", out
    assert_empty err
  end

  MUTATIONS = {
    'top_fields' => ['wbs_fields_invalid', ->(w) { w['surprise'] = 1 }],
    'status' => ['wbs_status_invalid', ->(w) { w['status'] = 'IMPLEMENTED' }],
    'foundation_count' => ['wbs_foundation_set_invalid', ->(w) { w['foundation_work_packages'].pop }],
    'foundation_name' => ['wbs_foundation_name_invalid', ->(w) { w['foundation_work_packages'][0]['name'] = 'recovered' }],
    'lineage' => ['wbs_lineage_invalid', ->(w) { w['foundation_work_packages'][0].delete('lineage') }],
    'history' => ['wbs_assumptions_invalid', ->(w) { w['assumptions']['historical_usage'] = 'fit_98_142' }],
    'missing_unit' => ['wbs_unit_set_invalid', ->(w) { w['migration_units'].pop }],
    'duplicate_unit' => ['wbs_unit_set_invalid', ->(w) { w['migration_units'][-1] = w['migration_units'][0] }],
    'extra_unit' => ['wbs_unit_set_invalid', ->(w) { w['migration_units'] << w['migration_units'][0].merge('id'=>'MU-PaperBuy') }],
    'catalog_sha' => ['wbs_catalog_sha_mismatch', ->(w) { w['catalog_sha256'] = 'a' * 64 }],
    'unit_sha' => ['wbs_catalog_unit_sha_mismatch', ->(w) { w['migration_units'][0]['catalog_unit_sha256'] = 'b' * 64 }],
    'owner' => ['wbs_catalog_snapshot_mismatch', ->(w) { w['migration_units'][0]['completion_owner'] = 'different owner' }],
    'phase' => ['wbs_catalog_snapshot_mismatch', ->(w) { w['migration_units'][0]['phase_epics'] = ['盘中'] }],
    'producer' => ['wbs_catalog_snapshot_mismatch', ->(w) { w['migration_units'][0]['producer_ids'] = ['cli-chain'] }],
    'dependency' => ['wbs_dependency_invalid', ->(w) { w['migration_units'][0]['dependencies'] << 'W99' }],
    'unit_dependency' => ['wbs_dependency_invalid', ->(w) { w['migration_units'][0]['dependencies'] << 'MU-p01' }],
    'foundation_cycle' => ['wbs_dependency_cycle', ->(w) { w['foundation_work_packages'][0]['dependencies'] = ['W21'] }],
    'missing_core_dependency' => ['wbs_unit_dependencies_incomplete', ->(w) { w['migration_units'][0]['dependencies'].delete('W09') }],
    'zero_estimate' => ['wbs_estimate_invalid', ->(w) { w['migration_units'][0]['optimistic_hours'] = 0 }],
    'negative_estimate' => ['wbs_estimate_invalid', ->(w) { w['migration_units'][0]['most_likely_hours'] = -1 }],
    'reversed_estimate' => ['wbs_estimate_invalid', ->(w) { w['migration_units'][0]['optimistic_hours'] = 99 }],
    'wrong_pert' => ['wbs_pert_mismatch', ->(w) { w['migration_units'][0]['pert_hours'] = 9.51 }],
    'baseline' => ['wbs_engineering_totals_mismatch', ->(w) { w['engineering_totals']['baseline_hours'] = 142 }],
    'contingency_twice' => ['wbs_contingency_invalid', ->(w) { w['contingency']['applications'] = 2 }],
    'buffered_total' => ['wbs_engineering_totals_mismatch', ->(w) { w['engineering_totals']['buffered_hours'] += 165.8 }],
    'risk' => ['wbs_field_invalid', ->(w) { w['migration_units'][0].delete('risk_class') }],
    'external_wait' => ['wbs_field_invalid', ->(w) { w['migration_units'][0].delete('external_wait_business_days') }],
    'calendar' => ['wbs_field_invalid', ->(w) { w['migration_units'][0]['calendar_constraints'] = [] }],
    'gates' => ['wbs_gates_invalid', ->(w) { w['migration_units'][0]['acceptance_gates'] = {} }],
    'specific_gate' => ['wbs_gates_invalid', ->(w) { w['migration_units'][0]['acceptance_gates']['specific'] = [] }],
    'gate_producer_binding' => ['wbs_gate_binding_invalid', ->(w) { w['migration_units'][0]['acceptance_gates']['specific'][0]['producer_ids'] = ['cli-chain'] }],
    'gate_template_copy' => ['wbs_gate_duplicate', ->(w) { w['migration_units'][1]['acceptance_gates']['specific'][0]['statement'] = w['migration_units'][0]['acceptance_gates']['specific'][0]['statement'] }],
    'promotion_missing' => ['wbs_sessions_invalid', ->(w) { w['migration_units'][0].delete('promotion_sessions') }],
    'promotion_zero' => ['wbs_sessions_invalid', ->(w) { w['migration_units'][0]['promotion_sessions'] = 0 }],
    'observation' => ['wbs_sessions_invalid', ->(w) { w['migration_units'][0]['observation_sessions'] = 0 }],
    'shadow_session' => ['wbs_sessions_invalid', ->(w) { w['migration_units'].find { |u| u['id']=='MU-earnings-beat' }['promotion_sessions'] = 1 }],
    'rank_null' => ['wbs_rank_invalid', ->(w) { w['migration_units'][0]['approved_promotion_rank'] = 1 }],
    'rank_q44' => ['wbs_rank_invalid', ->(w) { w['migration_units'].find { |u| u['id']=='MU-intraday-market' }['approved_promotion_rank'] = 7 }],
    'rank_r03' => ['wbs_rank_invalid', ->(w) { w['migration_units'].find { |u| u['id']=='MU-review-r03-auto' }['approved_promotion_rank'] = 9 }],
    'replay_gate' => ['wbs_replay_gate_invalid', ->(w) { w['migration_units'].find { |u| u['id']=='MU-cli-replay-force' }['acceptance_gates']['specific'][0]['requirements'] = [] }],
    'startup_gate' => ['wbs_startup_gate_invalid', ->(w) { w['migration_units'].find { |u| u['id']=='MU-p01' }['acceptance_gates']['specific'].each { |g| g['requirements'] = [] } }],
    'inactive_gate' => ['wbs_inactive_gate_invalid', ->(w) { w['migration_units'].find { |u| u['id']=='MU-fixed-fill' }['acceptance_gates']['specific'][0]['requirements'] = [] }],
    'inactive_owner_change' => ['wbs_inactive_gate_invalid', ->(w) { w['migration_units'].find { |u| u['id']=='MU-fixed-fill' }['changes_physical_owner'] = true }],
    'trading_total' => ['wbs_trading_totals_mismatch', ->(w) { w['trading_totals']['promotion_sessions'] = 52 }],
    'calendar_total' => ['wbs_calendar_mismatch', ->(w) { w['calendar_scenarios']['weekdays_only_calendar_days'] = 306 }],
    'path' => ['wbs_critical_path_invalid', ->(w) { w['critical_path']['engineering']['dependency_longest_path'] = ['W01', 'W21'] }],
    'path_total' => ['wbs_critical_path_invalid', ->(w) { w['critical_path']['engineering']['dependency_path_hours'] = 1 }],
    'serial_order' => ['wbs_critical_path_invalid', ->(w) { w['critical_path']['engineering']['serial_order'].reverse! }],
    'first_batch_prefix' => ['wbs_critical_path_invalid', ->(w) { order = w['critical_path']['engineering']['serial_order']; order[21],order[-1] = order[-1],order[21] }],
    'wave_order' => ['wbs_trading_path_invalid', ->(w) { w['critical_path']['trading_rollout']['within_wave_order'] = 'lexical_order_approved' }],
    'wave_mapping' => ['wbs_trading_path_invalid', ->(w) { w['critical_path']['trading_rollout']['wave_groups'][0]['unit_ids'].pop }],
    'reference' => ['wbs_reference_invalid', ->(w) { w['migration_units'][0]['evidence_or_catalog_refs'] << 'unit:MU-Watchdog' }]
  }.freeze

  MUTATIONS.each do |name, pair|
    define_method("test_rejects_#{name}") do
      with_fixture do |root|
        mutate(root, &pair[1])
        out, err, status = cli(root, '--write')
        assert_equal 1, status.exitstatus, out + err
        assert_includes out, pair[0], out + err
        assert_empty err
      end
    end
  end

  def test_renderer_is_read_only_on_check_and_preserves_outside_bytes_and_write_idempotence
    with_fixture do |root|
      path = File.join(root, RFC)
      File.write(path, "prefix sentinel\n" + File.read(path).sub('828.99h', 'stale hours') + "\nsuffix sentinel")
      before = File.binread(path)
      stamp = File.stat(path).mtime
      out, err, status = cli(root, '--check')
      assert_equal 1, status.exitstatus, out + err
      assert_includes out, 'wbs_rfc_stale'
      assert_equal before, File.binread(path)
      assert_equal stamp, File.stat(path).mtime
      out, err, status = cli(root, '--write')
      assert_equal 0, status.exitstatus, out + err
      after = File.binread(path)
      assert_equal before.split('<!-- RFC-WBS-BEGIN -->').first, after.split('<!-- RFC-WBS-BEGIN -->').first
      assert_equal before.split('<!-- RFC-WBS-END -->').last, after.split('<!-- RFC-WBS-END -->').last
      stamp = File.stat(path).mtime
      assert_equal 0, cli(root, '--write').last.exitstatus
      assert_equal after, File.binread(path)
      assert_equal stamp, File.stat(path).mtime
      assert_equal 0, cli(root, '--check').last.exitstatus
    end
  end

  def test_missing_duplicate_and_reversed_markers_fail_both_modes_without_writes
    [->(t) { t.sub('<!-- RFC-WBS-BEGIN -->', '') },
     ->(t) { t + "\n<!-- RFC-WBS-END -->" },
     ->(t) { t.sub('<!-- RFC-WBS-BEGIN -->', '<!-- swap -->').sub('<!-- RFC-WBS-END -->', '<!-- RFC-WBS-BEGIN -->').sub('<!-- swap -->', '<!-- RFC-WBS-END -->') }].each do |change|
      with_fixture do |root|
        path = File.join(root, RFC)
        File.write(path, change.call(File.read(path)))
        before = File.binread(path)
        %w[--check --write].each do |mode|
          out, err, status = cli(root, mode)
          assert_equal 1, status.exitstatus, out + err
          assert_includes out, 'wbs_markers_invalid'
          assert_equal before, File.binread(path)
        end
      end
    end
  end

  def test_catalog_hash_recomputed_from_original_unit_not_sorted_snapshot
    with_fixture do |root|
      path = File.join(root, CATALOG)
      c = JSON.parse(File.read(path))
      c['migration_units'][0]['note'] += ' drift'
      File.write(path, JSON.pretty_generate(c))
      mutate(root) { |w| w['catalog_sha256'] = Digest::SHA256.file(path).hexdigest }
      out, err, status = cli(root, '--write')
      assert_equal 1, status.exitstatus, out + err
      assert_includes out, 'wbs_catalog_unit_sha_mismatch id=MU-announcement'
    end
  end

  def test_validator_returns_stable_sorted_errors_independent_of_hash_insertion_order
    require_relative '../wbs'
    with_fixture do |root|
      mutate(root) { |w| w['catalog_sha256'] = 'bad'; w['migration_units'][0]['completion_owner'] = 'drift' }
      first = ArchitectureDocs::Wbs.validate(root)
      refute_empty first
      mutate(root) { |w| w.replace(w.to_a.reverse.to_h) }
      assert_equal first.sort, first
      assert_equal first, ArchitectureDocs::Wbs.validate(root)
    end
  end

  def test_decimal_half_up_and_stored_line_totals_are_distinct_checks
    require_relative '../wbs'
    with_fixture do |root|
      # Worked decimal example: (1 + 4*1 + 1.03)/6 = 1.005 -> 1.01, never 1.00.
      mutate(root) do |w|
        w['foundation_work_packages'][0].merge!('optimistic_hours'=>1,'most_likely_hours'=>1,'pessimistic_hours'=>1.03,'pert_hours'=>1.0)
      end
      assert_includes ArchitectureDocs::Wbs.validate(root), 'wbs_pert_mismatch id=W01'
      mutate(root) { |w| w['foundation_work_packages'][0]['pert_hours'] = 1.01 }
      errors = ArchitectureDocs::Wbs.validate(root)
      refute_includes errors, 'wbs_pert_mismatch id=W01'
      assert_includes errors, 'wbs_engineering_totals_mismatch'
    end
  end

  def test_reordering_catalog_object_keys_preserves_unit_canonical_hash_but_array_order_does_not
    with_fixture do |root|
      path = File.join(root, CATALOG)
      c = JSON.parse(File.read(path))
      c['migration_units'].each { |u| u.replace(u.to_a.reverse.to_h) }
      File.write(path, JSON.pretty_generate(c))
      mutate(root) { |w| w['catalog_sha256'] = Digest::SHA256.file(path).hexdigest }
      assert_equal 0, cli(root, '--write').last.exitstatus
      c['migration_units'].find { |u| u['id']=='MU-p01' }['producer_ids'].reverse!
      File.write(path, JSON.pretty_generate(c))
      mutate(root) { |w| w['catalog_sha256'] = Digest::SHA256.file(path).hexdigest }
      out, err, status = cli(root, '--write')
      assert_equal 1, status.exitstatus, out + err
      assert_includes out, 'wbs_catalog_unit_sha_mismatch id=MU-p01'
    end
  end

  def test_invalid_shapes_fail_without_exception_or_writes
    [->(w) { w['migration_units'][0]['id'] = 3 },
     ->(w) { w['migration_units'][0]['producer_ids'] = [3] },
     ->(w) { w['migration_units'][0]['acceptance_gates'] = [] },
     ->(w) { w['critical_path'] = nil },
     ->(w) { w['foundation_work_packages'] = [nil] }].each do |change|
      with_fixture do |root|
        mutate(root, &change)
        before = File.binread(File.join(root, RFC))
        out, err, status = cli(root, '--write')
        assert_equal 1, status.exitstatus, out + err
        assert_match(/wbs_/, out)
        assert_empty err
        assert_equal before, File.binread(File.join(root, RFC))
      end
    end
  end

  def test_duplicate_json_keys_are_rejected_without_silently_changing_authority
    with_fixture do |root|
      path = File.join(root, WBS)
      File.write(path, File.read(path).sub('"schema_version": 1,', '"schema_version": 2, "schema_version": 1,'))
      out, err, status = cli(root, '--write')
      assert_equal 1, status.exitstatus, out + err
      assert_includes out, 'wbs_duplicate_json_key'
      assert_empty err
    end
  end

  def test_links_and_missing_files_fail_without_modifying_the_target
    [WBS,RFC,CATALOG].each do |relative|
      with_fixture do |root|
        path = File.join(root, relative)
        saved = path+'.saved'
        File.rename(path,saved)
        File.symlink(saved,path)
        before = File.binread(saved)
        out, err, status = cli(root, '--write')
        assert_equal 1, status.exitstatus, out+err
        assert_includes out, 'wbs_document_path_invalid'
        assert_equal before, File.binread(saved)
      end
    end
    with_fixture do |root|
      File.link(File.join(root,RFC),File.join(root,'hardlink'))
      out, err, status = cli(root, '--write')
      assert_equal 1, status.exitstatus, out+err
      assert_includes out, 'wbs_rfc_hardlink_invalid'
      File.unlink(File.join(root,WBS))
      assert_includes cli(root,'--check').first, 'wbs_document_missing'
    end
  end

  def test_cli_requires_exactly_one_explicit_mode
    [[], ['--root',ROOT], ['--root',ROOT,'--check','--write'],
     ['--root',ROOT,'--check','--check'], ['--root',ROOT,'--write','extra']].each do |args|
      out, err, status = Open3.capture3(RbConfig.ruby,CLI,*args)
      assert_equal 2,status.exitstatus,out+err
      assert_empty out
      assert_includes err,'Usage: render-wbs.rb'
    end
  end

  private

  def with_fixture
    Dir.mktmpdir('wbs-test') do |root|
      [WBS, RFC, CATALOG].each do |path|
        FileUtils.mkdir_p(File.dirname(File.join(root, path)))
        FileUtils.copy_file(File.join(ROOT, path), File.join(root, path))
      end
      yield root
    end
  end

  def mutate(root)
    path = File.join(root, WBS)
    w = JSON.parse(File.read(path))
    yield w
    File.write(path, JSON.pretty_generate(w) + "\n")
  end

  def cli(root, mode)
    Open3.capture3(RbConfig.ruby, CLI, '--root', root, mode)
  end
end
