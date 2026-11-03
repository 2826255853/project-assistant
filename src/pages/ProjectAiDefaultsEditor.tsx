import { useEffect, useMemo, useRef, useState } from "react";
import { ErrorNotice } from "../components/ErrorNotice";
import { ProviderSelection, selectionFrom } from "../components/ProviderSelection";
import { call } from "../ipc/client";
import type { AiSelection, ModelCapability, ProjectSettingsView, ProviderProfile } from "../ipc/contracts";
import { toPublicError } from "../ipc/errors";
import type { PublicError } from "../ipc/errors";

type Props = {
  projectId: string;
  view: ProjectSettingsView;
  onUpdated: (view: ProjectSettingsView) => void;
  onReload: () => Promise<ProjectSettingsView>;
  disabled?: boolean;
};

function projectDefault(view: ProjectSettingsView): AiSelection | null {
  const setting = view.values.find(value => value.key === "ai_defaults");
  return setting?.source === "project" && setting.value
    ? { profileId: setting.value.profileId, expectedRevision: setting.value.expectedRevision, modelId: setting.value.modelId, effort: setting.value.effort }
    : null;
}

export function ProjectAiDefaultsEditor({ projectId, view, onUpdated, onReload, disabled = false }: Props) {
  const [profiles, setProfiles] = useState<ProviderProfile[]>([]);
  const [profilesLoaded, setProfilesLoaded] = useState(false);
  const [capabilities, setCapabilities] = useState<ModelCapability[]>([]);
  const [capabilitiesLoadedProfile, setCapabilitiesLoadedProfile] = useState<string | null>(null);
  const [mode, setMode] = useState<"inherit" | "project">("inherit");
  const [profileId, setProfileId] = useState("");
  const [modelId, setModelId] = useState("");
  const [effort, setEffort] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<PublicError | null>(null);
  const [notice, setNotice] = useState("");
  const profileRequest = useRef(0);
  const capabilityRequest = useRef(0);
  const stored = projectDefault(view);
  const availableProfiles = useMemo(
    () => profiles.filter(profile => profile.sourceKind === "relay" && profile.adapterKind === "self_managed"),
    [profiles],
  );
  const selectedProfile = availableProfiles.find(profile => profile.id === profileId) ?? null;
  const selectedCapability = capabilities.find(capability => capability.modelId === modelId) ?? null;
  const validEffort = selectedCapability && (effort === null
    ? selectedCapability.supportsDefaultEffort
    : selectedCapability.efforts.includes(effort));
  const selection = mode === "project" && validEffort
    ? selectionFrom(selectedProfile, modelId, effort)
    : null;
  const dirty = mode === "inherit"
    ? stored !== null
    : Boolean(selection && (!stored || stored.profileId !== selection.profileId || stored.expectedRevision !== selection.expectedRevision || stored.modelId !== selection.modelId || stored.effort !== selection.effort));
  const storedProfile = stored ? profiles.find(profile => profile.id === stored.profileId) : null;
  const storedInvalid = Boolean(stored && profilesLoaded && (
    !storedProfile || storedProfile.sourceKind !== "relay" || storedProfile.adapterKind !== "self_managed" ||
    !storedProfile.enabled || !storedProfile.available || storedProfile.configRevision !== stored.expectedRevision ||
    (capabilitiesLoadedProfile === stored.profileId && (!capabilities.some(capability =>
      capability.modelId === stored.modelId && (stored.effort === null
        ? capability.supportsDefaultEffort
        : capability.efforts.includes(stored.effort)))) )
  ));

  useEffect(() => {
    let active = true;
    const id = ++profileRequest.current;
    setError(null);
    setCapabilities([]);
    setCapabilitiesLoadedProfile(null);
    setProfilesLoaded(false);
    const initial = projectDefault(view);
    setMode(initial ? "project" : "inherit");
    setProfileId(initial?.profileId ?? "");
    setModelId(initial?.modelId ?? "");
    setEffort(initial?.effort ?? null);
    void call("providers_list", {}).then(result => {
      if (active && id === profileRequest.current) { setProfiles(result.profiles); setProfilesLoaded(true); }
    }).catch(cause => {
      if (active && id === profileRequest.current) setError(toPublicError(cause));
    });
    return () => { active = false; ++profileRequest.current; };
  }, [projectId, view]);

  function changeProfile(id: string) {
    ++capabilityRequest.current;
    const profile = availableProfiles.find(value => value.id === id);
    setProfileId(id);
    setCapabilities([]);
    setCapabilitiesLoadedProfile(null);
    setModelId(profile?.defaultModel ?? "");
    setEffort(profile?.defaultEffort ?? null);
    setNotice("");
    setError(null);
  }

  async function readCapabilities() {
    if (!selectedProfile || busy) return;
    const id = ++capabilityRequest.current;
    setBusy(true);
    setError(null);
    setNotice("");
    try {
      const result = await call("provider_models", {
        profileId: selectedProfile.id,
        expectedRevision: selectedProfile.configRevision,
        refresh: false,
      });
      if (id !== capabilityRequest.current) return;
      setCapabilities(result.capabilities);
      setCapabilitiesLoadedProfile(selectedProfile.id);
      if (!result.capabilities.some(value => value.modelId === modelId)) {
        const preferred = result.capabilities.find(value => value.modelId === selectedProfile.defaultModel) ?? result.capabilities[0];
        setModelId(preferred?.modelId ?? "");
        setEffort(selectedProfile.defaultEffort && preferred?.efforts.includes(selectedProfile.defaultEffort)
          ? selectedProfile.defaultEffort
          : preferred?.supportsDefaultEffort ? null : preferred?.efforts[0] ?? null);
      }
      setNotice(result.capabilities.length ? `已读取 ${result.capabilities.length} 个当前可用模型。` : "此服务源没有当前有效的能力记录。请先在 AI 配置页刷新能力并测试，再回来保存。");
    } catch (cause) {
      if (id === capabilityRequest.current) setError(toPublicError(cause));
    } finally {
      if (id === capabilityRequest.current) setBusy(false);
    }
  }

  useEffect(() => () => { ++capabilityRequest.current; }, []);

  async function reloadAfterConflict() {
    try {
      onUpdated(await onReload());
      setNotice("设置已重读。请确认当前值后重新编辑并提交。");
    } catch (cause) {
      setError(toPublicError(cause));
    }
  }

  async function save() {
    if (!dirty || busy || mode === "project" && !selection) return;
    setBusy(true);
    setError(null);
    setNotice("");
    try {
      const next = await call("project_settings_update", {
        projectId,
        expectedRevision: view.revision,
        expectedGlobalRevision: view.globalRevision,
        selection: mode === "inherit" ? null : selection,
      });
      onUpdated(next);
      setNotice(mode === "inherit" ? "项目默认已清除，当前继承全局 AI 默认值。" : "项目 AI 默认值已保存。" );
    } catch (cause) {
      const safe = toPublicError(cause);
      setError(safe);
      if (safe.code === "PA_PROJECT_CONFLICT_REVISION" || safe.code === "PA_CONFLICT_REVISION") await reloadAfterConflict();
    } finally {
      setBusy(false);
    }
  }

  const disabledForm = disabled || busy;
  return <section className="project-ai-defaults" data-testid="project-ai-defaults" data-revision={`${view.revision}:${view.globalRevision}`}>
    <h5>项目 AI 默认值</h5>
    <p className="field-help">任务将使用单一完整来源：本次选择优先，其次项目默认，再其次全局默认。此处不保存或显示 Key。</p>
    <label className="project-ai-mode"><input type="radio" data-testid="project-ai-inherit-mode" name={`project-ai-mode-${projectId}`} checked={mode === "inherit"} disabled={disabledForm} onChange={() => { setMode("inherit"); setNotice(""); }} />继承全局默认</label>
    <label className="project-ai-mode"><input type="radio" data-testid="project-ai-project-mode" name={`project-ai-mode-${projectId}`} checked={mode === "project"} disabled={disabledForm} onChange={() => { setMode("project"); setNotice(""); }} />设置项目默认</label>
    {mode === "project" && <>
      {storedInvalid && <p role="alert" className="project-ai-invalid" data-testid="project-ai-invalid">已保存的项目 AI 默认已失效。系统不会静默改用全局默认；请重新选择有效组合或清除项目覆盖。</p>}
      <button type="button" className="quiet-button" data-testid="project-ai-read-capabilities" disabled={disabledForm || !selectedProfile} onClick={() => void readCapabilities()}>{busy ? "正在读取…" : "读取模型能力"}</button>
      <ProviderSelection
        profiles={availableProfiles}
        capabilities={capabilities}
        selectedProfileId={profileId}
        modelId={modelId}
        effort={effort}
        disabled={disabledForm}
        onProfileChange={changeProfile}
        onModelChange={value => { setModelId(value); const capability = capabilities.find(item => item.modelId === value); setEffort(capability?.supportsDefaultEffort ? null : capability?.efforts[0] ?? null); }}
        onEffortChange={setEffort}
      />
      <p className="field-help">只接受启用的应用自管非官方 HTTPS 服务源和当前有效的 profile 修订、凭据状态及模型能力。能力过期或配置变化后，保存会被拒绝。</p>
    </>}
    <div className="form-actions">
      <button type="button" className="primary-button" data-testid="project-ai-save" disabled={disabledForm || !dirty || mode === "project" && !selection} onClick={() => void save()}>{busy ? "正在保存…" : "保存项目默认"}</button>
      {stored && <button type="button" className="quiet-button" data-testid="project-ai-inherit" disabled={disabledForm || mode === "inherit"} onClick={() => { setMode("inherit"); setNotice(""); }}>清除并继承</button>}
    </div>
    {notice && <p className="inline-status" role="status" data-testid="project-ai-notice">{notice}</p>}
    {error && <ErrorNotice error={error} testId="project-ai-error" />}
  </section>;
}
