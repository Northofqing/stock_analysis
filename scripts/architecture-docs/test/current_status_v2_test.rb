# frozen_string_literal: true

require 'json'
require 'minitest/autorun'
require_relative '../current_status_v2'

class CurrentStatusV2Test < Minitest::Test
  ROOT = File.expand_path('../../..', __dir__)
  AUDIT = ArchitectureDocs::CurrentStatusV2

  def document
    JSON.parse(File.binread(File.join(ROOT, AUDIT::JSON_PATH)))
  end

  def test_versioned_delta_validates_source_snapshot_and_renders_reviewable_statuses
    source = document
    assert_empty AUDIT.validate(ROOT, source)

    current_v1 = JSON.parse(File.binread(File.join(ROOT, AUDIT::CURRENT_V1_PATH)))
    rendered = AUDIT.render(source, current_v1)
    assert_equal File.binread(File.join(ROOT, AUDIT::MARKDOWN_PATH)), rendered.b
    assert_includes rendered, 'ACTIVE 37、INACTIVE 22、STARVED 4、OPT-IN 2'
    assert_includes rendered, '`account_metrics_complete_required=true`'
    assert_includes rendered, '`source_registered=false`'
  end

  def test_status_and_blocker_changes_cannot_silently_activate_a_kind
    source = document
    source['kinds'][0]['observed_status'] = 'STARVED'
    assert_includes AUDIT.validate(ROOT, source), 'v2_kind_fact_mismatch kind=IndustryChain field=observed_status'

    source = document
    source['kinds'][1]['facts']['source_registered'] = true
    assert_includes AUDIT.validate(ROOT, source), 'v2_kind_fact_mismatch kind=PostFixedPriceOrder field=facts'
  end

  def test_source_evidence_and_base_bytes_are_pinned
    source = document
    source['evidence'][0]['required_fragments'] = ['never_in_the_source']
    assert_includes AUDIT.validate(ROOT, source), 'v2_fragment_missing id=r03-account-phase'

    source = document
    source['historical_catalog_sha256'] = '0' * 64
    assert_includes AUDIT.validate(ROOT, source), 'v2_base_digest_mismatch field=historical_catalog_sha256'
  end
end
