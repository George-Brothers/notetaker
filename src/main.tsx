import React from "react";
import ReactDOM from "react-dom/client";
import "./styles/theme.css";
import "./styles/panels.css";
import App from "./App";
import { applyThemeParam } from "./lib/themeParam";

applyThemeParam(window.location.search, window.localStorage);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
