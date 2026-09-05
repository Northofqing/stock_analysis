#!/usr/bin/env ruby
# frozen_string_literal: true

require 'optparse'
require_relative 'catalog'

root = nil
strict = true
parser = OptionParser.new do |options|
  options.banner = 'Usage: check-catalog.rb --root ROOT [--draft]'
  options.on('--root ROOT') { |value| root = value }
  options.on('--draft') { strict = false }
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

errors = ArchitectureDocs::Catalog.validate(root, strict: strict)
puts(errors.empty? ? 'push_catalog_valid (源码审计；PROVISIONAL 不表示 Ready)' : errors)
puts 'NOT CHECKED：完整RFC/WBS/离线HTML/CI/运行时Foundation/部署/真实接收。'
exit(errors.empty? ? 0 : 1)
