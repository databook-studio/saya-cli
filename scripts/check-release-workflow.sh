#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKFLOW="$ROOT_DIR/.github/workflows/release-candidate.yml"

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
  "actions/checkout@11d5960a326750d5838078e36cf38b85af677262" => "v4",
  "actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02" => "v4",
  "actions/download-artifact@d3f86a106a0bac45b974a628896c90dbdf5c8093" => "v4",
  "dtolnay/rust-toolchain@4cda84d5c5c54efe2404f9d843567869ab1699d4" => "stable",
}.each do |action, version|
  raise "missing pinned #{action}" unless text.include?("#{action} # #{version}")
end
puts "release workflow structure and Windows sidecar contract valid"
RUBY
