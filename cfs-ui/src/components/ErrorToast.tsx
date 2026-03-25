import * as Toast from "@radix-ui/react-toast";
import { useAppStore } from "../store";

export default function ErrorToast() {
  const error = useAppStore((s) => s.error);
  const clearError = useAppStore((s) => s.clearError);

  return (
    <Toast.Provider duration={5000}>
      <Toast.Root
        open={!!error}
        onOpenChange={(open) => {
          if (!open) clearError();
        }}
        className="fixed top-12 right-3 z-50 border border-error bg-surface px-4 py-2 text-sm text-error opacity-0 data-[state=open]:opacity-100 transition-opacity duration-150"
      >
        <Toast.Description>{error}</Toast.Description>
      </Toast.Root>
      <Toast.Viewport />
    </Toast.Provider>
  );
}
