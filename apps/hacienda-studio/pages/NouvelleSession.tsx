import * as React from "react";
import { ChevronDown } from "lucide-react";
import { useNavigate } from "@tanstack/react-router";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { DetectionModal } from "@/components/session/DetectionModal";
import { Dropzone } from "@/components/session/Dropzone";
import { DetectionPanel } from "@/components/session/DetectionPanel";
import { TreatmentPanel } from "@/components/session/TreatmentPanel";
import { ConversionPanel } from "@/components/session/ConversionPanel";
import { ProcessedFilesList } from "@/components/session/ProcessedFilesList";
import { useAppState } from "@/lib/app-state";
import { DETECTION_CATEGORIES } from "@/lib/pii-categories";

export interface NouvelleSessionProps {
  readonly onCancel?: () => void;
}

export function NouvelleSession({ onCancel }: NouvelleSessionProps) {
  const appState = useAppState();

  const detectionSelection = appState?.detectionSelection ?? new Set<string>(["PR","MAIL","PHON","CIE","CID","ACT","ADR","LOC","CP","CARD","IBAN","URL","FILE","REF"]);
  const treatmentMode = appState?.treatmentMode ?? "pseudonymize";
  const conversionMode = appState?.conversionMode ?? "markdown";

  const [showAdvanced, setShowAdvanced] = React.useState(false);
  const navigate = useNavigate();

  function handleFiles(files: File[]): void {
    void appState.handleFilesAccepted(files);
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
            selectedCount={detectionSelection.size}
            totalCount={14}
            onOpenModal={appState.openDetectionModal}
          />
          <DetectionModal
            open={appState?.isDetectionModalOpen ?? false}
            selected={new Set(detectionSelection)}
            onToggle={(code) => { const next = new Set(detectionSelection); next.has(code) ? next.delete(code) : next.add(code); appState.setDetectionSelection(next); }}
            onToggleGroup={(group, all) => { const category = (DETECTION_CATEGORIES.find(g => g.title === group)?.categories ?? []).map(c => c.code); const next = new Set(detectionSelection); category.forEach(c => all ? next.add(c) : next.delete(c)); appState.setDetectionSelection(next); }}
            onDefaults={() => appState.setDetectionSelection(new Set(["PR","MAIL","PHON","CIE","CID","ACT","ADR","LOC","CP","CARD","IBAN","URL","FILE","REF"]))}
            onOpenChange={(open) => {
              appState.setIsDetectionModalOpen(open);
            }}
          />
          <div className="border-t border-slate-100" />
          <TreatmentPanel
            mode={treatmentMode}
            onChange={appState.setTreatmentMode}
          />
          <div className="border-t border-slate-100" />
          <ConversionPanel
            mode={conversionMode}
            onChange={appState.setConversionMode}
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
                    onDropFiles={files => void appState.handleFilesAccepted(files)}
                    onDropFolder={files => void appState.handleFilesAccepted(files)}
                    disabled={false}
                  />
                </div>
              </Card>
              <ProcessedFilesList
                files={appState.results.map((r, index) => {
                  const count = r.frontmatter.piiEntitiesFound;
                  const pagesHint = r.markdown.length > 1000 ? `${String(Math.round(r.markdown.length / 1000))} k caractères` : `${String(r.markdown.length)} caractères`;
                  const subtitle =
                    count === 0
                      ? `${pagesHint} · Aucune occurrence détectée`
                      : count === 1
                        ? `${pagesHint} · 1 occurrence détectée`
                        : `${String(count)} occurrences détectées`.includes("occurrences")
                          ? `${pagesHint} · ${String(count)} occurrences détectées`
                          : `${pagesHint} · ${String(count)} occurrences détectées`;
                  const prefix = appState.treatmentMode === "pseudonymize" ? "[PSEUDONYMISÉ] " : "";
                  return {
                    id: `${String(index)}-${r.name}`,
                    name: `${prefix}${r.name}`,
                    status: subtitle,
                  };
                })}
                onView={(name) => {
                  const clean = name.replace(/^\[PSEUDONYMISÉ\] /, "");
                  navigate({ to: "/documents/$name", params: { name: clean } });
                }}
                onDelete={(name) => {
                  const clean = name.replace(/^\[PSEUDONYMISÉ\] /, "");
                  appState.handleDeleteDocuments([clean]);
                }}
              />
            </div>
          ) : (
            <Dropzone
              onDropFiles={files => void appState.handleFilesAccepted(files)}
              onDropFolder={files => void appState.handleFilesAccepted(files)}
              disabled={false}
            />
          )}
        </div>
      </div>
    </div>
  );
}
