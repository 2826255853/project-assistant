import { useMemo } from "react";
import type { AiSelection, ModelCapability, ProviderProfile } from "../ipc/contracts";

type Props = {
  profiles: ProviderProfile[];
  capabilities: ModelCapability[];
  selectedProfileId: string;
  modelId: string;
  effort: string | null;
  disabled?: boolean;
  onProfileChange: (profileId: string) => void;
  onModelChange: (modelId: string) => void;
  onEffortChange: (effort: string | null) => void;
};

export function ProviderSelection({
  profiles,
  capabilities,
  selectedProfileId,
  modelId,
  effort,
  disabled = false,
  onProfileChange,
  onModelChange,
  onEffortChange,
}: Props) {
  const selected = profiles.find(profile => profile.id === selectedProfileId) ?? null;
  const models = useMemo(() => capabilities, [capabilities]);
  const selectedCapability = models.find(capability => capability.modelId === modelId) ?? null;
  const efforts = selectedCapability?.efforts ?? [];
  const effectiveEffort = effort && efforts.includes(effort) ? effort : null;
  return <fieldset className="provider-selection" disabled={disabled}>
    <legend>本次 AI 选择</legend>
    <label htmlFor="provider-select">服务源</label>
    <select id="provider-select" value={selectedProfileId} onChange={event => onProfileChange(event.target.value)}>
      <option value="">选择服务源</option>
      {profiles.map(profile => <option key={profile.id} value={profile.id} disabled={!profile.available || !profile.enabled}>
        {profile.name}{profile.credential.status === "verified" ? " · 已验证" : profile.credential.status === "missing" ? " · 未配置凭据" : " · 需测试"}
      </option>)}
    </select>
    <p className="field-help">{selected ? `${selected.endpoint} · 凭据版本 ${selected.credential.revision}` : "仅显示应用内自管的 HTTPS 服务源；不会读取 cc-switch。"}</p>
    <label htmlFor="model-select">模型</label>
    <select id="model-select" value={modelId} onChange={event => onModelChange(event.target.value)} disabled={!selectedProfileId || !models.length}>
      <option value="">选择模型</option>
      {models.map(capability => <option key={capability.modelId} value={capability.modelId}>{capability.modelId}</option>)}
    </select>
    <label htmlFor="effort-select">思考强度</label>
    <select id="effort-select" value={effectiveEffort ?? ""} onChange={event => onEffortChange(event.target.value || null)} disabled={!selectedCapability}>
      <option value="">使用模型默认值</option>
      {efforts.map(value => <option key={value} value={value}>{value}</option>)}
    </select>
    <p className="field-help">模型或能力列表变化后，强度会重新校验；不支持的组合不会提交。</p>
  </fieldset>;
}

export function selectionFrom(profile: ProviderProfile | null, modelId: string, effort: string | null): AiSelection | null {
  if (!profile || !modelId) return null;
  return { profileId: profile.id, expectedRevision: profile.configRevision, modelId, effort };
}
