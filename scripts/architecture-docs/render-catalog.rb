#!/usr/bin/env ruby
# frozen_string_literal: true

require 'optparse'
require_relative 'catalog'

root = nil
modes = []
parser = OptionParser.new do |options|
  options.banner = 'Usage: render-catalog.rb --root ROOT (--check | --write)'
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
unless errors.empty?
  puts errors
  exit 1
end
root = File.realpath(File.expand_path(root))
path = ArchitectureDocs::SourceCatalog.safe_path(root, 'docs/push-system/push-capability-catalog.md')
expected_path = File.join(root, 'docs/push-system/push-capability-catalog.md')
unless path == expected_path && File.realpath(File.dirname(path)) == File.dirname(path) &&
       (!File.exist?(path) || (File.file?(path) && File.stat(path).nlink == 1))
  puts 'markdown_path_invalid'
  exit 1
end
catalog = JSON.parse(File.binread(File.join(root, ArchitectureDocs::Catalog::CATALOG_PATH)))
manifest = JSON.parse(File.binread(File.join(root, ArchitectureDocs::Catalog::MANIFEST_PATH)))
text = ArchitectureDocs::Catalog.render(catalog, manifest)
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
