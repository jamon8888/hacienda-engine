import * as React from "react";
import { ChevronDown } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { DetectionModal } from "@/components/session/DetectionModal";
import { Dropzone } from "@/components/session/Dropzone";
import { DetectionPanel } from "@/components/session/DetectionPanel";
import { TreatmentPanel } from "@/components/session/TreatmentPanel";
import { ConversionPanel } from "@/components/session/ConversionPanel";
import { ProcessedFilesList } from "@/components/session/ProcessedFilesList";
import { useAppState } from "@/lib/app-state";

export interface NouvelleSessionProps {
  readonly onCancel?: () => void;
}

export function NouvelleSession({ onCancel }: NouvelleSessionProps) {
  let appState: ReturnType<typeof useAppState> | null = null;
  try {
    appState = useAppState();
  } catch {
    appState = null;
  }

  const detectionSelection = appState?.detectionSelection ?? new Set<string>(["PR","MAIL","PHON","CIE","CID","ACT","ADR","LOC","CP","CARD","IBAN","URL","FILE","REF"]);
  const treatmentMode = appState?.treatmentMode ?? "pseudonymize";
  const conversionMode = appState?.conversionMode ?? "markdown";

  const [showAdvanced, setShowAdvanced] = React.useState(false);

  function handleFiles(files: File[]): void {
    if (appState) {
      void appState.handleFilesAccepted(files);
    }
  }

  return (
    <div className="flex flex-1 flex-col bg-[#f8fafc] p-6">
      {/* Header */}
      <div className="mb-6 flex items-center justify-between">
        <h1 className="text-xl font-semibold text-slate-900">Nouvelle session</h1>
        <Button variant="ghost" size="sm" onClick={onCancel}>
          Annuler
        </Button>
      </div>

      {/* Content */}
      <div className="flex flex-1 gap-6">
        {/* Left panel 340px */}
        <Card className="flex w-[340px] shrink-0 flex-col bg-white p-4">
          <DetectionPanel
            selection={detectionSelection}
            onOpenModal={appState?.openDetectionModal}
          />
          <DetectionModal
            open={appState?.isDetectionModalOpen ?? false}
            selection={detectionSelection}
            onChange={appState ? appState.setDetectionSelection : () => {}}
            onOpenChange={(open) => {
              if (appState) appState.setIsDetectionModalOpen(open);
            }}
          />
          <div className="border-t border-slate-100" />
          <TreatmentPanel
            mode={treatmentMode}
            onChange={appState ? appState.setTreatmentMode : () => {}}
          />
          <div className="border-t border-slate-100" />
          <ConversionPanel
            mode={conversionMode}
            onChange={appState ? appState.setConversionMode : () => {}}
          />
          <div className="mt-auto border-t border-slate-100 pt-4">
            <button
              type="button"
              onClick={() => setShowAdvanced((v) => !v)}
              className="flex w-full items-center justify-between text-sm text-slate-600 hover:text-slate-900"
            >
              <span>Afficher les réglages avancés</span>
              <ChevronDown
                size={16}
                className={`transition-transform ${showAdvanced ? "rotate-180" : ""}`}
              />
            </button>
            {showAdvanced ? (
              <div className="mt-3 rounded-md bg-slate-50 p-3 text-xs text-slate-500">
                Réglages avancés à venir (Task 4).
              </div>
            ) : null}
          </div>
        </Card>

        {/* Right — either dropzone or processed files list */}
        <div className="flex flex-1 flex-col">
          {appState && appState.results.length > 0 ? (
            <div className="flex flex-col gap-6">
              <Card className="border border-dashed border-slate-200 bg-white p-4">
                <p className="text-sm font-medium text-slate-900">Ajouter des documents</p>
                <p className="text-xs text-slate-500">Cliquez ou glissez-déposez des fichiers</p>
                <p className="mt-2 text-xs text-slate-400">Formats supportés : PDF, DOCX, XLSX, PPTX, images et textes (TXT, JSON, FEC, etc.)</p>
                <div className="mt-3">
                  <Dropzone
                    onFiles={handleFiles}
                    folderMode={appState.folderMode}
                    onToggleFolderMode={appState.toggleFolderMode}
                  />
                </div>
              </Card>
              <ProcessedFilesList
                files={appState.results.map((r) => {
                  const count = r.frontmatter.piiEntitiesFound;
                  const pagesHint = r.markdown.length > 1000 ? `${Math.round(r.markdown.length / 1000)} k caractères` : `${String(r.markdown.length)} caractères`;
                  // Heuristic: if markdown contains many newlines, approximate pages
                  const subtitle =
                    count === 0
                      ? `${pagesHint} · Aucune occurrence détectée`
                      : count === 1
                        ? `${pagesHint} · 1 occurrence détectée`
                        : `${pagesHint} · ${String(count)} occurrences détectées`;
                  const prefix = appState.treatmentMode === "pseudonymize" ? "[PSEUDONYMISÉ] " : "";
                  return {
                    name: `${prefix}${r.name}`,
                    subtitle,
                    pseudonymized: appState.treatmentMode === "pseudonymize",
                  };
                })}
                onView={(name) => {
                  // name may have prefix — strip it to find original
                  const clean = name.replace(/^\[PSEUDONYMISÉ\] /, "");
                  window.location.hash = `#/documents/${encodeURIComponent(clean)}`;
                  // fallback navigation via router if available
                  const nav = (window as unknown as { __navigate?: (to: string) => void }).__navigate;
                  if (nav) nav(`/documents/${encodeURIComponent(clean)}`);
                  else window.location.href = `/documents/${encodeURIComponent(clean)}`;
                }}
                onDelete={(name) => {
                  const clean = name.replace(/^\[PSEUDONYMISÉ\] /, "");
                  appState.handleDeleteDocuments([clean]);
                }}
                onAddFiles={() => {
                  // re-use dropzone click — no extra logic
                }}
              />
            </div>
          ) : (
            <Dropzone
              onFiles={handleFiles}
              folderMode={appState?.folderMode ?? false}
              onToggleFolderMode={appState?.toggleFolderMode}
            />
          )}
        </div>
      </div>
    </div>
  );
}
