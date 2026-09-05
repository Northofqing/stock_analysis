#!/usr/bin/env ruby
# frozen_string_literal: true

require 'optparse'
require_relative 'source_catalog'

root = nil
parser = OptionParser.new do |options|
  options.banner = 'Usage: check-sources.rb --root ROOT'
  options.on('--root ROOT', 'repository root') { |value| root = value }
end

begin
  parser.parse!(ARGV)
  raise OptionParser::MissingArgument, '--root' unless root
  raise OptionParser::InvalidOption, ARGV.join(' ') unless ARGV.empty?
rescue OptionParser::ParseError => error
  warn error.message
  warn parser
  exit 2
end

errors = ArchitectureDocs::SourceCatalog.validate(root)
if errors.empty?
  puts 'source_catalog_valid'
  exit 0
end

puts errors
exit 1
