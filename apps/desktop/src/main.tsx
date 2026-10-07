import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { isBrowserCommand } from "./shortcuts";
import "./ui/theme/tokens.css";
import "./styles/base.css";
import "./styles/primitives.css";
import "./styles/chrome.css";
import "./styles/launcher.css";
import "./styles/workbench.css";
import "./styles/editor.css";
import "./styles/git.css";
import "./styles/search.css";
import "./styles/overlays.css";

// The web view's own commands (print, reload, find, going back) never run.
// On the window, so Aeria's shortcuts, which listen lower, act first.
window.addEventListener("keydown", (event) => {
  if (isBrowserCommand(event, import.meta.env.DEV)) event.preventDefault();
});

const root = document.getElementById("root");

if (!root) {
  throw new Error("Aeria root element is missing");
}

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
