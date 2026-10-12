import { spawnSync } from "node:child_process";
import { access, chmod, copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { constants } from "node:fs";
import { tmpdir, homedir } from "node:os";
import { isAbsolute, join, resolve, sep } from "node:path";
import { ProbeError } from "./client.mjs";

export async function discoverCodex(explicit = process.env.CODEX_BIN) {
  if (explicit) {
    if (!isAbsolute(explicit)) throw new ProbeError("CODEX_PATH_NOT_ABSOLUTE");
    try { await access(explicit, constants.F_OK); return explicit; }
    catch { throw new ProbeError("CODEX_NOT_INSTALLED"); }
  }
  if (process.platform === "win32") {
    const result = spawnSync("powershell.exe", ["-NoProfile", "-Command",
      "$cmd=Get-Command codex.exe -ErrorAction SilentlyContinue; if($cmd){$cmd.Source} else {$pkg=Get-AppxPackage OpenAI.Codex; if($pkg){Join-Path $pkg.InstallLocation 'app\\resources\\codex.exe'}}"],
    { encoding: "utf8", windowsHide: true, timeout: 10000 });
    const candidate = result.stdout?.trim();
    if (result.status === 0 && candidate) return discoverCodex(candidate);
  } else {
    for (const dir of (process.env.PATH ?? "").split(":")) {
      try { const candidate = resolve(dir, "codex"); await access(candidate, constants.X_OK); return candidate; } catch { /* next */ }
    }
  }
  throw new ProbeError("CODEX_NOT_INSTALLED");
}

// Deliberately narrow experiment import: excludes MCP, plugins, notify, project trust and instructions.
// Provider stanzas are copied as a whole (including multiline TOML); root values must be one line.
export function connectionConfig(source) {
  const allowed = new Set(["model", "model_provider", "model_reasoning_effort", "model_catalog_json",
    "model_context_window", "model_auto_compact_token_limit", "model_auto_compact_token_limit_scope",
    "cli_auth_credentials_store"]);
  let section = "";
  const lines = [];
  for (const line of source.split(/\r?\n/)) {
    if (/^\s*\[/.test(line)) section = line.trim();
    if (/^\[model_providers[.\]]/.test(section)) { lines.push(line); continue; }
    if (section) continue;
    const key = line.match(/^\s*([a-z_]+)\s*=/)?.[1];
    if (!allowed.has(key)) continue;
    if (/=\s*(\[|\{|"""|''')/.test(line)) throw new ProbeError("CONFIG_IMPORT_UNSUPPORTED");
    lines.push(line);
  }
  return lines.join("\n") + "\n";
}

// Fail closed before a real probe: official-source tests remain paused by the plan.
// Only the simple connection values exported by cc-switch are supported here.
export function nonOfficialConnection(source) {
  const root = source.split(/^\s*\[/m)[0];
  const provider = root.match(/^\s*model_provider\s*=\s*"([\w-]+)"\s*(?:#.*)?$/m)?.[1];
  if (!provider || provider === "openai") throw new ProbeError("OFFICIAL_SOURCE_TESTS_PAUSED");
  const stanza = source.split(/\r?\n/);
  let active = false;
  const lines = [];
  for (const line of stanza) {
    if (/^\s*\[/.test(line)) active = line.trim() === `[model_providers.${provider}]`;
    else if (active) lines.push(line);
  }
  const settings = lines.join("\n");
  const endpointText = settings.match(/^\s*base_url\s*=\s*"([^"\\]+)"\s*(?:#.*)?$/m)?.[1];
  let endpoint;
  try { endpoint = new URL(endpointText); } catch { throw new ProbeError("PROVIDER_ENDPOINT_INVALID"); }
  if (endpoint.hostname === "openai.com" || endpoint.hostname.endsWith(".openai.com")) {
    throw new ProbeError("OFFICIAL_SOURCE_TESTS_PAUSED");
  }
  if (endpoint.protocol !== "https:" || endpoint.username || endpoint.password || endpoint.search || endpoint.hash) {
    throw new ProbeError("PROVIDER_ENDPOINT_INVALID");
  }
  if (!/^\s*requires_openai_auth\s*=\s*false\s*(?:#.*)?$/m.test(settings)) {
    throw new ProbeError("OFFICIAL_SOURCE_TESTS_PAUSED");
  }
  return { provider, endpoint: endpoint.href, protocol: settings.match(/^\s*wire_api\s*=\s*"([\w-]+)"/m)?.[1] };
}

export async function isolatedEnvironment({ inheritConnection = true, config, nonOfficialOnly = false } = {}) {
  const root = await mkdtemp(join(tmpdir(), "project-assistant-0.3-"));
  try {
    // Credentials never enter artifacts. Windows requires an explicit restricted DACL.
    if (process.platform === "win32") {
      const sid = spawnSync("powershell.exe", ["-NoProfile", "-Command",
        "[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value"],
      { encoding: "utf8", windowsHide: true, timeout: 10000 });
      if (sid.status !== 0 || !/^S-1-\d+(?:-\d+)+$/.test(sid.stdout.trim())) throw new ProbeError("TEMP_ACL_FAILED");
      const acl = spawnSync("icacls.exe", [root, "/inheritance:r", "/grant:r", `*${sid.stdout.trim()}:(OI)(CI)F`],
        { windowsHide: true, stdio: "ignore", timeout: 10000 });
      if (acl.status !== 0) throw new ProbeError("TEMP_ACL_FAILED");
    } else await chmod(root, 0o700);
    const home = join(root, "codex-home");
    const project = join(root, "隔离 project & 验证");
    await mkdir(home);
    await mkdir(project);
    const sourceHome = process.env.CODEX_HOME || join(homedir(), ".codex");
    let configuration = config ?? "";
    if (inheritConnection) {
      try { configuration = connectionConfig(await readFile(join(sourceHome, "config.toml"), "utf8")); }
      catch (error) { if (error.code !== "ENOENT") throw error; }
      if (nonOfficialOnly) nonOfficialConnection(configuration);
      try { await copyFile(join(sourceHome, "auth.json"), join(home, "auth.json")); await chmod(join(home, "auth.json"), 0o600); }
      catch (error) { if (error.code !== "ENOENT") throw error; }
    }
    await writeFile(join(home, "config.toml"), configuration, { mode: 0o600 });
    return { root, home, project, env: { ...process.env, CODEX_HOME: home }, cleanup: () => clean(root) };
  } catch (error) { await clean(root); throw error; }
}

async function clean(root) {
  if (!resolve(root).startsWith(resolve(tmpdir()) + sep) || !root.split(sep).at(-1).startsWith("project-assistant-0.3-")) {
    throw new ProbeError("TEMP_CLEANUP_BOUNDARY");
  }
  await rm(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
}
