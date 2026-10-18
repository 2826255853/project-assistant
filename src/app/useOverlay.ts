import { useAppStatus } from "./useAppStatus";

export function useOverlay() {
  return useAppStatus().status?.overlay ?? null;
}
