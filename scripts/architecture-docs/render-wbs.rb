#!/usr/bin/env ruby
# frozen_string_literal: true

require 'optparse'
require 'tempfile'
require_relative 'wbs'

root = nil
modes = []
parser = OptionParser.new do |o|
  o.banner = 'Usage: render-wbs.rb --root ROOT (--check | --write)'
  o.on('--root ROOT') { |v| root = v }
  o.on('--check') { modes << :check }
  o.on('--write') { modes << :write }
end
begin
  parser.parse!(ARGV)
  raise OptionParser::InvalidOption, 'root and exactly one mode required' unless root && modes.length == 1 && ARGV.empty?
rescue OptionParser::ParseError => error
  warn error.message
  warn parser
  exit 2
end

errors = ArchitectureDocs::Wbs.validate(root, freshness: modes.first == :check)
unless errors.empty?
  puts errors
  exit 1
end
if modes.first == :check
  puts 'wbs_current'
  exit 0
end
begin
  root = File.realpath(root)
  path = File.join(root, ArchitectureDocs::Wbs::RFC)
  raise ArchitectureDocs::Wbs::Invalid, 'wbs_rfc_hardlink_invalid' unless File.stat(path).nlink == 1
  original = ArchitectureDocs::Wbs.read(root, ArchitectureDocs::Wbs::RFC)
  wbs = ArchitectureDocs::Wbs.parse(ArchitectureDocs::Wbs.read(root, ArchitectureDocs::Wbs::PATH))
  replacement = ArchitectureDocs::Wbs.replace(original, ArchitectureDocs::Wbs.render(wbs))
  if original != replacement
    Tempfile.create(['wbs-', '.md'], File.dirname(path)) do |file|
      file.binmode
      file.write(replacement)
      file.flush
      file.fsync
      File.chmod(File.stat(path).mode & 0777, file.path)
      File.rename(file.path, path)
    end
  end
  puts 'wbs_written'
rescue ArchitectureDocs::Wbs::Invalid => error
  puts error.message
  exit 1
rescue SystemCallError
  puts 'wbs_write_failed'
  exit 1
end
