import React from "react";
import ReactDOM from "react-dom/client";
import App from "./ShelfApp";
import "./styles/base.css";
import "./styles/shell.css";
import "./styles/books.css";
import "./styles/downloads.css";
import "./styles/settings.css";
import "./styles/reader.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
