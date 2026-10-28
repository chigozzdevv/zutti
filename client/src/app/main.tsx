import React from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app.js";
import "@fontsource-variable/dm-sans/wght.css";
import "./styles.css";

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
