import React from "react";
import ReactDOM from "react-dom/client";
import "./styles/theme.css";
import "./styles/panels.css";
import App from "./App";
import { Overlay } from "./components/Overlay";
import { TrayPanel } from "./components/TrayPanel";
import { applyThemeParam } from "./lib/themeParam";

applyThemeParam(window.location.search, window.localStorage);

// One bundle, two windows. The overlay webview loads the same index.html with
// this hash; it renders the floating pill and nothing else — no library, no
// runtime polling, no capture flow. All of that stays in the main window,
// which drives the overlay over events.
const isOverlay = window.location.hash === "#overlay";
const isTrayPanel = window.location.hash === "#tray-panel";
if (isOverlay || isTrayPanel) {
  document.documentElement.dataset.surface = "floating";
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {isOverlay ? <Overlay /> : isTrayPanel ? <TrayPanel /> : <App />}
  </React.StrictMode>,
);
