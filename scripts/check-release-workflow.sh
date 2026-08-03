#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKFLOW="$ROOT_DIR/.github/workflows/release-candidate.yml"
CI_WORKFLOW="$ROOT_DIR/.github/workflows/ci.yml"

ruby -ryaml - "$WORKFLOW" <<'RUBY'
path = ARGV.fetch(0)
workflow = YAML.load_file(path)
trigger = workflow["on"] || workflow[true]
raise "workflow_dispatch missing" unless trigger.is_a?(Hash) && trigger.key?("workflow_dispatch")
publish = trigger.dig("workflow_dispatch", "inputs", "publish")
raise "publish input is not false by default" unless publish["default"] == false
raise "publish input is not boolean" unless publish["type"] == "boolean"
jobs = workflow.fetch("jobs")
raise "top-level write permission" unless workflow.dig("permissions", "contents") == "read"
raise "publish write permission missing" unless jobs.dig("publish", "permissions", "contents") == "write"
raise "publish gate missing" unless jobs["publish"]["if"].include?("inputs.publish == true")
raise "publish dependencies missing" unless jobs["publish"]["needs"].sort == %w[build checksums]
text = File.read(path)
release = 'gh release create "v${{ steps.version.outputs.version }}" release-assets/* --target "${{ github.sha }}" --generate-notes'
raise "release target is not github.sha" unless text.include?(release)
raise "reserved PowerShell $host assignment" if text.match?(/^\s*\$host\s*=/i)
%w[$targetTriple System.IO.File] .each { |value| raise "missing #{value}" unless text.include?(value) }
%w[WriteAllText UTF8Encoding] .each { |value| raise "missing #{value}" unless text.include?(value) }
raise "missing UTF-8 without BOM" unless text.include?("UTF8Encoding]::new($false)")
raise "missing LF sidecar newline" unless text.include?("$stage.zip`n")
raise "offline release command" if text.include?("--offline")
{
  "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1" => "v7",
  "actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02" => "v4",
  "actions/download-artifact@d3f86a106a0bac45b974a628896c90dbdf5c8093" => "v4",
  "dtolnay/rust-toolchain@4cda84d5c5c54efe2404f9d843567869ab1699d4" => "stable",
}.each do |action, version|
  raise "missing pinned #{action}" unless text.include?("#{action} # #{version}")
end
puts "release workflow structure and Windows sidecar contract valid"
RUBY

ruby -ryaml - "$CI_WORKFLOW" "$ROOT_DIR/.cargo/config.toml" <<'RUBY'
workflow_path, cargo_config_path = ARGV
workflow = YAML.load_file(workflow_path)
text = File.read(workflow_path)
checkout = "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7"
toolchain = "dtolnay/rust-toolchain@4cda84d5c5c54efe2404f9d843567869ab1699d4 # stable"
raise "CI does not pin checkout v7" unless text.scan(checkout).length == 3
raise "CI does not pin rust-toolchain" unless text.scan(toolchain).length == 3
raise "CI matrix does not include Windows" unless workflow.dig("jobs", "verify", "strategy", "matrix", "os").include?("windows-latest")
config = File.read(cargo_config_path)
flag = 'CXXFLAGS_x86_64_pc_windows_msvc = { value = "/std:c++17", force = true }'
raise "MSVC C++17 flag missing or not forced" unless config.include?(flag)
puts "CI action pins and MSVC DuckDB C++17 contract valid"
RUBY
