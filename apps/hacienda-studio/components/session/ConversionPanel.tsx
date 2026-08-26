import { Sparkles, HelpCircle } from "lucide-react";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";

export interface ConversionPanelProps {
  readonly mode: "original" | "markdown";
  readonly onChange: (mode: "original" | "markdown") => void;
}

export function ConversionPanel({ mode, onChange }: ConversionPanelProps) {
  return (
    <div className="flex flex-col gap-3 py-4">
      <div className="flex items-center gap-1.5">
        <span className="text-sm font-medium text-slate-900">Conversion .pdf, .jpg, .png</span>
        <TooltipProvider>
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                aria-label="Aide conversion"
                className="rounded-full p-0.5 text-slate-400 hover:text-slate-600"
              >
                <HelpCircle size={14} />
              </button>
            </TooltipTrigger>
            <TooltipContent side="top">Choisir le format de sortie</TooltipContent>
          </Tooltip>
        </TooltipProvider>
      </div>

      <div className="flex rounded-lg bg-slate-100 p-1">
        <button
          type="button"
          onClick={() => onChange("original")}
          className={`flex flex-1 items-center justify-center rounded-md px-3 py-2 text-sm font-medium transition-colors ${
            mode === "original"
              ? "border border-slate-200 bg-white text-slate-900 shadow-sm"
              : "bg-transparent text-slate-500 hover:text-slate-700"
          }`}
        >
          Format original
        </button>
        <button
          type="button"
          onClick={() => onChange("markdown")}
          className={`flex flex-1 items-center justify-center gap-1.5 rounded-md px-3 py-2 text-sm font-medium transition-colors ${
            mode === "markdown"
              ? "border border-slate-200 bg-white text-slate-900 shadow-sm"
              : "bg-transparent text-slate-500 hover:text-slate-700"
          }`}
        >
          <Sparkles size={14} />
          Format Markdown
        </button>
      </div>
    </div>
  );
}
