import { ProbeError } from "./client.mjs";

// Phase-0 capability contract experiment: metadata in snapshots, secrets only in a resolver.
export function profileSnapshot(profile, credentialMetadata, selection, { allowLoopback = false } = {}) {
  let endpoint;
  try { endpoint = new URL(profile.endpoint); } catch { throw new ProbeError("PROVIDER_ENDPOINT_INVALID"); }
  const localTest = allowLoopback && endpoint.protocol === "http:" && endpoint.hostname === "127.0.0.1";
  if ((!localTest && endpoint.protocol !== "https:") || endpoint.username || endpoint.password || endpoint.search || endpoint.hash) {
    throw new ProbeError("PROVIDER_ENDPOINT_INVALID");
  }
  if (credentialMetadata.profileId !== profile.id || credentialMetadata.origin !== endpoint.origin) {
    throw new ProbeError("KEY_SITE_MISMATCH");
  }
  return Object.freeze({ profileId: profile.id, endpoint: endpoint.href, credentialReference: credentialMetadata.reference,
    model: selection.model, effort: selection.effort });
}
