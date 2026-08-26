import { createRootRoute, Outlet, useNavigate } from "@tanstack/react-router";
import { Toaster } from "@/components/ui/sonner";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "@/components/ui/resizable";
import { Settings } from "@/pages/Settings";
import { Documents } from "@/pages/Documents";
import { AppStateProvider, useAppState } from "@/lib/app-state";

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

  function openDocument(name: string) {
    navigate({ to: "/documents/$name", params: { name } });
  }

  const hasDocuments = state.results.length > 0;
  const showSidebar = hasDocuments && !state.isProcessing;

  return (
    <div className="flex min-h-screen flex-col bg-background text-foreground">
      <header className="flex items-center justify-between border-b border-border bg-card px-6 py-3">
        <div className="flex items-center gap-4">
          <button
            type="button"
            className="font-display text-xl font-semibold transition-opacity hover:opacity-80"
            onClick={() => navigate({ to: "/" })}
          >
            Hacienda Studio
          </button>
        </div>
        <button
          type="button"
          className="rounded-md px-3 py-1.5 text-sm text-muted-foreground transition-colors hover:bg-muted/60 hover:text-foreground"
          onClick={openSettings}
        >
          Paramètres
        </button>
      </header>

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

      {showSidebar ? (
        <ResizablePanelGroup orientation="horizontal" className="min-h-0 flex-1">
          <ResizablePanel defaultSize={30} minSize={20} maxSize={45} className="min-h-0 border-r border-border">
            <div className="h-full min-h-0 overflow-auto">
              <Documents
                documents={state.libraryDocuments}
                files={state.files}
                onOpenDocument={openDocument}
                onAddFiles={() => navigate({ to: "/" })}
                onDeleteDocuments={state.handleDeleteDocuments}
              />
            </div>
          </ResizablePanel>
          <ResizableHandle withHandle />
          <ResizablePanel defaultSize={70} className="min-h-0">
            <div className="h-full min-h-0 overflow-auto">
              <Outlet />
            </div>
          </ResizablePanel>
        </ResizablePanelGroup>
      ) : (
        <div className="flex-1">
          <Outlet />
        </div>
      )}

      <Dialog open={!!search.settings} onOpenChange={(open) => (open ? openSettings() : closeSettings())}>
        <DialogContent className="max-h-[85vh] w-[calc(100vw-2rem)] max-w-2xl overflow-x-hidden overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Paramètres</DialogTitle>
          </DialogHeader>
          <Settings config={state.config} onChange={state.setConfig} />
        </DialogContent>
      </Dialog>

      <Toaster />
    </div>
  );
}
