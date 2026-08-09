// Classic shadcn/ui recipe (cva + Tailwind), not ported from hacienda-private's own
// badge.tsx — that file is built on @base-ui/react + Tailwind v4 arbitrary-value
// functions, a different toolkit generation than this app's Tailwind v3 (see
// components/ui/README.md for why).
import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

const badgeVariants = cva(
  "inline-flex items-center rounded-full border font-semibold transition-colors focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2",
  {
    variants: {
      variant: {
        default: "border-transparent bg-primary text-primary-foreground hover:bg-primary/80",
        secondary:
          "border-transparent bg-secondary text-secondary-foreground hover:bg-secondary/80",
        destructive:
          "border-transparent bg-destructive text-destructive-foreground hover:bg-destructive/80",
        outline: "text-foreground",
        // Extend UI's vendored viewer components (docx-annotation-card.tsx) use these for
        // tracked-change/comment status badges — this shadcn recipe otherwise has no
        // success/error/warning/info colorway. ~keep
        success: "border-transparent bg-emerald-600 text-white hover:bg-emerald-600/80",
        error: "border-transparent bg-destructive text-destructive-foreground hover:bg-destructive/80",
        warning: "border-transparent bg-amber-500 text-white hover:bg-amber-500/80",
        info: "border-transparent bg-sky-600 text-white hover:bg-sky-600/80",
      },
      size: {
        default: "px-2.5 py-0.5 text-xs",
        sm: "px-2 py-0 text-[11px]",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
);

export interface BadgeProps
  extends React.HTMLAttributes<HTMLDivElement>,
    VariantProps<typeof badgeVariants> {}

function Badge({ className, variant, size, ...props }: BadgeProps) {
  return <div className={cn(badgeVariants({ variant, size }), className)} {...props} />;
}

export { Badge, badgeVariants };
