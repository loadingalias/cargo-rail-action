#!/usr/bin/env ruby
# frozen_string_literal: true

require "yaml"

root = File.expand_path("..", __dir__)
action = YAML.load_file(File.join(root, "action.yaml"))
cache_action = YAML.load_file(File.join(root, "cache", "action.yaml"))
steps = action.fetch("runs").fetch("steps")
working_directory = "${{ inputs.working-directory }}"

%w[base history plan].each do |id|
  step = steps.find { |candidate| candidate["id"] == id }
  abort "action.yaml is missing the #{id} step" unless step

  actual = step["working-directory"]
  next if actual == working_directory

  abort "action.yaml step #{id} must run in #{working_directory.inspect}, got #{actual.inspect}"
end

cache_inputs = cache_action.fetch("inputs")
abort "cache root-portability must default to physical" unless cache_inputs.dig("root-portability", "default") == "physical"
abort "cache strict-probe must default to false" unless cache_inputs.dig("strict-probe", "default") == "false"
cache_steps = cache_action.fetch("runs").fetch("steps")
setup_steps = cache_steps.select { |step| step["id"] == "setup" }
abort "cache action must retain one setup transaction" unless setup_steps.length == 1
setup_run = setup_steps.first.fetch("run")
abort "cache setup transaction omits root portability" unless setup_run.include?("--root-portability")
abort "cache setup transaction omits strict probe policy" unless setup_run.include?("--strict-probe")

puts "action metadata tests passed"
