import { useState } from "react";
import Breadcrumb from "../components/Breadcrumb";
import Sidebar from "../components/Sidebar";
import FileList from "../components/FileList";
import FileViewer from "../components/FileViewer";
import useKeyboard from "../hooks/useKeyboard";

export default function BrowserScreen() {
  useKeyboard();

  const [focusedPanel, setFocusedPanel] = useState<"sidebar" | "list" | "preview">("list");

  return (
    <div className="flex flex-col h-full min-h-0">
      {/* Breadcrumb bar */}
      <div className="border-b border-border shrink-0">
        <Breadcrumb />
      </div>

      {/* Three-column layout */}
      <div className="flex-1 grid grid-cols-[200px_1fr_300px] min-h-0">
        {/* Sidebar */}
        <div
          className={`border-r min-h-0 overflow-hidden ${
            focusedPanel === "sidebar" ? "border-border-focus" : "border-border"
          }`}
          onClick={() => setFocusedPanel("sidebar")}
        >
          <Sidebar />
        </div>

        {/* File list */}
        <div
          className={`border-r min-h-0 overflow-hidden ${
            focusedPanel === "list" ? "border-border-focus" : "border-border"
          }`}
          onClick={() => setFocusedPanel("list")}
        >
          <FileList />
        </div>

        {/* Preview panel */}
        <div
          className={`min-h-0 overflow-hidden ${
            focusedPanel === "preview" ? "border-l border-border-focus" : ""
          }`}
          onClick={() => setFocusedPanel("preview")}
        >
          <FileViewer />
        </div>
      </div>
    </div>
  );
}
