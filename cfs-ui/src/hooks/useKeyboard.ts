import { useEffect } from "react";
import { useAppStore } from "../store";
import { useNavigate } from "react-router-dom";

export default function useKeyboard() {
  const goUp = useAppStore((s) => s.goUp);
  const refresh = useAppStore((s) => s.refresh);
  const lock = useAppStore((s) => s.lock);
  const mount = useAppStore((s) => s.mount);
  const isMounted = useAppStore((s) => s.isMounted);
  const volumeInfo = useAppStore((s) => s.volumeInfo);
  const routerNavigate = useNavigate();

  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      // Ignore if typing in an input
      if (
        e.target instanceof HTMLInputElement ||
        e.target instanceof HTMLTextAreaElement ||
        e.target instanceof HTMLSelectElement
      ) {
        return;
      }

      // Block browser/devtools shortcuts
      if (
        e.key === "F12" ||
        (e.ctrlKey && e.shiftKey && (e.key === "I" || e.key === "i" || e.key === "J" || e.key === "j" || e.key === "C" || e.key === "c")) ||
        (e.ctrlKey && (e.key === "U" || e.key === "u" || e.key === "P" || e.key === "p" || e.key === "S" || e.key === "s"))
      ) {
        e.preventDefault();
        return;
      }

      // Ctrl+L — lock volume
      if (e.ctrlKey && e.key === "l") {
        e.preventDefault();
        if (volumeInfo) {
          lock().then(() => routerNavigate("/"));
        }
        return;
      }

      // Ctrl+R or F5 — refresh (handled here so it refreshes CFS, not the browser)
      if ((e.ctrlKey && e.key === "r") || e.key === "F5") {
        e.preventDefault();
        refresh();
        return;
      }

      // Backspace — go up
      if (e.key === "Backspace") {
        e.preventDefault();
        goUp();
        return;
      }

      // Shift+M — mount volume to default drive letter
      if (e.shiftKey && (e.key === "M" || e.key === "m") && !e.ctrlKey && !e.altKey) {
        e.preventDefault();
        if (volumeInfo && !isMounted) {
          mount();
        }
        return;
      }

      // Escape — lock volume and return to unlock screen
      if (e.key === "Escape") {
        e.preventDefault();
        if (volumeInfo) {
          lock().then(() => routerNavigate("/"));
        }
        return;
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [volumeInfo, isMounted, goUp, refresh, lock, mount, routerNavigate]);
}
