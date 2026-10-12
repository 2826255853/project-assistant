import { useEffect, useState } from "react";
import { getOverlay, listenOverlay } from "../ipc/client";
import type { OverlayView } from "../ipc/contracts";

export function useOverlay() {
  const [view, setView] = useState<OverlayView | null>(null);
  useEffect(() => {
    let active = true;
    let cleanup: (() => void) | undefined;
    void listenOverlay((state) => { if (active) setView(state); }).then(async (unlisten) => {
      if (!active) { unlisten(); return; }
      cleanup = unlisten;
      const state = await getOverlay();
      if (active) setView(state);
    }).catch(() => { /* The page presents a fixed unavailable state. */ });
    return () => { active = false; cleanup?.(); };
  }, []);
  return view;
}
