import React from "react";
import ReactDOM from "react-dom/client";
import "./styles/theme.css";
import "./styles/panels.css";
import "./styles/layout.css";
import App from "./App";
import { Overlay } from "./components/Overlay";
import { applyThemeParam } from "./lib/themeParam";

applyThemeParam(window.location.search, window.localStorage);

// One bundle, two windows. The overlay webview loads the same index.html with
// this hash; it renders the floating pill and nothing else — no library, no
// runtime polling, no capture flow. All of that stays in the main window,
// which drives the overlay over events.
const isOverlay = window.location.hash === "#overlay";
if (isOverlay) {
  document.documentElement.dataset.surface = "floating";
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {isOverlay ? <Overlay /> : <App />}
  </React.StrictMode>,
);
