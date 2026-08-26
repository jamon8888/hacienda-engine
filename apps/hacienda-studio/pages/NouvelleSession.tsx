import * as React from "react";
import { ChevronDown } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { DetectionModal } from "@/components/session/DetectionModal";
import { Dropzone } from "@/components/session/Dropzone";
import { DetectionPanel } from "@/components/session/DetectionPanel";
import { TreatmentPanel } from "@/components/session/TreatmentPanel";
import { ConversionPanel } from "@/components/session/ConversionPanel";
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

        {/* Right dropzone */}
        <div className="flex flex-1 flex-col">
          <Dropzone onFiles={handleFiles} />
        </div>
      </div>
    </div>
  );
}
