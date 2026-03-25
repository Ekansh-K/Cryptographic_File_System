import { useAppStore } from "../store";

export default function Breadcrumb() {
  const currentPath = useAppStore((s) => s.currentPath);
  const navigate = useAppStore((s) => s.navigate);

  const segments = currentPath.split("/").filter(Boolean);

  return (
    <div className="flex items-center gap-1 px-3 py-1.5 text-sm overflow-x-auto min-w-0">
      <button
        className="text-text-muted hover:text-text-bright shrink-0"
        onClick={() => navigate("/")}
      >
        /
      </button>
      {segments.map((seg, i) => {
        const path = "/" + segments.slice(0, i + 1).join("/");
        const isLast = i === segments.length - 1;
        return (
          <span key={path} className="flex items-center gap-1 min-w-0">
            <span className="text-text-muted shrink-0">&gt;</span>
            {isLast ? (
              <span className="text-text-bright truncate">{seg}</span>
            ) : (
              <button
                className="text-text-muted hover:text-text-bright truncate"
                onClick={() => navigate(path)}
              >
                {seg}
              </button>
            )}
          </span>
        );
      })}
    </div>
  );
}
