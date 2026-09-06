#!/usr/bin/env ruby
# frozen_string_literal: true

require 'optparse'
require_relative 'rfc_spec'

root = nil
mode = nil
parser = OptionParser.new do |options|
  options.banner = 'Usage: check-rfc.rb --root ROOT --draft|--check'
  options.on('--root ROOT') { |value| root = value }
  options.on('--draft') do
    raise OptionParser::InvalidOption, 'duplicate mode' if mode
    mode = :draft
  end
  options.on('--check') do
    raise OptionParser::InvalidOption, 'duplicate mode' if mode
    mode = :check
  end
end
begin
  parser.parse!(ARGV)
  raise OptionParser::MissingArgument, '--root and mode' unless root && mode
  raise OptionParser::InvalidOption, ARGV.join(' ') unless ARGV.empty?
rescue OptionParser::ParseError => error
  warn error.message
  warn parser
  exit 2
end

errors = ArchitectureDocs::RfcSpec.validate(root, strict: mode == :check)
puts(errors.empty? ? 'rfc_spec_valid' : errors)
exit(errors.empty? ? 0 : 1)
