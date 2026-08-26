import { Settings, HelpCircle } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";

export interface DetectionPanelProps {
  readonly selection: ReadonlySet<string>;
  readonly onOpenModal?: () => void;
}

export function DetectionPanel({ selection, onOpenModal }: DetectionPanelProps) {
  const count = selection.size;

  return (
    <div className="flex items-center justify-between py-4">
      <div className="flex items-center gap-1.5">
        <span className="text-sm font-medium text-slate-900">Détection</span>
        <TooltipProvider>
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                aria-label="Aide détection"
                className="rounded-full p-0.5 text-slate-400 hover:text-slate-600"
              >
                <HelpCircle size={14} />
              </button>
            </TooltipTrigger>
            <TooltipContent side="top">Choisir les catégories de données à détecter</TooltipContent>
          </Tooltip>
        </TooltipProvider>
      </div>

      <div className="relative">
        <Button variant="outline" size="sm" onClick={onOpenModal} className="gap-1.5 bg-white">
          <Settings size={14} />
          Sélectionner les données
        </Button>
        <Badge className="absolute -right-2 -top-2 flex h-5 min-w-5 items-center justify-center rounded-full bg-violet-600 px-1 text-[10px] leading-none text-white hover:bg-violet-600">
          {String(count)}
        </Badge>
      </div>
    </div>
  );
}
