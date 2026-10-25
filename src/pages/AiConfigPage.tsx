import { useEffect, useMemo, useRef, useState } from "react";
import { ErrorNotice } from "../components/ErrorNotice";
import { ProviderSelection, selectionFrom } from "../components/ProviderSelection";
import { call } from "../ipc/client";
import type { AiSelection, ModelCapability, OperationView, ProviderProfile } from "../ipc/contracts";
import { errorForCode, toPublicError } from "../ipc/errors";
import type { PublicError } from "../ipc/errors";
import { useAppStatus } from "../app/useAppStatus";

type Form = { name: string; endpoint: string; defaultModel: string; defaultEffort: string };
type PagePhase = "reading" | "ready" | "editing" | "saving" | "operating" | "failure";

const emptyForm: Form = { name: "", endpoint: "", defaultModel: "", defaultEffort: "" };
const terminal = (operation: OperationView) => ["completed", "failed", "cancelled"].includes(operation.phase);

export function AiConfigPage() {
  const { status } = useAppStatus();
  const [profiles, setProfiles] = useState<ProviderProfile[]>([]);
  const [active, setActive] = useState<AiSelection | null>(null);
  const [configState, setConfigState] = useState("unconfigured");
  const [capabilities, setCapabilities] = useState<ModelCapability[]>([]);
  const [profileId, setProfileId] = useState("");
  const [modelId, setModelId] = useState("");
  const [effort, setEffort] = useState<string | null>(null);
  const [form, setForm] = useState<Form>(emptyForm);
  const [secret, setSecret] = useState("");
  const [editingId, setEditingId] = useState<string | null>(null);
  const [phase, setPhase] = useState<PagePhase>("reading");
  const [error, setError] = useState<PublicError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [operation, setOperation] = useState<OperationView | null>(null);
  const requestId = useRef(0);
  const mounted = useRef(false);
  const polling = useRef<ReturnType<typeof setTimeout> | null>(null);
  const selectedProfile = profiles.find(profile => profile.id === profileId) ?? null;
  const selectedCapability = capabilities.find(capability => capability.modelId === modelId) ?? null;
  const canOperate = Boolean(selectedProfile?.available && selectedProfile.enabled && selectedCapability && (!effort || selectedCapability.efforts.includes(effort)));

  async function read() {
    const id = ++requestId.current;
    setPhase("reading"); setError(null); setNotice(null);
    try {
      const next = await call("providers_list", {});
      if (!mounted.current || id !== requestId.current) return;
      setProfiles(next.profiles); setActive(next.active); setConfigState(next.configState);
      const current = next.profiles.find(profile => profile.id === profileId) ?? next.profiles.find(profile => profile.id === next.active?.profileId) ?? next.profiles[0] ?? null;
      if (current) {
        setProfileId(current.id);
        setForm({ name: current.name, endpoint: current.endpoint, defaultModel: current.defaultModel ?? "", defaultEffort: current.defaultEffort ?? "" });
        const restored = next.active?.profileId === current.id ? next.active : null;
        setModelId(restored?.modelId ?? current.defaultModel ?? "");
        setEffort(restored?.effort ?? current.defaultEffort ?? null);
      } else {
        setProfileId(""); setForm(emptyForm);
      }
      if (!next.profiles.some(profile => profile.id === profileId)) { setCapabilities([]); setModelId(""); setEffort(null); }
      setPhase("ready");
    } catch (cause) { if (mounted.current && id === requestId.current) { setError(toPublicError(cause)); setPhase("failure"); } }
  }

  useEffect(() => { mounted.current = true; void read(); return () => { mounted.current = false; ++requestId.current; if (polling.current) clearTimeout(polling.current); }; }, []);
  useEffect(() => { if (phase === "ready" && !operation) void read(); }, [status?.streamId, status?.sequence]);

  function chooseProfile(id: string) {
    const next = profiles.find(profile => profile.id === id) ?? null;
    const restored = active?.profileId === id ? active : null;
    setProfileId(id); setCapabilities([]); setModelId(restored?.modelId ?? next?.defaultModel ?? ""); setEffort(restored?.effort ?? next?.defaultEffort ?? null); setNotice(null); setError(null);
    setEditingId(null);
    if (next) setForm({ name: next.name, endpoint: next.endpoint, defaultModel: next.defaultModel ?? "", defaultEffort: next.defaultEffort ?? "" });
  }
  function edit<K extends keyof Form>(key: K, value: Form[K]) { setForm(previous => ({ ...previous, [key]: value })); setPhase("editing"); setError(null); }
  function startNew() { setEditingId(null); setProfileId(""); setForm(emptyForm); setCapabilities([]); setModelId(""); setEffort(null); setSecret(""); setNotice(null); setError(null); setPhase("editing"); }
  function startEdit() { if (selectedProfile) { setEditingId(selectedProfile.id); setPhase("editing"); } }

  async function saveProfile(event: React.FormEvent) {
    event.preventDefault();
    if (!form.name.trim() || !form.endpoint.trim() || (form.defaultEffort && !form.defaultModel)) { setError(errorForCode("PA_PROVIDER_SELECTION_INVALID")); return; }
    setPhase("saving"); setError(null); setNotice(null);
    try {
      const saved = await call("provider_save", { profileId: editingId, expectedRevision: editingId ? selectedProfile?.configRevision ?? null : null, name: form.name.trim(), endpoint: form.endpoint.trim(), defaultModel: form.defaultModel || null, defaultEffort: form.defaultEffort || null });
      setProfiles(previous => editingId ? previous.map(profile => profile.id === saved.id ? saved : profile) : [...previous, saved]);
      chooseProfile(saved.id); setNotice("服务源元数据已保存。保存不会证明凭据或连接可用。"); setPhase("ready");
    } catch (cause) { setError(toPublicError(cause)); setPhase("failure"); }
  }

  async function submitSecret(event: React.FormEvent) {
    event.preventDefault();
    if (!selectedProfile || !secret) return;
    const value = secret; setSecret(""); setPhase("saving"); setError(null); setNotice(null);
    try {
      const result = await call("credential_put", { profileId: selectedProfile.id, expectedRevision: selectedProfile.configRevision, secret: value });
      setProfiles(previous => previous.map(profile => profile.id === selectedProfile.id ? { ...profile, configRevision: result.configRevision, credential: result.credential, available: result.credential.status === "configured_unverified" || result.credential.status === "verified", unavailableCode: null } : profile));
      setNotice("凭据已保存到系统凭据库，输入已清空。请刷新模型能力后主动连接测试。"); setPhase("ready");
    } catch (cause) { setError(toPublicError(cause)); setPhase("failure"); }
  }

  async function refreshModels() {
    if (!selectedProfile) return;
    setPhase("saving"); setError(null); setNotice(null);
    try { const result = await call("provider_models", { profileId: selectedProfile.id, expectedRevision: selectedProfile.configRevision, refresh: true }); setCapabilities(result.capabilities); const preferred = selectedProfile.defaultModel && result.capabilities.some(capability => capability.modelId === selectedProfile.defaultModel) ? selectedProfile.defaultModel : result.capabilities[0]?.modelId ?? ""; setModelId(preferred); const cap = result.capabilities.find(value => value.modelId === preferred); setEffort(selectedProfile.defaultEffort && cap?.efforts.includes(selectedProfile.defaultEffort) ? selectedProfile.defaultEffort : null); setNotice(`已读取 ${result.capabilities.length} 个模型能力；能力证据有效期为 24 小时。`); setPhase("ready"); } catch (cause) { setError(toPublicError(cause)); setPhase("failure"); }
  }

  async function removeCredential() {
    if (!selectedProfile) return;
    setPhase("saving"); setError(null); setNotice(null);
    try { const result = await call("credential_remove", { profileId: selectedProfile.id, expectedRevision: selectedProfile.configRevision }); setProfiles(previous => previous.map(profile => profile.id === selectedProfile.id ? { ...profile, configRevision: result.configRevision, credential: result.credential, available: false, unavailableCode: "PA_CREDENTIAL_INVALID" } : profile)); setCapabilities([]); setModelId(""); setEffort(null); setNotice("凭据已从系统凭据库移除；原默认选择不会自动改到其他服务源。"); setPhase("ready"); } catch (cause) { setError(toPublicError(cause)); setPhase("failure"); }
  }

  async function setEnabled(enabled: boolean) {
    if (!selectedProfile) return;
    setPhase("saving"); setError(null);
    try { const result = await call("provider_set_enabled", { profileId: selectedProfile.id, expectedRevision: selectedProfile.configRevision, enabled }); setProfiles(previous => previous.map(profile => profile.id === result.id ? result : profile)); setNotice(enabled ? "服务源已启用。" : "服务源已停用；不会自动切换默认服务源。"); setPhase("ready"); } catch (cause) { setError(toPublicError(cause)); setPhase("failure"); }
  }

  async function deleteProfile() {
    if (!selectedProfile) return;
    setPhase("saving"); setError(null);
    try { await call("provider_delete", { profileId: selectedProfile.id, expectedRevision: selectedProfile.configRevision }); setProfiles(previous => previous.filter(profile => profile.id !== selectedProfile.id)); startNew(); setNotice("服务源已删除。历史快照不会被改写。"); setPhase("ready"); } catch (cause) { setError(toPublicError(cause)); setPhase("failure"); }
  }

  async function startOperation(kind: "test" | "switch") {
    const selection = selectionFrom(selectedProfile, modelId, effort);
    if (!selection || !canOperate || operation) { setError(errorForCode("PA_PROVIDER_SELECTION_INVALID")); return; }
    setPhase("operating"); setError(null); setNotice(kind === "test" ? "连接测试可能产生服务用量；正在等待结果。" : "正在验证候选配置并等待运行中的任务空闲。旧默认值会保留到提交成功。" );
    try { const receipt = await call(kind === "test" ? "provider_test" : "provider_switch", selection); await pollOperation(receipt.operationId); } catch (cause) { setError(toPublicError(cause)); setPhase("failure"); }
  }
  async function pollOperation(operationId: string): Promise<void> {
    const view = await call("operation_get", { operationId });
    if (!mounted.current) return;
    setOperation(view);
    if (!terminal(view)) { polling.current = setTimeout(() => { void pollOperation(operationId); }, 350); return; }
    setPhase("ready");
    if (view.phase === "failed" || view.phase === "cancelled") setError(view.error ? errorForCode(view.error.code) : errorForCode("PA_PROVIDER_TEST_FAILED"));
    else if (view.result && "outcome" in view.result && view.result.outcome !== "applied") setError(errorForCode(view.result.error.code));
    else setNotice(view.result && "testedRevision" in view.result ? "连接测试通过；凭据状态已标记为已验证。" : "默认配置已应用，后续任务将使用新的不可变配置快照。");
    await read();
  }
  async function cancelOperation() { if (!operation || terminal(operation)) return; try { const result = await call("operation_cancel", { operationId: operation.operationId }); setNotice(result.accepted ? "已请求取消，正在确认服务端终态。" : "服务端已进入不可取消阶段，正在确认终态。"); } catch (cause) { setError(toPublicError(cause)); } }

  const operationLabel = operation && !terminal(operation) ? ({ checking: "正在检查服务源", waiting_for_idle: "正在等待任务空闲", applying: "正在应用默认配置", rolling_back: "正在恢复旧配置" } as Record<string, string>)[operation.phase] : null;
  const busy = phase === "reading" || phase === "saving" || phase === "operating";
  const selectedIsActive = Boolean(active && selectedProfile && active.profileId === selectedProfile.id && active.modelId === modelId && active.effort === effort);
  const summary = useMemo(() => configState === "error" ? "恢复失败，已阻止新任务" : configState === "switching" ? "正在切换" : configState === "ready" && active ? "已就绪" : configState === "degraded" ? "需要重新测试" : "未配置", [configState, active]);

  return <div className="page-content ai-config-page">
    <section className="ai-intro"><div><p className="eyebrow">应用自管配置</p><h2>服务源与模型</h2><p>配置只保存在本应用的数据和系统凭据库中。cc-switch 由你在应用外手动管理，本页不会读取、导入或切换它的 profile。</p></div><div className={`config-state config-${configState}`}><span className="status-mark" aria-hidden="true"/><strong>{summary}</strong><small>{active ? `当前：${profiles.find(profile => profile.id === active.profileId)?.name ?? "未知服务源"} · ${active.modelId}` : "没有可用的默认选择"}</small></div></section>
    <div className="ai-layout">
      <section className="ai-panel profile-panel"><div className="section-heading"><div><p className="eyebrow">Profiles</p><h3>服务源</h3></div><button type="button" className="quiet-button" onClick={startNew} disabled={busy}>新增</button></div>
        {profiles.length === 0 ? <p className="empty-inline">还没有应用自管服务源。请添加独立 HTTPS Responses 地址。</p> : <div className="profile-list">{profiles.map(profile => <button type="button" key={profile.id} className={`profile-row${profile.id === profileId ? " selected" : ""}`} onClick={() => chooseProfile(profile.id)} disabled={busy}><span><strong>{profile.name}</strong><small>{profile.endpoint}</small></span><span className={`profile-status ${profile.available && profile.enabled ? "ready" : "muted"}`}>{profile.enabled ? (profile.credential.status === "verified" ? "已验证" : profile.credential.status === "missing" ? "缺少凭据" : "需测试") : "已停用"}</span></button>)}</div>}
        <form className="profile-form" onSubmit={saveProfile}><fieldset disabled={busy}><legend>{editingId ? "编辑服务源" : "服务源元数据"}</legend><label htmlFor="profile-name">名称</label><input id="profile-name" maxLength={128} value={form.name} onChange={event => edit("name", event.target.value)} placeholder="例如：团队中转站"/><label htmlFor="profile-endpoint">Responses 服务地址</label><input id="profile-endpoint" maxLength={2048} value={form.endpoint} onChange={event => edit("endpoint", event.target.value)} placeholder="https://relay.example/v1"/><p className="field-help">仅接受 HTTPS、无认证信息/查询/片段的非官方地址。</p><label htmlFor="profile-default-model">声明默认模型（可选）</label><input id="profile-default-model" maxLength={128} value={form.defaultModel} onChange={event => edit("defaultModel", event.target.value)} placeholder="保存后仍需读取能力并测试"/><label htmlFor="profile-default-effort">声明默认强度（可选）</label><input id="profile-default-effort" maxLength={128} value={form.defaultEffort} onChange={event => edit("defaultEffort", event.target.value)} disabled={!form.defaultModel}/><div className="form-actions"><button className="primary-button" type="submit" disabled={busy || !form.name.trim() || !form.endpoint.trim()}>保存元数据</button>{selectedProfile && <button className="quiet-button" type="button" onClick={startEdit} disabled={busy}>编辑当前</button>}</div></fieldset></form>
      </section>
      <section className="ai-panel selection-panel"><div className="section-heading"><div><p className="eyebrow">Selection</p><h3>能力与默认选择</h3></div><button type="button" className="quiet-button" onClick={() => void refreshModels()} disabled={busy || !selectedProfile}>刷新能力</button></div>
        <ProviderSelection profiles={profiles} capabilities={capabilities} selectedProfileId={profileId} modelId={modelId} effort={effort} disabled={busy || !selectedProfile} onProfileChange={chooseProfile} onModelChange={value => { setModelId(value); const capability = capabilities.find(entry => entry.modelId === value); setEffort(capability?.efforts.includes(effort ?? "") ? effort : null); }} onEffortChange={setEffort}/>
        <div className="selection-actions"><button className="primary-button" type="button" onClick={() => void startOperation("test")} disabled={busy || !canOperate}>连接测试</button><button className="primary-button" type="button" onClick={() => void startOperation("switch")} disabled={busy || !canOperate || selectedIsActive}>设置为默认</button></div>
        <p className="field-help">连接测试只验证当前 profile、模型和强度组合；设置默认会等待空闲并在失败时保留旧配置或明确进入受限状态。</p>
        {selectedProfile && <div className="credential-box"><div><strong>凭据</strong><span>{selectedProfile.credential.status} · 版本 {selectedProfile.credential.revision}</span></div><form onSubmit={submitSecret}><label htmlFor="provider-secret">新增 Key</label><input id="provider-secret" type="password" autoComplete="off" value={secret} onChange={event => setSecret(event.target.value)} placeholder="仅在提交时使用，不会回显" disabled={busy}/><div className="form-actions"><button className="primary-button" type="submit" disabled={busy || !secret}>保存凭据</button>{selectedProfile.credential.status !== "missing" && <button type="button" className="quiet-button" onClick={() => void removeCredential()} disabled={busy}>移除凭据</button>}</div></form><small>凭据提交完成后输入会立即清空；页面、日志和错误不会显示完整 Key 或末尾片段。</small></div>}
        {selectedProfile && <div className="profile-actions"><button type="button" className="quiet-button" onClick={() => void setEnabled(!selectedProfile.enabled)} disabled={busy}>{selectedProfile.enabled ? "停用服务源" : "启用服务源"}</button><button type="button" className="quiet-button danger-button" onClick={() => void deleteProfile()} disabled={busy}>删除服务源</button></div>}
      </section>
    </div>
    <div className="ai-status" role="status" data-testid="ai-config-status"><strong>{phase === "reading" ? "正在读取配置…" : phase === "saving" ? "正在保存…" : phase === "operating" ? (operationLabel ?? "正在处理操作…") : phase === "failure" ? "操作失败" : notice ?? "配置已读取"}</strong>{operation && <span>操作 {operation.operationId} · {operation.phase}<button type="button" className="quiet-button" onClick={() => void pollOperation(operation.operationId)}>重新查询</button>{!terminal(operation) && <button type="button" className="quiet-button" onClick={() => void cancelOperation()}>取消等待</button>}</span>}</div>
    {error && <ErrorNotice error={error} testId="ai-config-error"/>}
    <div className="ai-footnote"><p>官方源和外部登录仅作后续预留。当前页面只管理非官方、自有 HTTPS 服务源；真实任务入口将在阶段 3 接入。</p><button type="button" className="quiet-button" onClick={() => void read()} disabled={busy}>重新读取配置</button></div>
  </div>;
}
