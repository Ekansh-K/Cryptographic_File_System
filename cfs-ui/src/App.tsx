import { Outlet } from "react-router-dom";
import Titlebar from "./components/Titlebar";
import StatusBar from "./components/StatusBar";
import ErrorToast from "./components/ErrorToast";

export default function App() {
  return (
    <div
      className="flex flex-col h-full bg-bg text-text font-mono"
      onContextMenu={(e) => e.preventDefault()}
    >
      <Titlebar />
      <main className="flex-1 min-h-0 overflow-hidden">
        <Outlet />
      </main>
      <StatusBar />
      <ErrorToast />
    </div>
  );
}
