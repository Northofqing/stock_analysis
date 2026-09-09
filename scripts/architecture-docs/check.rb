#!/usr/bin/env ruby
# frozen_string_literal: true

require 'json'
require_relative 'catalog'
require_relative 'html_builder'
require_relative 'rfc_inputs'
require_relative 'rfc_spec'

USAGE = 'Usage: check.rb --draft|--check [--root ROOT]'
CATALOG_RELEASE_ERRORS = [
  "provisional path=#{ArchitectureDocs::Catalog::CATALOG_PATH}",
  "provisional path=#{ArchitectureDocs::Catalog::MANIFEST_PATH}",
  'worktree_dirty'
].freeze

def usage_error
  warn USAGE
  exit 2
end

if ARGV == ['--help'] || ARGV == ['-h']
  puts USAGE
  exit 0
end

mode = nil
root_argument = nil
arguments = ARGV.dup
until arguments.empty?
  argument = arguments.shift
  case argument
  when '--draft', '--check'
    usage_error if mode
    mode = argument == '--draft' ? :draft : :check
  when '--root'
    usage_error if root_argument || arguments.empty?
    root_argument = arguments.shift
  else
    usage_error
  end
end
usage_error unless mode

root = root_argument ? File.expand_path(root_argument) : File.expand_path('../..', __dir__)
begin
  root_stat = File.lstat(root)
  if root_stat.symlink?
    puts "root_path_invalid path=#{root}"
    exit 1
  end
  unless root_stat.directory?
    puts "root_invalid path=#{root}"
    exit 1
  end
  root = File.realpath(root)
rescue Errno::ENOENT
  puts "root_missing path=#{root}"
  exit 1
rescue SystemCallError, ArgumentError
  puts "root_invalid path=#{root}"
  exit 1
end

ENV['GIT_OPTIONAL_LOCKS'] = '0'
strict = mode == :check
errors = ArchitectureDocs::RfcInputs.validate(root)
catalog_errors = ArchitectureDocs::Catalog.validate(root, strict: strict)
errors.concat(catalog_errors)

catalog_content_errors = catalog_errors.reject { |error| CATALOG_RELEASE_ERRORS.include?(error) }
if catalog_content_errors.empty?
  pair = [ArchitectureDocs::Catalog::CATALOG_PATH, ArchitectureDocs::Catalog::MANIFEST_PATH].map do |path|
    checked, problem = ArchitectureDocs::RfcInputs.checked_path(root, path)
    if problem || File.stat(checked).nlink != 1
      errors << "push_path_invalid path=#{path}"
      nil
    else
      JSON.parse(File.binread(checked))
    end
  end
  if pair.all?
    markdown_path = 'docs/push-system/push-capability-catalog.md'
    checked, problem = ArchitectureDocs::RfcInputs.checked_path(root, markdown_path)
    if problem == 'missing'
      errors << 'markdown_missing'
    elsif problem || File.stat(checked).nlink != 1
      errors << 'markdown_path_invalid'
    elsif File.binread(checked) != ArchitectureDocs::Catalog.render(*pair).b
      errors << 'markdown_stale'
    end
  end
end

errors.concat(ArchitectureDocs::RfcSpec.validate(root, strict: strict))
begin
  ArchitectureDocs::HtmlBuilder.check(root, 'rfc')
rescue ArchitectureDocs::HtmlBuilder::Invalid => error
  errors << error.message
end

errors.uniq!
if errors.empty?
  puts 'architecture_docs_valid html_targets=rfc'
  exit 0
end

puts 'html_targets=rfc'
puts errors
exit 1
