import { KeyRound, EyeOff } from "lucide-react";

export interface TreatmentPanelProps {
  readonly mode: "pseudonymize" | "anonymize";
  readonly onChange: (mode: "pseudonymize" | "anonymize") => void;
}

export function TreatmentPanel({ mode, onChange }: TreatmentPanelProps) {
  return (
    <div className="flex flex-col gap-3 py-4">
      <span className="text-sm font-medium text-slate-900">Traitement</span>
      <div className="flex rounded-lg bg-slate-100 p-1">
        <button
          type="button"
          onClick={() => onChange("pseudonymize")}
          className={`flex flex-1 items-center justify-center gap-1.5 rounded-md px-3 py-2 text-sm font-medium transition-colors ${
            mode === "pseudonymize"
              ? "border border-slate-200 bg-white text-slate-900 shadow-sm"
              : "bg-transparent text-slate-500 hover:text-slate-700"
          }`}
        >
          <KeyRound size={14} />
          Pseudonymiser
        </button>
        <button
          type="button"
          onClick={() => onChange("anonymize")}
          className={`flex flex-1 items-center justify-center gap-1.5 rounded-md px-3 py-2 text-sm font-medium transition-colors ${
            mode === "anonymize"
              ? "border border-slate-200 bg-white text-slate-900 shadow-sm"
              : "bg-transparent text-slate-500 hover:text-slate-700"
          }`}
        >
          <EyeOff size={14} />
          Anonymiser
        </button>
      </div>
    </div>
  );
}
