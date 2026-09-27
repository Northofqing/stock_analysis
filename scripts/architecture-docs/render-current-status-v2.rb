#!/usr/bin/env ruby
# frozen_string_literal: true

require 'optparse'
require_relative 'current_status_v2'

root = nil
mode = nil
parser = OptionParser.new do |options|
  options.banner = 'Usage: render-current-status-v2.rb --root ROOT --check|--write'
  options.on('--root ROOT') { |value| root = value }
  options.on('--check') { mode = mode.nil? ? :check : :invalid }
  options.on('--write') { mode = mode.nil? ? :write : :invalid }
end
begin
  parser.parse!(ARGV)
  raise OptionParser::MissingArgument, '--root' unless root
  raise OptionParser::InvalidOption, 'choose exactly one --check or --write' unless %i[check write].include?(mode)
  raise OptionParser::InvalidOption, ARGV.join(' ') unless ARGV.empty?
  root = File.realpath(File.expand_path(root))
  raise OptionParser::InvalidArgument, 'root must be a directory' unless File.directory?(root)
rescue OptionParser::ParseError, SystemCallError => error
  warn error.message
  warn parser
  exit 2
end

audit = ArchitectureDocs::CurrentStatusV2
document, errors = audit.load(root)
errors.concat(audit.validate(root, document)) if document
if errors.empty?
  begin
    current_v1 = JSON.parse(File.binread(File.join(root, audit::CURRENT_V1_PATH)))
    expected = audit.render(document, current_v1)
    target = File.join(root, audit::MARKDOWN_PATH)
    if mode == :write
      if File.exist?(target) && !audit.regular_file?(target)
        errors << 'v2_markdown_path_invalid'
      else
        File.binwrite(target, expected)
      end
    elsif !audit.regular_file?(target)
      errors << 'v2_markdown_missing_or_invalid'
    elsif File.binread(target) != expected.b
      errors << 'v2_markdown_stale'
    end
  rescue JSON::ParserError, SystemCallError, KeyError
    errors << 'v2_render_invalid'
  end
end

if errors.empty?
  puts "current_status_v2_valid mode=#{mode} source_commit=#{document.fetch('source_commit')}"
  exit 0
end
puts errors.uniq
exit 1
