import { createRootRoute, Outlet, useNavigate } from "@tanstack/react-router";
import { Toaster } from "@/components/ui/sonner";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Settings } from "@/pages/Settings";
import { AppStateProvider, useAppState } from "@/lib/app-state";
import { AppShell } from "@/components/layout/AppShell";

type RootSearch = { settings?: boolean };

export const Route = createRootRoute({
  validateSearch: (search: Record<string, unknown>): RootSearch => ({
    settings: search.settings === true || search.settings === "1" || undefined,
  }),
  component: RootComponent,
});

function RootComponent() {
  return (
    <AppStateProvider>
      <RootShell />
    </AppStateProvider>
  );
}

function RootShell() {
  const navigate = useNavigate();
  const search = Route.useSearch();
  const state = useAppState();

  function openSettings() {
    navigate({ to: ".", search: { settings: true } });
  }
  function closeSettings() {
    navigate({ to: ".", search: { settings: undefined } });
  }

  return (
    <AppShell>
      {state.error && (
        <div
          className="error-banner flex items-center justify-between bg-destructive/15 px-6 py-3 text-destructive"
          role="alert"
        >
          <span>❌ {state.error}</span>
          <button aria-label="Fermer" className="text-lg leading-none" onClick={() => state.setError(null)}>
            ✕
          </button>
        </div>
      )}

      {state.skipNotice && (
        <div
          className="skip-notice flex items-center justify-between border-b border-border bg-card px-6 py-3 text-sm text-muted-foreground"
          role="status"
        >
          <span>ℹ️ {state.skipNotice}</span>
          <button
            aria-label="Fermer"
            className="text-lg leading-none"
            onClick={() => state.setSkipNotice(null)}
          >
            ✕
          </button>
        </div>
      )}

      <Outlet />

      <Dialog open={!!search.settings} onOpenChange={(open) => (open ? openSettings() : closeSettings())}>
        <DialogContent className="max-h-[85vh] w-[calc(100vw-2rem)] max-w-2xl overflow-x-hidden overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Paramètres</DialogTitle>
          </DialogHeader>
          <Settings config={state.config} onChange={state.setConfig} />
        </DialogContent>
      </Dialog>

      <Toaster />
    </AppShell>
  );
}
