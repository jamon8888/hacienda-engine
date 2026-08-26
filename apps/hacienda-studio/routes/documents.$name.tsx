import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { DocumentDetail } from "@/pages/DocumentDetail";
import { useAppState } from "@/lib/app-state";

export const Route = createFileRoute("/documents/$name")({
  component: DocumentDetailPage,
});

function DocumentDetailPage() {
  const { name } = Route.useParams();
  const navigate = useNavigate();
  const state = useAppState();

  const result = state.findResultByName(name);

  // No router loader here on purpose: the document library lives in `useAppState()`'s
  // React context (see lib/app-state.tsx), not router context — a loader runs outside the
  // component tree and can't read it. A missing document (stale link, or IndexedDB
  // hydration still in flight right after a hard refresh) renders inline instead of a
  // hard `notFound()`.
  if (!result) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 p-10 text-center">
        <p className="text-sm font-medium">Document introuvable</p>
        <p className="max-w-sm text-xs leading-relaxed text-muted-foreground">
          « {name} » n'existe pas dans cette bibliothèque, ou n'a pas encore fini de se charger.
        </p>
        <button
          type="button"
          className="mt-2 rounded-md bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground hover:bg-primary/90"
          onClick={() => navigate({ to: "/" })}
        >
          Retour à l'accueil
        </button>
      </div>
    );
  }

  const findings = state.findingsFor(result);
  const originalFile = state.findOriginalFile(result);

  return (
    <DocumentDetail
      result={result}
      findings={findings}
      originalFile={originalFile}
      onBack={() => navigate({ to: "/" })}
      onAddFinding={(start, end, category) => state.handleAddFinding(result, start, end, category)}
      onRemoveFinding={(i) => state.handleRemoveFinding(result, i)}
      onExportBody={(body) => state.exportBody(result, body)}
      onDelete={() => {
        state.handleDeleteDocuments([result.name]);
        navigate({ to: "/" });
      }}
    />
  );
}
