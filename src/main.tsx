import React from "react";
import ReactDOM from "react-dom/client";
import Bootstrap, { AppBoundary } from "./app/Bootstrap";
import OverlayPage from "./pages/OverlayPage";
import { isOverlayWindow } from "./ipc/client";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <AppBoundary>{isOverlayWindow() ? <OverlayPage /> : <Bootstrap />}</AppBoundary>
  </React.StrictMode>,
);
