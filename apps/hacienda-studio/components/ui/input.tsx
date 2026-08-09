// Classic shadcn/ui recipe, not ported from hacienda-private's own input.tsx (built on
// @base-ui/react's Input primitive) — see components/ui/README.md.
import * as React from "react";

import { cn } from "@/lib/utils";

// Native `size` is a numeric HTML attribute (character width); Extend UI's viewer toolbars
// pass shadcn's string size scale instead (`"sm" | "default"`) — shadowed here rather than
// forwarded to the DOM element. ~keep
export interface InputProps
  extends Omit<React.InputHTMLAttributes<HTMLInputElement>, "size"> {
  size?: "default" | "sm";
}

const Input = React.forwardRef<HTMLInputElement, InputProps>(
  ({ className, type, size = "default", ...props }, ref) => {
    return (
      <input
        type={type}
        className={cn(
          "flex w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors file:border-0 file:bg-transparent file:text-sm file:font-medium placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
          size === "sm" ? "h-8" : "h-9",
          className,
        )}
        ref={ref}
        {...props}
      />
    );
  },
);
Input.displayName = "Input";

export { Input };
