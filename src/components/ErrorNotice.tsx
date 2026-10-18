import type { ReactNode } from "react";
import { toPublicError } from "../ipc/errors";
import "./ErrorNotice.css";

export function ErrorNotice({ error, actions, testId }: { error: unknown; actions?: ReactNode; testId?: string }) {
  const safe = toPublicError(error);
  return <div className="error-notice" role="alert" data-testid={testId}>
    <p><strong>{safe.code}</strong>：{safe.userMessage}</p>
    <p>{safe.suggestion}</p>
    <p className="error-correlation">关联 ID：{safe.correlationId ?? "未提供（本地提示）"}</p>
    {actions && <div className="error-actions">{actions}</div>}
  </div>;
}
