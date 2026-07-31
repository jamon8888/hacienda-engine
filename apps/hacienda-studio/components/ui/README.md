# `components/ui/` — provenance

Track E1 decided to adopt `hacienda-private`'s shadcn/ui component set. In practice that
repo's `components/ui/*.tsx` split into two generations:

- **Plain shadcn/ui** (`card.tsx`): a `React.forwardRef` wrapper around plain HTML elements
  with Tailwind classes, no other runtime dependency. Ported here **verbatim**.
- **`@base-ui/react`-based** (`button.tsx`, `badge.tsx`, `dialog.tsx`, `popover.tsx`,
  `input.tsx`, and others not yet needed here): built on Base UI's `useRender`/`mergeProps`
  polymorphic-render pattern, plus Tailwind v4 arbitrary-value functions (`--spacing(3)`,
  etc.) this app's Tailwind v3 setup doesn't understand. `hacienda-private` has migrated to
  this generation; this app has not adopted `@base-ui/react` as a dependency.

For the second group, this directory has **its own components**, written from the classic
`@radix-ui/react-*` + `class-variance-authority` shadcn/ui recipe instead of a port —
same visual/API contract (`variant`/`size` props, `asChild`, etc.), different underlying
primitive library. Each file says so at the top. If `@base-ui/react` is adopted later
(matching hacienda-private exactly), these are the files to replace with real ports.

`popover.tsx` and `dialog.tsx` also diverge in one more way: neither this app's ported
tokens (`app.css`) nor hacienda-private's own `globals.css` define a `--popover` CSS
variable, despite shadcn's usual convention assuming one exists — `card`/`background` are
used instead, the closest already-defined surface colors.
