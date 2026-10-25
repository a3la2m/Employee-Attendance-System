import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ConfirmProvider } from "./components/Confirm";
import "./index.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {/* مزوّد التأكيد «نعم/لا» يغطي التطبيق كله — تُستعمل بدل window.confirm */}
    <ConfirmProvider>
      <App />
    </ConfirmProvider>
  </React.StrictMode>,
);
