root_dir = ARGV.fetch(0)
publish_script = File.join(root_dir, "scripts", "publish-crates.sh")
raise "scripts/publish-crates.sh missing" unless File.file?(publish_script)

crates = %w[saya-types saya-config saya-store saya-agent saya-connectors saya-harness saya-cli]
dependencies = {
  "saya-types" => [],
  "saya-config" => ["saya-types"],
  "saya-store" => ["saya-types"],
  "saya-agent" => ["saya-types"],
  "saya-connectors" => ["saya-config", "saya-types"],
  "saya-harness" => ["saya-agent", "saya-store", "saya-types", "saya-connectors"],
  "saya-cli" => ["saya-agent", "saya-config", "saya-connectors", "saya-harness", "saya-store", "saya-types"],
}

metadata = lambda do |versions, members = crates|
  packages = members.map do |name|
    id = "path+file:///fixture/#{name}##{versions.fetch(name)}"
    {
      "name" => name,
      "version" => versions.fetch(name),
      "id" => id,
      "dependencies" => dependencies.fetch(name, []).map do |dependency|
        { "name" => dependency, "req" => "^#{versions.fetch(dependency)}", "path" => "/fixture/#{dependency}" }
      end,
    }
  end
  { "workspace_members" => packages.map { |package| package.fetch("id") }, "packages" => packages }
end

run_fixture = lambda do |payload, reject_crate = ""|
  Dir.mktmpdir("publish-check") do |dir|
    scripts_dir = File.join(dir, "scripts")
    bin_dir = File.join(dir, "bin")
    FileUtils.mkdir_p(scripts_dir)
    FileUtils.mkdir_p(bin_dir)
    FileUtils.cp(publish_script, File.join(scripts_dir, "publish-crates.sh"))
    metadata_path = File.join(dir, "metadata.json")
    cargo_log = File.join(dir, "cargo.log")
    curl_log = File.join(dir, "curl.log")
    File.write(metadata_path, JSON.generate(payload))
    File.write(File.join(bin_dir, "cargo"), <<~'SH')
      #!/usr/bin/env bash
      set -euo pipefail
      if [[ "${1:-}" == "metadata" ]]; then
        cat "$SAYA_METADATA_FILE"
      elif [[ "${1:-}" == "package" ]]; then
        crate=""
        while (($#)); do
          if [[ "$1" == "-p" ]]; then crate="$2"; shift 2; else shift; fi
        done
        target_dir="${CARGO_TARGET_DIR:-target}"
        mkdir -p "$target_dir/package"
        : > "$target_dir/package/$crate-0.4.1.crate"
        printf 'stub cargo package %s\n' "$crate"
        printf 'package %s\n' "$crate" >> "$SAYA_CARGO_LOG"
      elif [[ "${1:-}" == "publish" ]]; then
        crate=""
        while (($#)); do
          if [[ "$1" == "-p" ]]; then crate="$2"; shift 2; else shift; fi
        done
        printf 'stub cargo publish -p %s\n' "$crate"
        printf 'publish %s\n' "$crate" >> "$SAYA_CARGO_LOG"
      else
        echo "unexpected cargo invocation: $*" >&2
        exit 1
      fi
    SH
    File.write(File.join(bin_dir, "python3"), <<~'SH')
      #!/usr/bin/env bash
      set -euo pipefail
      [[ "$1" == "scripts/check-artifact-content.py" && "$2" == "crate" ]]
      [[ -f "$3" ]]
      if [[ "${SAYA_REJECT_CRATE:-}" == "$(basename "$3" .crate)" ]]; then
        echo "synthetic content rejection" >&2
        exit 1
      fi
    SH
    File.write(File.join(bin_dir, "curl"), <<~'SH')
      #!/usr/bin/env bash
      set -euo pipefail
      printf 'stub curl %s\n' "$*" >&2
      printf '%s\n' "$*" >> "$SAYA_CURL_LOG"
      printf '404'
    SH
    FileUtils.chmod(0o755, [File.join(bin_dir, "cargo"), File.join(bin_dir, "curl"), File.join(bin_dir, "python3")])
    Open3.capture3(
      {
        "PATH" => "#{bin_dir}:#{ENV.fetch('PATH')}",
        "SAYA_METADATA_FILE" => metadata_path,
        "SAYA_CARGO_LOG" => cargo_log,
        "SAYA_CURL_LOG" => curl_log,
        "SAYA_REJECT_CRATE" => reject_crate,
        "DRY_RUN" => "1",
      },
      "bash", File.join(scripts_dir, "publish-crates.sh")
    ).tap { |result| result << cargo_log << curl_log }
  end
end

versions = crates.to_h { |name| [name, "0.4.1"] }
output, error, status, cargo_log, curl_log = run_fixture.call(metadata.call(versions))
raise "valid publish fixture should pass: #{output}\n#{error}" unless status.success?
published = output.lines.grep(/^stub cargo publish /).map { |line| line.split.fetch(4) }
raise "publish order omitted saya-harness before saya-cli: #{published.inspect}" unless published == crates
inspected = output.lines.grep(/^stub cargo package /).map { |line| line.split.fetch(3) }
raise "actual package command missing for every crate" unless inspected == crates
raise "content check did not run for every generated crate" unless output.lines.count { |line| line.include?("== inspect package ") } == crates.length
raise "valid publish fixture should query each crate exactly once" unless error.lines.count { |line| line.start_with?("stub curl ") } == crates.length

output, error, status, cargo_log, curl_log = run_fixture.call(metadata.call(versions), "saya-types-0.4.1")
raise "rejected crate content should fail the publish run" if status.success?
raise "content rejection should be reported" unless error.include?("synthetic content rejection")
raise "rejected crate reached cargo publish" if output.include?("stub cargo publish")

versions["saya-harness"] = "0.4.0"
output, error, status, cargo_log, curl_log = run_fixture.call(metadata.call(versions))
raise "mismatched workspace versions should fail" if status.success?
raise "version mismatch should be reported" unless (output + error).include?("saya-harness") && (output + error).include?("0.4.0")
raise "version mismatch reached publish" if output.include?("stub cargo publish")
raise "version mismatch reached registry" if error.include?("stub curl")

extra = crates + ["saya-plugin"]
versions["saya-plugin"] = "0.4.1"
output, error, status, cargo_log, curl_log = run_fixture.call(metadata.call(versions, extra))
raise "an added workspace package should fail" if status.success?
raise "added package should be reported" unless (output + error).include?("saya-plugin")
raise "added package reached publish" if output.include?("stub cargo publish")
raise "added package reached registry" if error.include?("stub curl")

puts "publish preflight and dependency order contract valid"
