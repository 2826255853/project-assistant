// Read-only installation survey. Does not query internal database tables or change live configuration.
import { spawnSync } from "node:child_process";
import { mkdir, open, writeFile } from "node:fs/promises";
import { join, resolve, isAbsolute } from "node:path";
import { homedir } from "node:os";
import { ProbeError } from "./codex/client.mjs";

const output = resolve(import.meta.dirname, "..", "artifacts", "cc-switch");
await mkdir(output, { recursive: true });
const report = { date: new Intl.DateTimeFormat("sv-SE", { timeZone: "Asia/Shanghai" }).format(new Date()),
  supportedSurveyVersion: "4.0.6", sourceCommit: "c261126630b3416e67b407da018cc614bbbe84cd",
  providerReadSwitchEditAuthentication: "blocked-no-documented-external-bidirectional-interface",
  internalTauriCommands: "source-only-not-an-external-api", communityCli: "not-installed-or-tested",
  deepLink: "documented-import-only-not-tested", databaseTablesRead: false, liveConfigChanged: false };
try {
  if (process.platform !== "win32") throw new ProbeError("CC_SWITCH_SURVEY_WINDOWS_ONLY");
  const executable = process.env.CC_SWITCH_BIN ?? join(process.env.LOCALAPPDATA, "Programs", "CC Switch", "cc-switch.exe");
  if (!isAbsolute(executable)) throw new ProbeError("CC_SWITCH_PATH_NOT_ABSOLUTE");
  const metadata = spawnSync("powershell.exe", ["-NoProfile", "-Command", `
    $item=Get-Item -LiteralPath $env:PHASE_0_3_CC_BIN -ErrorAction Stop;
    $db=Join-Path $env:USERPROFILE '.cc-switch\\cc-switch.db';
    $broad=@();
    if(Test-Path -LiteralPath $db) {
      foreach($rule in (Get-Acl -LiteralPath $db).Access) {
        $sid=$rule.IdentityReference.Translate([System.Security.Principal.SecurityIdentifier]).Value;
        if($rule.AccessControlType -eq 'Allow' -and $sid -in @('S-1-1-0','S-1-5-11','S-1-5-32-545') -and ($rule.FileSystemRights -band [System.Security.AccessControl.FileSystemRights]::ReadData)) { $broad += $sid }
      }
    }
    [pscustomobject]@{version=$item.VersionInfo.ProductVersion; desktopRunning=[bool](Get-Process -Name 'cc-switch' -ErrorAction SilentlyContinue); protocolRegistered=(Test-Path 'Registry::HKEY_CLASSES_ROOT\\ccswitch'); broadReadSids=@($broad)} | ConvertTo-Json -Compress
  `], { encoding: "utf8", windowsHide: true, timeout: 10000, env: { ...process.env, PHASE_0_3_CC_BIN: executable } });
  if (metadata.status !== 0) throw new ProbeError("CC_SWITCH_NOT_INSTALLED_OR_INACCESSIBLE");
  Object.assign(report, JSON.parse(metadata.stdout));
  report.versionMatches = report.version === report.supportedSurveyVersion;
  const db = await open(join(homedir(), ".cc-switch", "cc-switch.db"), "r");
  try {
    const header = Buffer.alloc(16);
    await db.read(header, 0, 16, 0);
    report.standardSqliteHeader = header.equals(Buffer.from("SQLite format 3\0"));
  } finally { await db.close(); }
  report.credentialProtection = "source-serializes-provider-settings-as-json-no-at-rest-encryption-established";
  report.status = "blocked";
} catch (error) {
  report.status = "blocked";
  report.error = error instanceof ProbeError ? error.code : "CC_SWITCH_SURVEY_FAILED";
  process.exitCode = 1;
}
await writeFile(join(output, "report.json"), JSON.stringify(report, null, 2) + "\n");
console.log(`cc-switch survey: ${report.status}; evidence: ${join(output, "report.json")}`);
