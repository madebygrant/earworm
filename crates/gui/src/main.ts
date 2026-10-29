// Bundled with the app rather than fetched: a desktop app has to look right offline.
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/700.css";
// The mockups' stylesheets are the app's, so the two cannot drift; core's palette repaints the colours.
import "../../../docs/mockups/tokens.css";
import "../../../docs/mockups/base.css";
import "./app.css";
import "./screens.css";
import { mount } from "svelte";
import App from "./App.svelte";

if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
  const { previewState } = await import("./preview");
  const { ui } = await import("./lib/ui.svelte");
  ui.quitting = previewState.quit;
  // A handle for putting the page on a screen no message reaches, such as an artist's page, to capture it.
  const { screens } = await import("./lib/screens.svelte");
  const { app } = await import("./lib/store.svelte");
  Object.assign(window, { earworm: { app, ui, screens } });
  ui.logs = previewState.logs;
  if (previewState.mode) ui.mode = previewState.mode;
}
mount(App, { target: document.body });
