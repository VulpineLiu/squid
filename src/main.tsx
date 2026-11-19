import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ToastProvider } from "./components/ui";
import { WidgetWindow } from "./pages/WidgetWindow";
import "./styles.css";

// The widget window reuses this bundle with ?view=widget (see widget.rs).
const isWidget = new URLSearchParams(window.location.search).get("view") === "widget";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ToastProvider>{isWidget ? <WidgetWindow /> : <App />}</ToastProvider>
  </React.StrictMode>,
);
