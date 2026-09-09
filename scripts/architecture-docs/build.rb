#!/usr/bin/env ruby
# frozen_string_literal: true

require 'optparse'
require_relative 'html_builder'

roots = []
drafts = []
checks = []
all_targets = []

parser = OptionParser.new do |options|
  options.banner = 'Usage: build.rb rfc [--root ROOT] [--check] [--draft]'
  options.on('--root ROOT') { |value| roots << value }
  options.on('--check') { checks << true }
  options.on('--draft') { drafts << true }
  options.on('--all') { all_targets << true }
  options.on_tail('-h', '--help') do
    puts options
    exit 0
  end
end

begin
  parser.parse!(ARGV)
  target = ARGV.shift
  valid_target = target == 'rfc' && all_targets.empty?
  valid_all = target.nil? && all_targets.length == 1
  valid_counts = roots.length <= 1 && checks.length <= 1 && drafts.length <= 1
  valid_counts &&= ARGV.empty? && (valid_target || valid_all)
  raise OptionParser::InvalidArgument, 'invalid or conflicting arguments' unless valid_counts
rescue OptionParser::ParseError => error
  warn error.message
  warn parser
  exit 2
end

root = roots.first || File.expand_path('../..', __dir__)
target = 'rfc'
label = all_targets.empty? ? 'target=rfc' : 'targets=rfc'

if drafts.empty?
  puts "html_status_provisional #{label}"
  exit 1
end

begin
  result = if checks.empty?
             ArchitectureDocs::HtmlBuilder.build(root, target)
           else
             ArchitectureDocs::HtmlBuilder.check(root, target)
           end
  puts "html_#{result} #{label} status=PROVISIONAL"
rescue ArchitectureDocs::HtmlBuilder::Invalid => error
  reason = error.message.sub(/\s+target=rfc\z/, '')
  puts "#{reason} #{label}"
  exit 1
end
