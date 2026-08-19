# Hacienda Studio Next.js Rebuild — Design Specification

**Date**: 2026-08-10
**Status**: Approved for implementation planning

---

## 1. Architecture

### Framework & Rendering
- **Next.js 14+** with App Router, TypeScript, Tailwind CSS
- **Server Components** (default): Layout, marketing pages, auth-protected dashboard shell
- **Client Components** (`'use client'`): All interactive features — file upload, processing pipeline, editors, viewers, file browser
- **Edge Middleware**: Auth protection, locale detection
- **Deployment**: Vercel (native Next.js support, Edge Functions, automatic WASM handling)

### Project Structure
```
apps/hacienda-studio/
├── src/
│   ├── app/
│   │   ├── (auth)/                    # Auth routes (login, register, callback)
│   │   ├── (dashboard)/               # Protected dashboard routes
│   │   │   ├── studio/                # Main studio app
│   │   │   │   ├── page.tsx           # Upload → Queue → Browser → Detail
│   │   │   │   ├── components/        # Studio-specific components
│   │   │   │   └── hooks/             # Studio hooks
│   │   │   ├── settings/              # User settings
│   │   │   └── layout.tsx             # Dashboard shell with sidebar
│   │   ├── api/
│   │   │   ├── auth/[...nextauth]/    # NextAuth.js
│   │   │   ├── sync/                  # IndexedDB ↔ server sync
│   │   │   ├── documents/             # Document CRUD (server-first)
│   │   │   └── audit/                 # Audit chain endpoints
│   │   ├── layout.tsx                 # Root layout
│   │   └── page.tsx                   # Landing page
│   ├── components/
│   │   ├── ui/                        # shadcn/ui primitives + extend-hq/ui
│   │   ├── extend/                    # extend-hq/ui components (installed via shadcn)
│   │   └── shared/                    # Shared components across features
│   ├── lib/
│   │   ├── wasm/                      # WASM initialization, wrappers
│   │   ├── auth/                      # NextAuth config, providers
│   │   ├── db/                        # Prisma/Drizzle client (server)
│   │   ├── indexeddb/                 # IndexedDB utilities (client)
│   │   ├── sync/                      # Sync engine (client + server)
│   │   ├── pii/                       # PII detection/redaction types
│   │   └── utils/
│   ├── hooks/                         # Shared React hooks
│   └── types/                         # Shared TypeScript types
├── public/
│   └── wasm/                          # WASM files (copied at build)
├── components.json                    # shadcn config with extend aliases
├── next.config.js
├── tailwind.config.ts
└── package.json
```

### Onboarding Flow (Model Download)
1. **First visit** → `/onboarding` page
2. **Asset loading screen**:
   - Load `@xberg-io/xberg-wasm` (downloads ~48MB WASM)
   - Check IndexedDB for cached NER model (GLiNER2, ~1.2GB)
   - If not cached: download with progress (byte-level via `fetch` + `ReadableStream`)
   - Download tessdata for Tesseract (languages: eng, configurable)
   - Show clear progress for each asset
3. **Fallback handling**: If NER model fails → continue with regex-only (compromise.js), show banner
4. **Complete** → redirect to `/studio`, set `onboardingComplete` in localStorage
5. **Return visits** → skip onboarding, go straight to `/studio`

**Implementation**:
- `src/app/(dashboard)/studio/onboarding/page.tsx`
- `src/lib/wasm/asset-loader.ts`
- `src/components/studio/AssetLoadingScreen.tsx`

---

## 2. Components & extend-hq/ui Integration

### extend-hq/ui Components (installed via `npx shadcn@latest add @extend/*`)
| Component | Package | Replaces |
|-----------|---------|----------|
| PDF Viewer | `@extend/pdf-viewer` | Custom `PDFViewer` |
| DOCX Viewer | `@extend/docx-viewer` | Custom `DocxViewerPreview` |
| XLSX Viewer | `@extend/xlsx-viewer` | Custom `XlsxViewerPreview` |
| PPTX Viewer | `@extend/pptx-viewer` | Custom `PptxViewerPreview` |
| File System Block | `@extend/file-system-block` | Custom `FileBrowser` + `FileBrowserScreen` |
| File Upload | `@extend/file-upload` | Custom `UploadScreen` drop zone |
| File Thumbnail | `@extend/file-thumbnail` | Custom file icons/previews |
| Layout Blocks | `@extend/layout-blocks` | New: OCR block visualization |
| Bounding Box Citations | `@extend/bounding-box-citations` | New: PII finding review with citations |
| E-Signature | `@extend/e-signature` | Future: document signing |

### Custom Studio Components
- `MarkdownEditor` — CodeMirror 6 based, with PII finding decorations
- `RedactedEditor` — Split-pane editor for redacted markdown
- `PiiPanel` — Findings list with add/remove actions
- `ConfigPanel` — Pipeline configuration (NER categories, chunking, PII, redaction mode, verticals)
- `QueueScreen` — Processing progress with per-file stages
- `DetailScreen` — Split view: native viewer (left) + tabs: Redacted Markdown / Findings (right)
- `AssetLoadingScreen` — Onboarding asset download progress
- `RedactionModeSelector` — Mask / Hash / Pseudonymize / Remove

### shadcn/ui Primitives
Button, Dialog, Tabs, Select, ScrollArea, Tooltip, Progress, ResizablePanelGroup, etc.

### Installation Commands
```bash
# Primitives first
npx shadcn@latest add button dialog tabs select scroll-area tooltip progress resizable-panel-group

# extend-hq/ui components
npx shadcn@latest add @extend/pdf-viewer @extend/docx-viewer @extend/xlsx-viewer @extend/pptx-viewer
npx shadcn@latest add @extend/file-system-block @extend/file-upload @extend/file-thumbnail
npx shadcn@latest add @extend/layout-blocks @extend/bounding-box-citations @extend/e-signature
```

### Import Aliases (`components.json`)
```json
{
  "@": "./src",
  "@/components/ui": "./src/components/ui",
  "@/components/extend": "./src/components/extend"
}
```

---

## 3. Data Flow & State Management

### Client-Side State (React + Zustand)
```
Studio Page (root)
├── Onboarding State (localStorage)
├── Assets State (WASM, NER model, tessdata)
├── Files State (File[] + validation)
├── Processing State (Web Worker + progress Map)
├── Results State (ProcessedFile[])
├── Config State (AppConfig - synced to worker)
├── UI State (screen: upload/queue/browser/detail, modals)
├── Editing State (editedFindings Map, redactedDrafts Map)
├── Preview URLs (Object URLs for native viewers)
├── Content Hashes (SHA-256 for IndexedDB keys)
└── Auth State (NextAuth session)
```

### Web Worker Pipeline (unchanged)
- `worker/pipeline.ts` — processes files via `@xberg-io/xberg-wasm` + local `hacienda-wasm`
- Messages: `init` → `ready`, `process` → `progress`/`file-complete`/`batch-complete`/`error`, `build-zip` → `zip-ready`
- WASM initialized once in worker, reused for all files

### IndexedDB (Client Persistence)
- `audit-chain` — tamper-evident audit log (via `AuditHandle` from `hacienda-wasm`)
- `redaction-drafts` — per-document redacted markdown drafts (keyed by content hash)
- `ner-model-cache` — GLiNER2 model + tokenizer (IndexedDB, ~1.2GB)
- `tessdata-cache` — Tesseract language data
- `user-preferences` — config, theme, etc.

### Server Sync (Background, when authenticated)
**Sync Engine** (client): watches IndexedDB changes, queues sync operations
**API Routes** (server):
- `POST /api/sync/documents` — upsert processed documents
- `POST /api/sync/audit` — append audit entries
- `POST /api/sync/drafts` — save/restore redaction drafts
- `GET /api/sync/manifest` — get server state for conflict resolution

**Conflict Resolution**: Last-write-wins with content-hash comparison; server wins on audit chain (append-only)

### NextAuth.js Integration
- Providers: Credentials (email/password), GitHub, Google
- Session: JWT in HTTP-only cookie, 30-day expiry
- Protected routes: middleware checks session, redirects to `/login`
- User data: `id`, `email`, `name`, `image`, `createdAt`

---

## 4. Error Handling & Edge Cases

| Scenario | Handling |
|----------|----------|
| WASM load failure | Retry 3× with exponential backoff; clear cache on corruption; graceful degradation notice |
| NER model download | Resume via Range requests; quota exceeded → clear non-essential caches; 5-min timeout → regex fallback |
| Processing errors | Per-file, non-blocking: show in FileBrowser with error badge, continue batch |
| Zip build failure | Show error, allow individual file download |
| Sync conflicts | Draft: server wins, notify "restored from server"; Audit: server wins (append-only), verify chain |
| Offline edits | Queue locally, sync on reconnect |
| Auth errors | Session expired → silent refresh; CSRF → NextAuth built-in; Rate limit → exponential backoff |

---

## 5. Testing Strategy

| Layer | Tool | Coverage |
|-------|------|----------|
| Unit (lib) | Vitest | WASM loaders, sync engine, utilities |
| Component | React Testing Library + Vitest | UI components, hooks |
| Integration | Playwright | Full flows: onboarding → upload → process → edit → export |
| E2E (auth) | Playwright | Login, register, protected routes, session persistence |
| WASM | wasm-bindgen-test | Rust-side pipeline logic |

**Test Fixtures**: Reuse existing `apps/hacienda-studio/tests/` fixtures + add auth scenarios.

---

## 6. Deployment (Vercel)

### Build Configuration (`next.config.js`)
```js
module.exports = {
  async rewrites() {
    return [{
      source: '/wasm/:path*',
      destination: '/wasm/:path*',
    }];
  },
  experimental: {
    serverActions: { bodySizeLimit: '50mb' },
  },
  async headers() {
    return [{
      source: '/wasm/:path*.wasm',
      headers: [{ key: 'Content-Type', value: 'application/wasm' }],
    }];
  },
};
```

### Vercel Settings
- Framework: Next.js
- Build Command: `pnpm build` (includes `wasm-pack build` for hacienda-wasm)
- Output: `.next` (default)
- Functions: Default (Edge for middleware, Node.js for API routes)
- Environment Variables: `NEXTAUTH_SECRET`, `NEXTAUTH_URL`, OAuth credentials, `DATABASE_URL`

### WASM Build Step (`package.json`)
```json
"scripts": {
  "prebuild": "cd ../../crates/hacienda-wasm && wasm-pack build --target web --out-dir ../../apps/hacienda-studio/public/wasm/hacienda-wasm --no-default-features",
  "build": "next build",
  "postbuild": "cp ../../crates/hacienda-wasm/pkg/web/* public/wasm/hacienda-wasm/"
}
```

---

## 7. Implementation Phases

### Phase 1: Foundation
1. Initialize Next.js 14+ project with TypeScript, Tailwind, ESLint, Prettier
2. Configure shadcn/ui with extend-hq/ui aliases in `components.json`
3. Install shadcn primitives + extend-hq/ui components
4. Set up NextAuth.js with Credentials + OAuth providers
5. Configure middleware for auth protection
6. Set up Prisma/Drizzle with PostgreSQL (Vercel Postgres or Neon)
7. Configure WASM build pipeline (`wasm-pack` → `public/wasm`)

### Phase 2: Onboarding & Asset Loading
1. Create `/onboarding` route with `AssetLoadingScreen`
2. Implement `asset-loader.ts` with progress tracking
3. IndexedDB caching for NER model + tessdata
4. Fallback to regex-only on NER failure

### Phase 3: Studio Core (Client Components)
1. Studio page with screen state machine (upload → queue → browser → detail)
2. Web Worker integration (port existing `pipeline.ts`)
3. File upload with `@extend/file-upload`
4. Processing queue with progress
5. File browser with `@extend/file-system-block`
6. Detail screen with split view (native viewers + tabs)

### Phase 4: Editors & PII Features
1. `MarkdownEditor` with CodeMirror 6 + PII decorations
2. `RedactedEditor` for split-pane editing
3. `PiiPanel` with findings list + add/remove
4. `ConfigPanel` with all pipeline options
5. Redaction modes: mask, hash, pseudonymize, remove
6. Zip export (on-demand via worker)

### Phase 5: Server Sync & Auth Integration
1. IndexedDB utilities for audit chain, drafts, preferences
2. Sync engine (client) with queue + conflict resolution
3. API routes for documents, audit, drafts, manifest
4. NextAuth session integration in studio
5. Protected dashboard layout with settings page

### Phase 6: Polish & Deploy
1. Landing page with feature overview
2. Error boundaries, loading states, empty states
3. Accessibility audit (WCAG 2.1 AA)
4. Performance optimization (bundle analysis, WASM streaming)
5. Vercel deployment + preview environments
6. Documentation + README

---

## 8. Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| WASM + Next.js SSR incompatibility | Load WASM only in `'use client'` components; use dynamic import with `ssr: false` |
| NER model size (1.2GB) | Stream download with progress; cache in IndexedDB; fallback to regex |
| extend-hq/ui customization needs | Components installed as source — fully editable |
| IndexedDB quota limits | Monitor usage; clear old drafts; prompt user before large downloads |
| Sync conflicts | Content-hash based resolution; audit chain append-only |
| Vercel function limits | Large processing stays client-side; API routes only for sync/metadata |

---

## 9. Open Questions (Resolved)

1. **Auth provider** → NextAuth.js (Auth.js) ✓
2. **WASM integration** → Keep `@xberg-io/xberg-wasm` + local `hacienda-wasm` ✓
3. **extend-hq/ui scope** → Full suite ✓
4. **Data persistence** → Local-first + server sync ✓
5. **Deployment** → Vercel ✓
6. **Onboarding** → Included with model download ✓

---

*This design has been reviewed and approved. Next step: invoke `writing-plans` skill to create detailed implementation plan.*