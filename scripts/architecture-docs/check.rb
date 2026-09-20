#!/usr/bin/env ruby
# frozen_string_literal: true

require 'json'
require_relative 'catalog'
require_relative 'html_builder'
require_relative 'rfc_inputs'
require_relative 'rfc_spec'

USAGE = 'Usage: check.rb --draft|--check [--root ROOT]'

def usage_error
  warn USAGE
  exit 2
end

if ARGV == ['--help']
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
    usage_error if root_argument || arguments.empty? || arguments.first.start_with?('-')
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

[[ArchitectureDocs::Catalog, 'historical', 'docs/push-system/push-capability-catalog.md'],
 [ArchitectureDocs::CurrentAudit, 'current', ArchitectureDocs::CurrentAudit::MARKDOWN_PATH]].each do |domain, origin, markdown_path|
  content_errors = catalog_errors.select { |error| error.end_with?("pair=#{origin}") && !error.start_with?('provisional ') }
  next unless content_errors.empty?

  pair = [domain::CATALOG_PATH, domain::MANIFEST_PATH].map do |path|
    checked, problem = ArchitectureDocs::RfcInputs.checked_path(root, path)
    if problem || File.stat(checked).nlink != 1
      errors << "push_path_invalid path=#{path} pair=#{origin}"
      nil
    else
      JSON.parse(File.binread(checked))
    end
  end
  if pair.all?
    checked, problem = ArchitectureDocs::RfcInputs.checked_path(root, markdown_path)
    if problem == 'missing'
      errors << "markdown_missing pair=#{origin}"
    elsif problem || File.stat(checked).nlink != 1
      errors << "markdown_path_invalid pair=#{origin}"
    elsif File.binread(checked) != domain.render(*pair).b
      errors << "markdown_stale pair=#{origin}"
    end
  end
end

errors.concat(ArchitectureDocs::RfcSpec.validate(root, strict: strict))
%w[rfc blueprint].each do |target|
  begin
    ArchitectureDocs::HtmlBuilder.check(root, target)
  rescue ArchitectureDocs::HtmlBuilder::Invalid => error
    errors << error.message
  end
end

errors.uniq!
if errors.empty?
  puts 'architecture_docs_valid html_targets=rfc,blueprint'
  exit 0
end

puts 'html_targets=rfc,blueprint'
puts errors
exit 1
