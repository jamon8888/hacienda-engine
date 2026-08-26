import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { Landing } from "@/pages/Landing";
import { Studio } from "@/pages/Studio";
import { useAppState } from "@/lib/app-state";

export const Route = createFileRoute("/")({
  component: HomePage,
});

function HomePage() {
  const navigate = useNavigate();
  const state = useAppState();

  return (
    <div className="flex flex-1 flex-col">
      <Studio
        workerReady={state.workerReady}
        folderMode={state.folderMode}
        onToggleFolderMode={state.toggleFolderMode}
        onFilesAccepted={state.handleFilesAccepted}
        pendingFiles={state.pendingFiles}
        onRemovePending={state.handleRemovePending}
        onClearPending={state.handleClearPending}
        onProcessQueue={state.handleProcessQueue}
        files={state.files}
        progress={state.progress}
        results={state.results}
        fileErrors={state.fileErrors}
        onOpenDocument={(name) => navigate({ to: "/documents/$name", params: { name } })}
        config={state.config}
        onConfigChange={state.setConfig}
        onOpenSettings={() => navigate({ to: ".", search: { settings: true } })}
        assets={state.assets}
        nerModelProgress={state.nerModelProgress}
        nerModelDegraded={state.nerModelDegraded}
      />
      <Landing />
    </div>
  );
}
