#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKFLOW="$ROOT_DIR/.github/workflows/release-candidate.yml"
CI_WORKFLOW="$ROOT_DIR/.github/workflows/ci.yml"

ruby -ryaml - "$WORKFLOW" <<'RUBY'
# Read as UTF-8 whatever the caller's locale: the workflows contain em dashes,
# and a US-ASCII default makes every regex match on their text raise.
Encoding.default_external = Encoding::UTF_8
Encoding.default_internal = Encoding::UTF_8
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
  "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a" => "v7.0.1",
  "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c" => "v8.0.1",
  "dtolnay/rust-toolchain@4cda84d5c5c54efe2404f9d843567869ab1699d4" => "stable",
}.each do |action, version|
  raise "missing pinned #{action}" unless text.include?("#{action} # #{version}")
end
puts "release workflow structure and Windows sidecar contract valid"
RUBY

ruby -ryaml - "$CI_WORKFLOW" "$ROOT_DIR/Cargo.toml" "$ROOT_DIR/Cargo.lock" "$ROOT_DIR" <<'RUBY'
# Read as UTF-8 whatever the caller's locale: the workflows contain em dashes,
# and a US-ASCII default makes every regex match on their text raise.
Encoding.default_external = Encoding::UTF_8
Encoding.default_internal = Encoding::UTF_8
workflow_path, manifest_path, lock_path, root_dir = ARGV
workflow = YAML.load_file(workflow_path)
text = File.read(workflow_path)
checkout = "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7"
toolchain = "dtolnay/rust-toolchain@4cda84d5c5c54efe2404f9d843567869ab1699d4 # stable"
# Assert every use is pinned rather than counting uses: the guard exists to stop
# an unpinned action reaching CI, and a magic number goes stale the moment a job
# is added — which is exactly how this check came to fail.
uses_checkout = text.scan(%r{actions/checkout@\S+}).length
raise "CI has an unpinned checkout" unless uses_checkout.positive? && text.scan(checkout).length == uses_checkout
uses_toolchain = text.scan(%r{dtolnay/rust-toolchain@\S+}).length
pinned_toolchain = text.scan(%r{dtolnay/rust-toolchain@4cda84d5c5c54efe2404f9d843567869ab1699d4}).length
raise "CI has an unpinned rust-toolchain" unless uses_toolchain.positive? && pinned_toolchain == uses_toolchain
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
pin = 'duckdb = { version = "=1.10505.0", features = ["bundled", "chrono", "serde_json", "uuid"] }'
raise "workspace DuckDB pin or bundled features changed" unless manifest.include?(pin)
locked = File.read(lock_path).scan(/\[\[package\]\]\nname = "(duckdb|libduckdb-sys)"\nversion = "([^"]+)"/)
expected = [["duckdb", "1.10505.0"], ["libduckdb-sys", "1.10505.0"]]
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

ruby -ryaml -ropen3 -rtmpdir - "$WORKFLOW" "$ROOT_DIR" <<'RUBY'
# Read as UTF-8 whatever the caller's locale: the workflows contain em dashes,
# and a US-ASCII default makes every regex match on their text raise.
Encoding.default_external = Encoding::UTF_8
Encoding.default_internal = Encoding::UTF_8
workflow_path, root_dir = ARGV
workflow = YAML.load_file(workflow_path)
text = File.read(workflow_path)
jobs = workflow.fetch("jobs")

# The tap is the last mile of a release: until the tap's formula serves the
# tagged version, `brew install` quietly hands out the previous one (0.4.0
# shipped while the tap kept 0.3.2 for a month). The check must therefore run
# after the bump on success AND failure — a rejected token is exactly when the
# tap is stale — and must never need the token itself, because a check that
# required HOMEBREW_TAP_TOKEN could never detect that token being broken.
tap = jobs["verify-tap"] or raise "verify-tap job missing"
raise "verify-tap must depend on bump-homebrew" unless tap["needs"] == %w[bump-homebrew]
raise "verify-tap gate missing the tag condition" unless tap["if"].to_s.include?("refs/tags/v")
raise "verify-tap must run after a failed bump" unless tap["if"].to_s.include?("!cancelled()")
# The step guards are load-bearing: without the check step's skip gate, a
# failed publish would be masked by a misleading secondary staleness failure;
# without the explicit bump-failure path (job-level !cancelled()), the whole
# feature would silently no-op on exactly the failure it exists to catch.
check_step = tap.fetch("steps").find { |s| s["if"].to_s == "needs.bump-homebrew.result != 'skipped'" } or raise "verify-tap lost its skip-aware gate on the check step"
raise "verify-tap check step no longer runs the tap check" unless check_step.dig("run").to_s.include?("bash scripts/check-homebrew-tap.sh")
raise "verify-tap warn-only knob lost its fail/warn split" unless check_step.dig("env", "TAP_CHECK_WARN_ONLY").to_s.include?("secrets.HOMEBREW_TAP_TOKEN != '' && '0' || '1'")
skip_step = tap.fetch("steps").find { |s| s["if"].to_s == "needs.bump-homebrew.result == 'skipped'" } or raise "verify-tap lost the explicit skip-explanation step"
tap_text = File.read(workflow_path)[/^  verify-tap:.*\z/m] or raise "verify-tap job text missing"
raise "verify-tap must invoke scripts/check-homebrew-tap.sh" unless tap_text.include?("bash scripts/check-homebrew-tap.sh")
raise "verify-tap must invoke the check with the tag's version" unless tap_text.include?('"${GITHUB_REF_NAME#v}"')
# The token's value may reach the bump job only; the check observes at most
# *that* it is configured, which keeps it working when the token is broken.
raise "the tap token value must be passed exactly once (to the bump job)" unless text.scan("HOMEBREW_TAP_TOKEN: ${{ secrets.HOMEBREW_TAP_TOKEN }}").length == 1
raise "verify-tap must observe only whether the token is configured" unless tap_text.include?("secrets.HOMEBREW_TAP_TOKEN != ''")
bump = jobs["bump-homebrew"] or raise "bump-homebrew job missing"
raise "bump-homebrew gate changed" unless bump["if"].to_s.include?("refs/tags/v")
raise "bump-homebrew no longer bumps after publish" unless bump["needs"] == %w[publish]
raise "bump-homebrew invocation changed" unless bump["steps"].map { |s| s["run"] }.compact.join.include?('bash scripts/update-homebrew-formula.sh "${GITHUB_REF_NAME#v}"')

script = File.join(root_dir, "scripts", "check-homebrew-tap.sh")
raise "scripts/check-homebrew-tap.sh missing" unless File.file?(script)

TRIPLES = ["aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-unknown-linux-gnu"].freeze
formula_fixture = lambda do |versions|
  lines = [
    "class Saya < Formula",
    '  desc "Database-aware terminal AI agent: TUI, schema discovery, read-only SQL"',
    '  homepage "https://github.com/databook-studio/saya-cli"',
    '  license "Apache-2.0"',
    "",
  ]
  TRIPLES.each_with_index do |triple, i|
    version = versions[i]
    lines << %(  url "https://github.com/databook-studio/saya-cli/releases/download/v#{version}/saya-#{version}-#{triple}.tar.gz")
    lines << '  sha256 "' + ("0" * 64) + '"'
  end
  lines << "end"
  lines.join("\n") + "\n"
end

Dir.mktmpdir("tap-check-fixtures") do |dir|
  fixture_path = File.join(dir, "saya.rb")
  write_fixture = ->(versions) { File.write(fixture_path, formula_fixture.call(versions)) }
  run_check = lambda do |version, env = {}|
    Open3.capture3(env, "bash", script, version)
  end

  # The tap serves the released version: pass.
  write_fixture.call(["0.4.1"] * 3)
  out, = run_check.call("0.4.1", "SAYA_TAP_FORMULA_FILE" => fixture_path)
  raise "matching tap should pass" unless out.include?("0.4.1") && out.include?("matches")

  # The tap still serves the previous release: fail naming both versions.
  write_fixture.call(["0.3.2"] * 3)
  out, err, status = run_check.call("0.4.1", "SAYA_TAP_FORMULA_FILE" => fixture_path)
  raise "stale tap should exit 1" unless status.exitstatus == 1
  raise "stale-tap message must name the served and expected versions" unless err.include?("0.3.2") && err.include?("0.4.1")

  # A deliberately unconfigured token downgrades staleness to a warning.
  out, err, status = run_check.call("0.4.1", "SAYA_TAP_FORMULA_FILE" => fixture_path, "TAP_CHECK_WARN_ONLY" => "1")
  raise "warn-only stale tap should exit 0" unless status.success?
  raise "warn-only stale tap should annotate ::warning" unless (out + err).include?("::warning")
  raise "warn-only message must still name both versions" unless (out + err).include?("0.3.2") && (out + err).include?("0.4.1")

  # Mixed versions in one formula are stale too.
  write_fixture.call(["0.4.1", "0.4.1", "0.3.2"])
  out, err, status = run_check.call("0.4.1", "SAYA_TAP_FORMULA_FILE" => fixture_path)
  raise "mixed-version tap should exit 1" unless status.exitstatus == 1
  raise "mixed-version message must name both versions" unless err.include?("0.3.2") && err.include?("0.4.1")

  # A formula without recognizable release URLs cannot be judged: never
  # report that as staleness.
  File.write(fixture_path, "class Saya < Formula\nend\n")
  out, err, status = run_check.call("0.4.1", "SAYA_TAP_FORMULA_FILE" => fixture_path)
  raise "unparseable formula should exit 2" unless status.exitstatus == 2
  raise "unparseable formula must say the check could not run" unless err.include?("could not run")

  # A missing formula is a stale (empty) tap, not a broken check.
  out, err, status = run_check.call("0.4.1", "SAYA_TAP_FORMULA_FILE" => File.join(dir, "absent.rb"))
  raise "missing formula should exit 1" unless status.exitstatus == 1
  raise "missing-formula message must name the expected version" unless err.include?("0.4.1")

  # A stale version quoted in a comment must not make a correct tap look mixed.
  write_fixture.call(["0.4.0"] * 3)
  commented = "# superseded: .../releases/download/v0.3.2/saya-0.3.2-x86_64-unknown-linux-gnu.tar.gz\n"
  File.write(fixture_path, commented + File.read(fixture_path))
  out, _err, status = run_check.call("0.4.0", "SAYA_TAP_FORMULA_FILE" => fixture_path)
  raise "a stale version in a comment should not fail a correct tap" unless status.success? && out.include?("matches")

  # Prerelease versions survive the extraction and comparison verbatim.
  write_fixture.call(["0.5.0-rc.1"] * 3)
  out, _err, status = run_check.call("0.5.0-rc.1", "SAYA_TAP_FORMULA_FILE" => fixture_path)
  raise "prerelease version should round-trip" unless status.success? && out.include?("0.5.0-rc.1")

  # Editor/transport artifacts (BOM, CRLF) must not break the extraction.
  File.write(fixture_path, "\xEF\xBB\xBF".dup.force_encoding(Encoding::UTF_8) + formula_fixture.call(["0.4.0"] * 3).gsub("\n", "\r\n"))
  out, _err, status = run_check.call("0.4.0", "SAYA_TAP_FORMULA_FILE" => fixture_path)
  raise "BOM/CRLF formula should still verify" unless status.success?

  # An empty formula is unknowable, and a directory is not a formula.
  File.write(fixture_path, "")
  out, err, status = run_check.call("0.4.0", "SAYA_TAP_FORMULA_FILE" => fixture_path)
  raise "empty formula should exit 2" unless status.exitstatus == 2 && err.include?("could not run")
  out, err, status = run_check.call("0.4.0", "SAYA_TAP_FORMULA_FILE" => dir)
  raise "directory as formula should exit 1" unless status.exitstatus == 1 && err.include?("serves no")

  # Usage errors are could-not-run (2), distinct from staleness (1).
  out, err, status = Open3.capture3("bash", script)
  raise "missing argument should exit 2" unless status.exitstatus == 2 && err.include?("usage:")
  out, err, status = Open3.capture3("bash", script, "")
  raise "empty argument should exit 2" unless status.exitstatus == 2

  # Warn-only downgrades staleness only: could-not-run must never exit 0.
  File.write(fixture_path, "class Saya < Formula\nend\n")
  out, err, status = run_check.call("0.4.0", "SAYA_TAP_FORMULA_FILE" => fixture_path, "TAP_CHECK_WARN_ONLY" => "1")
  raise "warn-only must not downgrade could-not-run" unless status.exitstatus == 2 && err.include?("could not run")
  out, err, status = run_check.call("0.4.0", "SAYA_TAP_FORMULA_FILE" => File.join(dir, "absent.rb"), "TAP_CHECK_WARN_ONLY" => "1")
  raise "warn-only missing formula should warn and pass" unless status.success? && (out + err).include?("::warning") && (out + err).include?("0.4.0")

  # Warn-only mixed versions still warn with both named; a matching tap in
  # warn-only mode must not warn at all.
  write_fixture.call(["0.4.0", "0.4.0", "0.3.2"])
  out, err, status = run_check.call("0.4.0", "SAYA_TAP_FORMULA_FILE" => fixture_path, "TAP_CHECK_WARN_ONLY" => "1")
  raise "warn-only mixed should warn and pass" unless status.success? && (out + err).include?("::warning") && (out + err).include?("0.3.2")
  write_fixture.call(["0.4.0"] * 3)
  out, err, status = run_check.call("0.4.0", "SAYA_TAP_FORMULA_FILE" => fixture_path, "TAP_CHECK_WARN_ONLY" => "1")
  raise "warn-only matching tap must not warn" unless status.success? && !(out + err).include?("::warning")
end
puts "tap check contract and verify-tap wiring valid"
RUBY
