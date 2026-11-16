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
build_steps = jobs.fetch("build").fetch("steps")
build_run = build_steps.map { |step| step["run"] }.compact.join("\n")
raise "release build does not scan the binary" unless build_run.include?("check-artifact-content.py binary")
raise "release build does not use shared path remapping" unless build_run.include?("release-build-env.sh") && build_run.include?("configure_saya_release_build_env")
raise "release binary path ignores CARGO_TARGET_DIR" unless build_run.include?("${CARGO_TARGET_DIR:-target}")
raise "Unix release archive is not scanned" unless build_run.include?("check-artifact-content.py archive")
raise "Unix tar retains AppleDouble metadata" unless build_run.include?("COPYFILE_DISABLE=1 tar -czf")
raise "Windows release ZIP is not scanned" unless build_run.include?("check-artifact-content.py archive") && build_run.include?("python scripts/check-artifact-content.py archive")
publish_script = File.read(File.join(File.dirname(path), "..", "..", "scripts", "publish-crates.sh"))
raise "crate payload scan missing before publish" unless publish_script.index("check-artifact-content.py crate")&.<(publish_script.index("cargo publish"))
resource_env = jobs.dig("build", "env")
raise "release build jobs are not serialized" unless resource_env["CARGO_BUILD_JOBS"] == "1"
raise "release test debug info is enabled" unless resource_env["CARGO_PROFILE_TEST_DEBUG"] == "0"
raise "top-level write permission" unless workflow.dig("permissions", "contents") == "read"
raise "publish write permission missing" unless jobs.dig("publish", "permissions", "contents") == "write"
raise "publish gate missing" unless jobs["publish"]["if"].include?("inputs.publish == true")
raise "publish dependencies missing" unless jobs["publish"]["needs"].sort == %w[build checksums guard-tag]
matrix = jobs.dig("build", "strategy", "matrix", "include")
arm64 = matrix.find { |entry| entry["platform"] == "linux-arm64" }
raise "linux-arm64 matrix entry missing" if arm64.nil?
raise "linux-arm64 must build natively (no target key)" if arm64.key?("target")
raise "linux-arm64 must run on a native ARM runner" unless arm64["os"] == "ubuntu-24.04-arm"
text = File.read(path)
release = 'gh release create "v${{ steps.version.outputs.version }}" release-assets/* --target "${{ github.sha }}" --generate-notes'
raise "release target is not github.sha" unless text.include?(release)
raise "reserved PowerShell $host assignment" if text.match?(/^\s*\$host\s*=/i)
%w[$targetTriple System.IO.File] .each { |value| raise "missing #{value}" unless text.include?(value) }
%w[WriteAllText UTF8Encoding] .each { |value| raise "missing #{value}" unless text.include?(value) }
raise "missing UTF-8 without BOM" unless text.include?("UTF8Encoding]::new($false)")
raise "missing LF sidecar newline" unless text.include?("$stage.zip`n")
raise "offline release command" if text.include?("--offline")
%w[_SECURE_SCL /std:c++17 /EHsc].each do |flag|
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
