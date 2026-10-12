import type { ReactNode } from "react";
export function StatePanel({ kind, title, children, actions }: {
    kind: "loading" | "empty" | "unavailable" | "error";
    title: string;
    children: ReactNode;
    actions?: ReactNode;
}) {
    return <section className={`state-panel state-${kind}`} data-testid={`state-${kind}`} role={kind === "error" ? "alert" : "status"} aria-busy={kind === "loading"}>
    <div className="state-symbol" aria-hidden="true">{kind === "loading" ? "…" : kind === "error" ? "!" : "○"}</div>
    <h2>{title}</h2><div className="state-detail">{children}</div>
    {actions && <div className="state-actions">{actions}</div>}
  </section>;
}
