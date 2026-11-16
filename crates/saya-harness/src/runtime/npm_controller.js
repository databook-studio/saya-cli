"use strict";

const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const bundle = "/saya/bundle";
const source = "/saya/source";
const work = "/saya/work";
const project = path.join(work, "project");
const cache = "/saya/cache";
const npm = "/usr/local/lib/node_modules/npm/bin/npm-cli.js";
const node = "/usr/local/bin/node";
const action = process.argv[2];

function stop(code, message) {
  if (message) process.stderr.write(`${message}\n`);
  process.exit(code);
}

function run(args) {
  const result = spawnSync(node, [npm, ...args], {
    cwd: project,
    env: {
      PATH: "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
      HOME: path.join(work, "home"),
      NPM_CONFIG_CACHE: cache,
      NPM_CONFIG_USERCONFIG: path.join(work, "user.npmrc"),
      NPM_CONFIG_GLOBALCONFIG: path.join(work, "global.npmrc"),
      NPM_CONFIG_AUDIT: "false",
      NPM_CONFIG_FUND: "false",
      NPM_CONFIG_OFFLINE: "true",
    },
    stdio: "inherit",
  });
  if (result.error) stop(125, "managed npm process could not start");
  if (result.signal) process.kill(process.pid, result.signal);
  return result.status === null ? 125 : result.status;
}

if (!new Set(["install-deps", "test-ci", "build"]).has(action)) {
  stop(125, "unsupported managed Node action");
}
if (process.env.SAYA_NODE_ACTION !== action) {
  stop(125, "managed Node action marker did not match");
}

try {
  fs.mkdirSync(path.join(work, "home"), { recursive: true });
  fs.mkdirSync(cache, { recursive: true });
  fs.writeFileSync(path.join(work, "user.npmrc"), "", { mode: 0o600 });
  fs.writeFileSync(path.join(work, "global.npmrc"), "", { mode: 0o600 });
  fs.mkdirSync(project, { recursive: true });
  if (action !== "install-deps") fs.cpSync(source, project, { recursive: true });
  fs.copyFileSync(path.join(bundle, "package.json"), path.join(project, "package.json"));
  fs.copyFileSync(path.join(bundle, "package-lock.json"), path.join(project, "package-lock.json"));
  const manifest = JSON.parse(fs.readFileSync(path.join(bundle, "bundle.json"), "utf8"));
  for (const item of manifest.tarballs) {
    if (!/^tarballs\/[0-9]{4}\.tgz$/.test(item.file)) stop(125, "invalid verified tarball path");
    const status = run([
      "cache", "add", path.join(bundle, item.file), "--offline", "--ignore-scripts",
      "--no-audit", "--no-fund", "--cache", cache, "--userconfig", path.join(work, "user.npmrc"),
      "--globalconfig", path.join(work, "global.npmrc"),
    ]);
    if (status !== 0) process.exit(status);
  }
  const offline = [
    "--offline", "--ignore-scripts", "--no-audit", "--no-fund", "--global=false",
    "--include=optional", "--registry=https://registry.npmjs.org", "--cache", cache,
    "--userconfig", path.join(work, "user.npmrc"), "--globalconfig", path.join(work, "global.npmrc"),
  ];
  const installed = run([...offline, "ci"]);
  if (installed !== 0 || action === "install-deps") process.exit(installed);
  process.exit(run([...offline, "run", action === "test-ci" ? "test:ci" : "build"]));
} catch (error) {
  process.stderr.write("managed npm action failed before a result was produced\n");
  process.exit(125);
}
