import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./ui/theme/tokens.css";
import "./styles.css";

const root = document.getElementById("root");

if (!root) {
  throw new Error("Aeria root element is missing");
}

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
