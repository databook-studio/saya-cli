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
# Single-source the MSRV from the workspace manifest: clippy.toml and the CI
# MSRV job must equal it. Hardcoding the number here has already drifted once
# (1.88 -> 1.89 landed without this check noticing), so the script itself never
# names the value — it only enforces that the three declarations agree.
rust_version = package[/^rust-version = "([^"]+)"$/, 1] or raise "workspace rust-version missing"
clippy = File.read(File.join(root_dir, "clippy.toml"))
clippy_msrv = clippy[/^msrv = "([^"]+)"$/, 1] or raise "clippy.toml msrv setting missing"
raise "clippy.toml msrv #{clippy_msrv.inspect} does not match the workspace rust-version #{rust_version.inspect}" unless clippy_msrv == rust_version
member_block = workspace[/members\s*=\s*\[(.*?)\]/m, 1] or raise "workspace members missing"
members = member_block.scan(/"([^"]+)"/).flatten
raise "expected seven workspace members" unless members.length == 7
manifests = members.map { |member| File.join(root_dir, member, "Cargo.toml") }
raise "workspace member manifest missing" unless manifests.all? { |path| File.file?(path) }
package_names = manifests.map { |path| File.read(path)[/^\[package\]\n(.*?)(?=^\[|\z)/m, 1][/^name = "([^"]+)"$/, 1] }
raise "workspace package names do not match members" unless package_names.sort == %w[saya-agent saya-cli saya-config saya-connectors saya-harness saya-store saya-types]
release_version = package_names.zip(manifests).map { |_name, path| File.read(path)[/^\[package\]\n(.*?)(?=^\[|\z)/m, 1][/^version = "([^"]+)"$/, 1] }.compact.uniq
raise "workspace release versions are inconsistent" unless release_version.length == 1
manifests.each do |path|
  member_package = File.read(path)[/^\[package\]\n(.*?)(?=^\[|\z)/m, 1]
  raise "#{path} does not inherit the workspace MSRV" unless member_package&.match?(/^rust-version\.workspace = true$/)
end
publish_script = File.read(File.join(root_dir, "scripts", "publish-crates.sh"))
publish_order = publish_script[/^CRATES=\((.*?)\)$/, 1]&.split
raise "publish order does not cover every workspace crate" unless publish_order == %w[saya-types saya-config saya-store saya-agent saya-connectors saya-harness saya-cli]
msrv = workflow.dig("jobs", "msrv") or raise "MSRV job missing"
raise "MSRV job is not Ubuntu" unless msrv["runs-on"] == "ubuntu-latest"
raise "MSRV build is not serialized" unless msrv.dig("env", "CARGO_BUILD_JOBS") == "1"
raise "MSRV debug info is enabled" unless msrv.dig("env", "CARGO_PROFILE_DEV_DEBUG") == "0"
msrv_action = msrv.fetch("steps").find { |step| step["uses"]&.include?("dtolnay/rust-toolchain@") }
# The MSRV job must pin the exact toolchain release the workspace MSRV names:
# a two-component rust-version (1.89) maps to its patch-0 toolchain (1.89.0).
msrv_toolchain = rust_version.count(".") == 1 ? "#{rust_version}.0" : rust_version
raise "MSRV job toolchain #{msrv_action&.dig("with", "toolchain").inspect} does not match the workspace rust-version #{rust_version.inspect}" unless msrv_action&.dig("with", "toolchain") == msrv_toolchain
raise "MSRV action is not pinned" unless msrv_action["uses"].end_with?("4cda84d5c5c54efe2404f9d843567869ab1699d4")
commands = msrv.fetch("steps").map { |step| step["run"] }.compact
raise "MSRV workspace check missing" unless commands == ["cargo check --workspace --locked"]
# Validate the DuckDB dependency semantically, not line-for-line: the exact
# substring went stale the moment the parquet feature was added. The contract
# is the exact pin "=1.10505.0" plus the features the bundled build requires,
# with feature order left free.
duckdb = manifest[/^duckdb = \{.*\}$/, 0] or raise "workspace DuckDB dependency missing"
duckdb_version = duckdb[/version = "([^"]+)"/, 1] or raise "workspace DuckDB version pin missing"
raise "workspace DuckDB is not exactly pinned (got #{duckdb_version.inspect}, expected =1.10505.0)" unless duckdb_version == "=1.10505.0"
duckdb_features = duckdb[/features = \[(.*?)\]/, 1].to_s.scan(/"([^"]+)"/).flatten
%w[bundled parquet].each do |feature|
  raise "workspace DuckDB is missing the required #{feature} feature" unless duckdb_features.include?(feature)
end
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
