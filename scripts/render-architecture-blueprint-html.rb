#!/usr/bin/env ruby
# frozen_string_literal: true

require 'rbconfig'

build = File.expand_path('architecture-docs/build.rb', __dir__)
exec(RbConfig.ruby, build, 'blueprint', *ARGV)
