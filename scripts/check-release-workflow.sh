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
resource_env = jobs.dig("build", "env")
raise "release build jobs are not serialized" unless resource_env["CARGO_BUILD_JOBS"] == "1"
raise "release test debug info is enabled" unless resource_env["CARGO_PROFILE_TEST_DEBUG"] == "0"
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
%w[CXXFLAGS _SECURE_SCL /std:c++17 /EHsc].each do |flag|
  raise "release workflow overrides dependency-owned C++ flags" if text.include?(flag)
end
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

ruby -ryaml - "$CI_WORKFLOW" "$ROOT_DIR/Cargo.toml" "$ROOT_DIR/Cargo.lock" "$ROOT_DIR" <<'RUBY'
workflow_path, manifest_path, lock_path, root_dir = ARGV
workflow = YAML.load_file(workflow_path)
text = File.read(workflow_path)
checkout = "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7"
toolchain = "dtolnay/rust-toolchain@4cda84d5c5c54efe2404f9d843567869ab1699d4 # stable"
raise "CI does not pin checkout v7" unless text.scan(checkout).length == 4
raise "CI does not pin rust-toolchain" unless text.scan(toolchain).length == 3
raise "CI matrix does not include Windows" unless workflow.dig("jobs", "verify", "strategy", "matrix", "os").include?("windows-latest")
resource_env = workflow.dig("jobs", "verify", "env")
raise "CI build jobs are not serialized" unless resource_env["CARGO_BUILD_JOBS"] == "1"
raise "CI test debug info is enabled" unless resource_env["CARGO_PROFILE_TEST_DEBUG"] == "0"
manifest = File.read(manifest_path)
workspace = manifest[/^\[workspace\]\n(.*?)(?=^\[|\z)/m, 1] or raise "workspace table missing"
package = manifest[/^\[workspace\.package\]\n(.*?)(?=^\[|\z)/m, 1] or raise "workspace package table missing"
raise "workspace does not use resolver 3" unless workspace.match?(/^resolver = "3"$/)
raise "workspace MSRV is not Rust 1.88" unless package.match?(/^rust-version = "1\.88"$/)
clippy = File.read(File.join(root_dir, "clippy.toml"))
raise "Clippy MSRV is not Rust 1.88" unless clippy.match?(/^msrv = "1\.88"$/)
member_block = workspace[/members\s*=\s*\[(.*?)\]/m, 1] or raise "workspace members missing"
members = member_block.scan(/"([^"]+)"/).flatten
raise "expected six workspace members" unless members.length == 6
manifests = members.map { |member| File.join(root_dir, member, "Cargo.toml") }
raise "workspace member manifest missing" unless manifests.all? { |path| File.file?(path) }
manifests.each do |path|
  member_package = File.read(path)[/^\[package\]\n(.*?)(?=^\[|\z)/m, 1]
  raise "#{path} does not inherit the workspace MSRV" unless member_package&.match?(/^rust-version\.workspace = true$/)
end
msrv = workflow.dig("jobs", "msrv") or raise "MSRV job missing"
raise "MSRV job is not Ubuntu" unless msrv["runs-on"] == "ubuntu-latest"
raise "MSRV build is not serialized" unless msrv.dig("env", "CARGO_BUILD_JOBS") == "1"
raise "MSRV debug info is enabled" unless msrv.dig("env", "CARGO_PROFILE_DEV_DEBUG") == "0"
msrv_action = msrv.fetch("steps").find { |step| step["uses"]&.include?("dtolnay/rust-toolchain@") }
raise "MSRV toolchain is not exactly 1.88.0" unless msrv_action&.dig("with", "toolchain") == "1.88.0"
raise "MSRV action is not pinned" unless msrv_action["uses"].end_with?("4cda84d5c5c54efe2404f9d843567869ab1699d4")
commands = msrv.fetch("steps").map { |step| step["run"] }.compact
raise "MSRV workspace check missing" unless commands == ["cargo check --workspace --locked"]
pin = 'duckdb = { version = "=1.10504.0", features = ["bundled", "chrono", "serde_json", "uuid"] }'
raise "workspace DuckDB pin or bundled features changed" unless manifest.include?(pin)
locked = File.read(lock_path).scan(/\[\[package\]\]\nname = "(duckdb|libduckdb-sys)"\nversion = "([^"]+)"/)
expected = [["duckdb", "1.10504.0"], ["libduckdb-sys", "1.10504.0"]]
raise "DuckDB crates are not locked as a matched pair" unless locked.sort == expected.sort
config_paths = %w[.cargo/config .cargo/config.toml].map { |path| File.join(root_dir, path) }.select { |path| File.file?(path) }
cpp_inputs = [[workflow_path, text]] + config_paths.map { |path| [path, File.read(path)] }
cpp_inputs.each do |path, content|
  %w[CXXFLAGS _SECURE_SCL /std:c++17 /EHsc].each do |flag|
    raise "#{path} overrides dependency-owned C++ flags" if content.include?(flag)
  end
end
puts "CI action pins, MSRV contract, resource limits, and bundled DuckDB dependency valid"
RUBY
