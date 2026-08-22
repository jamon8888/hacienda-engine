import {
  CircleCheckIcon,
  InfoIcon,
  Loader2Icon,
  OctagonXIcon,
  TriangleAlertIcon,
} from "lucide-react"
import { Toaster as Sonner, type ToasterProps } from "sonner"
import type { CSSProperties } from "react"

/** Sonner's own custom CSS variables (`--normal-bg` etc., documented in its own
 * `ToasterProps.style` typing) — `React.CSSProperties` doesn't declare these, hence the
 * dedicated interface instead of an `as React.CSSProperties` assertion that would also
 * silently accept an unrelated typo in one of the four keys below. */
interface ToasterStyle extends CSSProperties {
  "--normal-bg"?: string
  "--normal-text"?: string
  "--normal-border"?: string
  "--border-radius"?: string
}

// No `next-themes` here: the app is dark-only (see app.css's `.dark` block, applied
// unconditionally in main.tsx) with no toggle to read a theme from. `--normal-bg`/
// `--normal-text` point at `--card`/`--card-foreground`, not `--popover`/
// `--popover-foreground` — this project's shadcn setup never defined popover tokens
// (see components/ui/popover.tsx, which reuses `bg-card`/`text-card-foreground` too).
// A typed variable, not an inline object literal, so JSX's excess-property check (which
// applies to object literals assigned directly into a prop, even one satisfying a wider
// interface) doesn't reject the custom `--*` keys against `ToasterProps.style`'s declared
// `React.CSSProperties` — `toasterStyle`'s own type already proves it's a valid
// `CSSProperties` (via `extends`), so passing the variable through needs no assertion.
const toasterStyle: ToasterStyle = {
  "--normal-bg": "var(--card)",
  "--normal-text": "var(--card-foreground)",
  "--normal-border": "var(--border)",
  "--border-radius": "var(--radius)",
}

const Toaster = ({ ...props }: ToasterProps) => {
  return (
    <Sonner
      theme="dark"
      className="toaster group"
      icons={{
        success: <CircleCheckIcon className="size-4" />,
        info: <InfoIcon className="size-4" />,
        warning: <TriangleAlertIcon className="size-4" />,
        error: <OctagonXIcon className="size-4" />,
        loading: <Loader2Icon className="size-4 animate-spin" />,
      }}
      style={toasterStyle}
      {...props}
    />
  )
}

export { Toaster }
