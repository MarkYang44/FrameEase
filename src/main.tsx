import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { useStore } from "./lib/store";
import "./styles.css";

// Dev builds expose the store for automated UI checks (never in production bundles).
if (import.meta.env.DEV) (window as unknown as { __fe: typeof useStore }).__fe = useStore;

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
