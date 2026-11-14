import React from "react";
import ReactDOM from "react-dom/client";
import "./i18n";
import "./styles.css";
import App from "./App";
import { isTauri } from "./lib/api";

// In the native app the window is transparent and the rounded shell is the window.
if (isTauri) document.documentElement.classList.add("native");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
