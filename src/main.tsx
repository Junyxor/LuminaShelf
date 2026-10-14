import React from "react";
import ReactDOM from "react-dom/client";
import App from "./ShelfApp";
import "./paper.css";
import "./network.css";
import "./account-light.css";
import "./reading.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
