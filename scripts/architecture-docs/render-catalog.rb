#!/usr/bin/env ruby
# frozen_string_literal: true

require 'optparse'
require_relative 'catalog'

root = nil
modes = []
current = false
parser = OptionParser.new do |options|
  options.banner = 'Usage: render-catalog.rb --root ROOT [--current] (--check | --write)'
  options.on('--current') { current = true }
  options.on('--root ROOT') { |value| root = value }
  options.on('--check') { modes << :check }
  options.on('--write') { modes << :write }
end
begin
  parser.parse!(ARGV)
  raise OptionParser::MissingArgument, '--root' unless root
  raise OptionParser::InvalidOption, 'choose exactly one --check or --write' unless modes.length == 1
  raise OptionParser::InvalidOption, ARGV.join(' ') unless ARGV.empty?
rescue OptionParser::ParseError => error
  warn error.message
  warn parser
  exit 2
end

errors = ArchitectureDocs::Catalog.validate(root, strict: false)
puts 'NOT CHECKED：完整RFC/WBS/离线HTML/CI/运行时Foundation/部署/真实接收。'
begin
  root = File.realpath(File.expand_path(root))
  relative = current ? ArchitectureDocs::CurrentAudit::MARKDOWN_PATH : 'docs/push-system/push-capability-catalog.md'
  path = ArchitectureDocs::SourceCatalog.safe_path(root, relative)
  expected_path = File.join(root, relative)
  unless !File.symlink?(expected_path) && path == expected_path && File.realpath(File.dirname(path)) == File.dirname(path) &&
         (!File.exist?(path) || (File.file?(path) && File.stat(path).nlink == 1))
    errors << 'markdown_path_invalid'
  end
rescue SystemCallError
  errors << 'markdown_path_invalid'
end
unless errors.empty?
  puts errors
  exit 1
end
domain = current ? ArchitectureDocs::CurrentAudit : ArchitectureDocs::Catalog
catalog = JSON.parse(File.binread(File.join(root, domain::CATALOG_PATH)))
manifest = JSON.parse(File.binread(File.join(root, domain::MANIFEST_PATH)))
text = domain.render(catalog, manifest)
if modes.first == :write
  File.binwrite(path, text)
  puts 'markdown_written'
elsif !File.file?(path)
  puts 'markdown_missing'
  exit 1
elsif File.binread(path) != text.b
  puts 'markdown_stale'
  exit 1
else
  puts 'markdown_current'
end
