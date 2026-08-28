#!/usr/bin/env ruby
# frozen_string_literal: true

require "yaml"

root = File.expand_path("..", __dir__)
action = YAML.load_file(File.join(root, "action.yaml"))
YAML.load_file(File.join(root, "cache", "action.yaml"))
steps = action.fetch("runs").fetch("steps")
working_directory = "${{ inputs.working-directory }}"

%w[base history plan].each do |id|
  step = steps.find { |candidate| candidate["id"] == id }
  abort "action.yaml is missing the #{id} step" unless step

  actual = step["working-directory"]
  next if actual == working_directory

  abort "action.yaml step #{id} must run in #{working_directory.inspect}, got #{actual.inspect}"
end

puts "action metadata tests passed"
