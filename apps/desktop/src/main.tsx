import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./ui/theme/tokens.css";
import "./styles/base.css";
import "./styles/primitives.css";
import "./styles/chrome.css";
import "./styles/launcher.css";
import "./styles/workbench.css";
import "./styles/editor.css";
import "./styles/git.css";
import "./styles/overlays.css";

const root = document.getElementById("root");

if (!root) {
  throw new Error("Aeria root element is missing");
}

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
