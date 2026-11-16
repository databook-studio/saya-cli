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
raise "bump-homebrew no longer bumps after publish" unless bump["needs"].sort == %w[guard-tag publish]
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
  # A033 guard: tag/manifest equality must refuse before side effects.
  guard = jobs["guard-tag"] or raise "guard-tag job missing"
  raise "guard-tag must not be conditional on its own dispatch input" if guard.key?("if")
  guard_text = File.read(workflow_path)[/^  guard-tag:.*?(?=^  \S)/m] or raise "guard-tag job text missing"
  raise "guard-tag must run the tag/manifest check" unless guard_text.include?("bash scripts/check-tag-manifest.sh")
  raise "guard-tag must pass the tag version" unless guard_text.include?('"${GITHUB_REF_NAME#v}"')
  raise "guard-tag failure message must name both values" unless guard_text.include?("pushed tag") && guard_text.include?("manifest")
  %w[publish publish-crates bump-homebrew].each do |side_effect|
    needs = Array(jobs.dig(side_effect, "needs"))
    raise "#{side_effect} can run without guard-tag" unless needs.include?("guard-tag")
  end

  script = File.join(root_dir, "scripts", "check-tag-manifest.sh")
  raise "scripts/check-tag-manifest.sh missing" unless File.file?(script)

  Dir.mktmpdir("tag-guard-fixtures") do |dir|
    run_check = lambda do |tag, manifest|
      Open3.capture3("bash", script, tag, manifest)
    end

    # Matching tag/manifest: pass, naming the agreed version.
    out, _err, status = run_check.call("0.4.1", "0.4.1")
    raise "matching tag/manifest should pass" unless status.success? && out.include?("0.4.1")

    # A dispatch validation build carries no tag: it must keep working.
    out, _err, status = run_check.call("", "0.4.1")
    raise "empty tag should pass as a validation build" unless status.success?

    # The defect: pushed tag v0.4.2 with manifest 0.4.1 must fail, naming both.
    out, err, status = run_check.call("0.4.2", "0.4.1")
    raise "mismatched tag/manifest should exit 1" unless status.exitstatus == 1
    raise "mismatch message must name the tag and the manifest" unless err.include?("0.4.2") && err.include?("0.4.1")

    # Usage errors are could-not-run (2), distinct from a mismatch (1).
    _out, err, status = Open3.capture3("bash", script)
    raise "missing arguments should exit 2" unless status.exitstatus == 2 && err.include?("usage:")
  end
  puts "tap check contract and verify-tap wiring valid"
