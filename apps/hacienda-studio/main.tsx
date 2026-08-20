import "./app.css";
import { createRoot } from "react-dom/client";
import { App } from "./App";

// Redesign: the app ships dark-only (see `.dark` block in app.css) — no toggle exists,
// so the class is applied once here rather than via a theme-context nobody reads yet.
document.documentElement.classList.add("dark");

// No <StrictMode>: it double-invokes effects in dev, which would create and tear down
// the worker (a 48 MB WASM compile) once before the "real" mount — real cost for no
// benefit here, since the effect below has no side effect StrictMode would help surface.
createRoot(document.getElementById("app")!).render(<App />);
