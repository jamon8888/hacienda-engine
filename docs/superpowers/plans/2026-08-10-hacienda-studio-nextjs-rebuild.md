# Hacienda Studio Next.js Rebuild — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild Hacienda Studio from Vite/React 18 to Next.js 14+ App Router with extend-hq/ui components, NextAuth.js authentication, and Vercel deployment.

**Architecture:** Next.js App Router with Server Components for layout/auth and Client Components for all interactive features (WASM processing, file upload, viewers, editors). extend-hq/ui replaces custom document viewers and file browser. Local-first processing via Web Worker + WASM, with background server sync when authenticated.

**Tech Stack:** Next.js 14+, React 18, TypeScript, Tailwind CSS, shadcn/ui + extend-hq/ui, NextAuth.js (Auth.js), Zustand, CodeMirror 6, @xberg-io/xberg-wasm, hacienda-wasm (WASM), IndexedDB, Prisma/Drizzle, PostgreSQL, Vercel

## Global Constraints

- Next.js 14+ App Router required (not Pages Router)
- WASM modules loaded only in `'use client'` components via dynamic import with `ssr: false`
- extend-hq/ui components installed as source via `npx shadcn@latest add @extend/*` — fully customizable
- shadcn/ui primitives installed first (button, dialog, tabs, etc.) before extend-hq/ui components
- `components.json` must define aliases: `@` → `./src`, `@/components/ui` → `./src/components/ui`, `@/components/extend` → `./src/components/extend`
- Local-first: all processing in browser via Web Worker + WASM
- Server sync: background sync of drafts, audit chain, processed documents when online + authenticated
- Auth: NextAuth.js with Credentials + OAuth (GitHub, Google), JWT in HTTP-only cookie
- Deployment: Vercel with `wasm-pack build` in prebuild script
- Reuse existing worker/pipeline.ts logic — port, don't rewrite
- Reuse existing lib/ files (annotate.ts, pii-engine.ts, pseudonymize.ts, etc.) — adapt imports for Next.js

---

## Phase 1: Foundation

### Task 1: Initialize Next.js Project

**Files:**
- Create: `apps/hacienda-studio-next/` (new directory)
- Create: `apps/hacienda-studio-next/package.json`
- Create: `apps/hacienda-studio-next/next.config.js`
- Create: `apps/hacienda-studio-next/tsconfig.json`
- Create: `apps/hacienda-studio-next/tailwind.config.ts`
- Create: `apps/hacienda-studio-next/postcss.config.js`
- Create: `apps/hacienda-studio-next/src/app/layout.tsx`
- Create: `apps/hacienda-studio-next/src/app/page.tsx`

**Interfaces:**
- Consumes: None (fresh project)
- Produces: Working Next.js dev server at localhost:3000

- [ ] **Step 1: Create Next.js project**

```bash
cd /home/jamin/Documents/hacienda-engine/apps
npx create-next-app@latest hacienda-studio-next --typescript --tailwind --eslint --app --src-dir --import-alias "@/*" --use-pnpm
```

- [ ] **Step 2: Verify dev server starts**

```bash
cd hacienda-studio-next && pnpm dev
# Open http://localhost:3000 — should show default Next.js page
```

- [ ] **Step 3: Clean default content**

Replace `src/app/page.tsx`:

```tsx
export default function Home() {
  return (
    <main className="flex min-h-screen flex-col items-center justify-center">
      <h1 className="text-4xl font-bold">Hacienda Studio</h1>
      <p className="mt-4 text-muted-foreground">Document intelligence platform</p>
    </main>
  );
}
```

- [ ] **Step 4: Commit**

```bash
git add apps/hacienda-studio-next
git commit -m "feat: initialize Next.js 14 project for hacienda studio"
```

---

### Task 2: Configure shadcn/ui and extend-hq/ui

**Files:**
- Create: `apps/hacienda-studio-next/components.json`
- Create: `apps/hacienda-studio-next/src/components/ui/` (shadcn primitives)
- Create: `apps/hacienda-studio-next/src/components/extend/` (extend-hq/ui)

**Interfaces:**
- Consumes: Task 1 (working Next.js project)
- Produces: shadcn + extend-hq/ui components available via imports

- [ ] **Step 1: Initialize shadcn/ui**

```bash
cd apps/hacienda-studio-next
npx shadcn@latest init
# Choose: New York style, Zinc base color, CSS variables: yes
```

- [ ] **Step 2: Install shadcn primitives**

```bash
npx shadcn@latest add button dialog tabs select scroll-area tooltip progress resizable-panel-group card separator dropdown-menu input label
```

- [ ] **Step 3: Install extend-hq/ui components**

```bash
npx shadcn@latest add @extend/pdf-viewer
npx shadcn@latest add @extend/docx-viewer
npx shadcn@latest add @extend/xlsx-viewer
npx shadcn@latest add @extend/pptx-viewer
npx shadcn@latest add @extend/file-system-block
npx shadcn@latest add @extend/file-upload
npx shadcn@latest add @extend/file-thumbnail
npx shadcn@latest add @extend/layout-blocks
npx shadcn@latest add @extend/bounding-box-citations
npx shadcn@latest add @extend/e-signature
```

- [ ] **Step 4: Verify components installed**

```bash
ls src/components/ui/
ls src/components/extend/
```

- [ ] **Step 5: Commit**

```bash
git add apps/hacienda-studio-next/components.json apps/hacienda-studio-next/src/components
git commit -m "feat: configure shadcn/ui and install extend-hq/ui components"
```

---

### Task 3: Set Up NextAuth.js

**Files:**
- Create: `apps/hacienda-studio-next/src/lib/auth/config.ts`
- Create: `apps/hacienda-studio-next/src/app/api/auth/[...nextauth]/route.ts`
- Create: `apps/hacienda-studio-next/src/middleware.ts`
- Create: `apps/hacienda-studio-next/.env.local` (template)

**Interfaces:**
- Consumes: Task 1 (Next.js project)
- Produces: `auth()` function, session hook, protected route middleware

- [ ] **Step 1: Install NextAuth.js**

```bash
cd apps/hacienda-studio-next
pnpm add next-auth@beta
```

- [ ] **Step 2: Create auth config**

```bash
mkdir -p src/lib/auth
```

Create `src/lib/auth/config.ts`:

```ts
import { NextAuthConfig } from "next-auth";
import Credentials from "next-auth/providers/credentials";
import GitHub from "next-auth/providers/github";
import Google from "next-auth/providers/google";

export const authConfig: NextAuthConfig = {
  providers: [
    GitHub({
      clientId: process.env.GITHUB_ID,
      clientSecret: process.env.GITHUB_SECRET,
    }),
    Google({
      clientId: process.env.GOOGLE_ID,
      clientSecret: process.env.GOOGLE_SECRET,
    }),
    Credentials({
      name: "credentials",
      credentials: {
        email: { label: "Email", type: "email" },
        password: { label: "Password", type: "password" },
      },
      async authorize(credentials) {
        return null;
      },
    }),
  ],
  session: { strategy: "jwt" },
  pages: {
    signIn: "/login",
    error: "/login",
  },
  callbacks: {
    async jwt({ token, user }) {
      if (user) {
        token.id = user.id;
      }
      return token;
    },
    async session({ session, token }) {
      if (session.user) {
        session.user.id = token.id as string;
      }
      return session;
    },
  },
};
```

- [ ] **Step 3: Create API route**

```bash
mkdir -p "src/app/api/auth/[...nextauth]"
```

Create `src/app/api/auth/[...nextauth]/route.ts`:

```ts
import NextAuth from "next-auth";
import { authConfig } from "@/lib/auth/config";

const handler = NextAuth(authConfig);

export { handler as GET, handler as POST };
```

- [ ] **Step 4: Create middleware**

Create `src/middleware.ts`:

```ts
import NextAuth from "next-auth";
import { authConfig } from "@/lib/auth/config";

const { auth } = NextAuth(authConfig);

export default auth((req) => {
  const isLoggedIn = !!req.auth;
  const isOnAuth = req.nextUrl.pathname.startsWith("/login") || req.nextUrl.pathname.startsWith("/register");
  const isOnApi = req.nextUrl.pathname.startsWith("/api");
  const isOnPublic = req.nextUrl.pathname === "/";

  if (isOnAuth || isOnApi || isOnPublic) return;

  if (!isLoggedIn) {
    return Response.redirect(new URL("/login", req.nextUrl));
  }
});

export const config = {
  matcher: ["/((?!_next/static|_next/image|favicon.ico|public/).*)"],
};
```

- [ ] **Step 5: Create .env.local template**

```bash
cat > .env.local << 'EOF'
# NextAuth
NEXTAUTH_URL=http://localhost:3000
NEXTAUTH_SECRET=your-secret-here-change-in-production

# GitHub OAuth (optional)
GITHUB_ID=
GITHUB_SECRET=

# Google OAuth (optional)
GOOGLE_ID=
GOOGLE_SECRET=

# Database (for server sync)
DATABASE_URL=
EOF
```

- [ ] **Step 6: Commit**

```bash
git add apps/hacienda-studio-next/src/lib/auth apps/hacienda-studio-next/src/app/api apps/hacienda-studio-next/src/middleware.ts apps/hacienda-studio-next/.env.local
git commit -m "feat: set up NextAuth.js with Credentials, GitHub, Google providers"
```

---

### Task 4: Configure WASM Build Pipeline

**Files:**
- Create: `apps/hacienda-studio-next/public/wasm/` (output directory)
- Modify: `apps/hacienda-studio-next/package.json` (add build scripts)
- Modify: `apps/hacienda-studio-next/next.config.js` (WASM headers)

**Interfaces:**
- Consumes: Task 1 (Next.js project)
- Produces: WASM files in public/wasm/, build scripts for wasm-pack

- [ ] **Step 1: Create wasm output directory**

```bash
mkdir -p apps/hacienda-studio-next/public/wasm/hacienda-wasm
```

- [ ] **Step 2: Update package.json scripts**

Add to `apps/hacienda-studio-next/package.json` scripts:

```json
{
  "scripts": {
    "prebuild:wasm": "cd ../../crates/hacienda-wasm && wasm-pack build --target web --out-dir ../../apps/hacienda-studio-next/public/wasm/hacienda-wasm --no-default-features",
    "build:wasm": "echo 'WASM build complete'",
    "postbuild:wasm": "cp ../../crates/hacienda-wasm/pkg/web/*.wasm apps/hacienda-studio-next/public/wasm/hacienda-wasm/ 2>/dev/null || true"
  }
}
```

- [ ] **Step 3: Update next.config.js for WASM**

```js
/** @type {import('next').NextConfig} */
const nextConfig = {
  async headers() {
    return [
      {
        source: "/wasm/:path*.wasm",
        headers: [
          { key: "Content-Type", value: "application/wasm" },
          { key: "Cache-Control", value: "public, max-age=31536000, immutable" },
        ],
      },
    ];
  },
  experimental: {
    serverActions: { bodySizeLimit: "50mb" },
  },
};

module.exports = nextConfig;
```

- [ ] **Step 4: Verify wasm-pack is available**

```bash
which wasm-pack || cargo install wasm-pack
```

- [ ] **Step 5: Commit**

```bash
git add apps/hacienda-studio-next/public/wasm apps/hacienda-studio-next/package.json apps/hacienda-studio-next/next.config.js
git commit -m "feat: configure WASM build pipeline with wasm-pack"
```

---

### Task 5: Set Up Database (Prisma + PostgreSQL)

**Files:**
- Create: `apps/hacienda-studio-next/prisma/schema.prisma`
- Create: `apps/hacienda-studio-next/src/lib/db/client.ts`
- Create: `apps/hacienda-studio-next/prisma/seed.ts`

**Interfaces:**
- Consumes: Task 1 (Next.js project)
- Produces: Prisma client, database schema for User, Document, AuditEntry, Draft

- [ ] **Step 1: Install Prisma**

```bash
cd apps/hacienda-studio-next
pnpm add prisma @prisma/client
pnpm add -D ts-node
```

- [ ] **Step 2: Initialize Prisma**

```bash
npx prisma init --datasource-provider postgresql
```

- [ ] **Step 3: Define schema**

Replace `prisma/schema.prisma`:

```prisma
generator client {
  provider = "prisma-client-js"
}

datasource db {
  provider = "postgresql"
  url      = env("DATABASE_URL")
}

model User {
  id            String    @id @default(cuid())
  email         String    @unique
  name          String?
  image         String?
  passwordHash  String?
  createdAt     DateTime  @default(now())
  updatedAt     DateTime  @updatedAt
  documents     Document[]
  auditEntries  AuditEntry[]
  drafts        Draft[]
}

model Document {
  id            String    @id @default(cuid())
  userId        String
  user          User      @relation(fields: [userId], references: [id])
  name          String
  originalType  String
  markdown      String
  rawMarkdown   String
  contentHash   String    @unique
  piiCount      Int       @default(0)
  entityCount   Int       @default(0)
  entities      Json?
  createdAt     DateTime  @default(now())
  updatedAt     DateTime  @updatedAt
  auditEntries  AuditEntry[]
  drafts        Draft[]
}

model AuditEntry {
  id              String    @id @default(cuid())
  userId          String
  user            User      @relation(fields: [userId], references: [id])
  documentId      String
  document        Document  @relation(fields: [documentId], references: [id])
  category        String
  action          String
  spanHash        String
  spanLength      Int
  confidence      Float
  source          String
  pipelineVersion String
  configHash      String?
  vertical        String?
  createdAt       DateTime  @default(now())
}

model Draft {
  id            String    @id @default(cuid())
  userId        String
  user          User      @relation(fields: [userId], references: [id])
  documentId    String
  document      Document  @relation(fields: [documentId], references: [id])
  content       String
  contentHash   String
  createdAt     DateTime  @default(now())
  updatedAt     DateTime  @updatedAt

  @@unique([userId, documentId])
}
```

- [ ] **Step 4: Create Prisma client**

```bash
mkdir -p src/lib/db
```

Create `src/lib/db/client.ts`:

```ts
import { PrismaClient } from "@prisma/client";

const globalForPrisma = globalThis as unknown as {
  prisma: PrismaClient | undefined;
};

export const prisma = globalForPrisma.prisma ?? new PrismaClient();

if (process.env.NODE_ENV !== "production") globalForPrisma.prisma = prisma;
```

- [ ] **Step 5: Run migration**

```bash
npx prisma migrate dev --name init
```

- [ ] **Step 6: Commit**

```bash
git add apps/hacienda-studio-next/prisma apps/hacienda-studio-next/src/lib/db
git commit -m "feat: set up Prisma with PostgreSQL schema for User, Document, AuditEntry, Draft"
```

---

## Phase 2: Onboarding & Asset Loading

### Task 6: Port Asset Loader to Next.js

**Files:**
- Create: `apps/hacienda-studio-next/src/lib/wasm/asset-loader.ts`
- Create: `apps/hacienda-studio-next/src/lib/wasm/indexeddb-cache.ts`

**Interfaces:**
- Consumes: Existing `apps/hacienda-studio/lib/asset-loader.ts` (reference)
- Produces: `preloadXbergWasm()`, `loadNerModel()`, `isModelCached()`, `DownloadProgress`

- [ ] **Step 1: Create indexeddb-cache.ts**

```bash
mkdir -p src/lib/wasm
```

Create `src/lib/wasm/indexeddb-cache.ts`:

```ts
const DB_NAME = "hacienda-studio-cache";
const DB_VERSION = 1;

export async function openCacheDB(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, DB_VERSION);
    request.onerror = () => reject(request.error);
    request.onsuccess = () => resolve(request.result);
    request.onupgradeneeded = (event) => {
      const db = (event.target as IDBOpenDBRequest).result;
      if (!db.objectStoreNames.contains("ner-model")) {
        db.createObjectStore("ner-model");
      }
      if (!db.objectStoreNames.contains("tessdata")) {
        db.createObjectStore("tessdata");
      }
    };
  });
}

export async function getCachedItem(storeName: string, key: string): Promise<ArrayBuffer | null> {
  const db = await openCacheDB();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(storeName, "readonly");
    const store = tx.objectStore(storeName);
    const request = store.get(key);
    request.onerror = () => reject(request.error);
    request.onsuccess = () => resolve(request.result ?? null);
  });
}

export async function setCachedItem(storeName: string, key: string, value: ArrayBuffer): Promise<void> {
  const db = await openCacheDB();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(storeName, "readwrite");
    const store = tx.objectStore(storeName);
    const request = store.put(value, key);
    request.onerror = () => reject(request.error);
    request.onsuccess = () => resolve();
  });
}
```

- [ ] **Step 2: Create asset-loader.ts**

Create `src/lib/wasm/asset-loader.ts`:

```ts
import { getCachedItem, setCachedItem } from "./indexeddb-cache";

export interface DownloadProgress {
  loaded: number;
  total: number;
  percent: number;
}

let wasmInitialized = false;

export async function preloadXbergWasm(): Promise<void> {
  if (wasmInitialized) return;

  const initWasm = (await import("@xberg-io/xberg-wasm")).default;
  const wasmUrl = (await import("@xberg-io/xberg-wasm/pkg/web/xberg_wasm_bg.wasm?url")).default;

  await initWasm({ module_or_path: fetch(wasmUrl) });
  wasmInitialized = true;
}

export async function isModelCached(): Promise<boolean> {
  const cached = await getCachedItem("ner-model", "gliner2-model");
  return cached !== null;
}

export async function loadNerModel(
  onProgress?: (progress: DownloadProgress) => void
): Promise<{ model: ArrayBuffer; tokenizer: ArrayBuffer; encoderConfig: unknown }> {
  const cached = await getCachedItem("ner-model", "gliner2-model");
  if (cached) {
    return { model: cached, tokenizer: new ArrayBuffer(0), encoderConfig: {} };
  }

  const modelUrl = "https://huggingface.co/hacienda/gliner2/resolve/main/model.bin";
  const response = await fetch(modelUrl);

  if (!response.ok) {
    throw new Error(`Failed to download NER model: ${response.status}`);
  }

  const contentLength = response.headers.get("content-length");
  const total = contentLength ? parseInt(contentLength, 10) : 0;

  const reader = response.body!.getReader();
  const chunks: Uint8Array[] = [];
  let loaded = 0;

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;

    chunks.push(value);
    loaded += value.length;

    if (onProgress) {
      onProgress({ loaded, total, percent: total ? Math.round((loaded / total) * 100) : 0 });
    }
  }

  const model = new Uint8Array(loaded);
  let offset = 0;
  for (const chunk of chunks) {
    model.set(chunk, offset);
    offset += chunk.length;
  }

  await setCachedItem("ner-model", "gliner2-model", model.buffer);

  return { model: model.buffer, tokenizer: new ArrayBuffer(0), encoderConfig: {} };
}

export async function createNerBackend(
  model: ArrayBuffer,
  tokenizer: ArrayBuffer,
  encoderConfig: unknown
): Promise<{ detect: (text: string, opts: { categories: string[] }) => Promise<unknown[]> }> {
  const { NerModel } = await import("@xberg-io/xberg-wasm");
  const nerModel = new NerModel(model);

  return {
    detect: async (text: string, opts: { categories: string[] }) => {
      return nerModel.detect(text, opts);
    },
  };
}
```

- [ ] **Step 3: Verify imports work**

```bash
cd apps/hacienda-studio-next
npx tsc --noEmit
```

- [ ] **Step 4: Commit**

```bash
git add apps/hacienda-studio-next/src/lib/wasm
git commit -m "feat: port asset-loader with IndexedDB caching for NER model"
```

---

### Task 7: Create Asset Loading Screen

**Files:**
- Create: `apps/hacienda-studio-next/src/components/studio/AssetLoadingScreen.tsx`
- Create: `apps/hacienda-studio-next/src/app/(dashboard)/studio/onboarding/page.tsx`

**Interfaces:**
- Consumes: Task 6 (asset-loader.ts)
- Produces: Onboarding UI with progress bars, redirects to /studio on completion

- [ ] **Step 1: Create AssetLoadingScreen component**

```bash
mkdir -p src/components/studio
```

Create `src/components/studio/AssetLoadingScreen.tsx`:

```tsx
"use client";

import { useEffect, useState } from "react";
import { Progress } from "@/components/ui/progress";
import { preloadXbergWasm, loadNerModel, isModelCached, type DownloadProgress } from "@/lib/wasm/asset-loader";

interface AssetLoadingScreenProps {
  onComplete: () => void;
}

export function AssetLoadingScreen({ onComplete }: AssetLoadingScreenProps) {
  const [assets, setAssets] = useState({ xbergWasm: false, nerModel: false, tessdata: false });
  const [nerProgress, setNerProgress] = useState<DownloadProgress | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function load() {
      try {
        setAssets((a) => ({ ...a, xbergWasm: true }));
        await preloadXbergWasm();
        if (cancelled) return;

        if (await isModelCached()) {
          setAssets((a) => ({ ...a, nerModel: true }));
        } else {
          try {
            await loadNerModel((p) => setNerProgress(p));
            if (cancelled) return;
            setAssets((a) => ({ ...a, nerModel: true }));
          } catch (e) {
            console.warn("NER model download failed, using fallback:", e);
            setAssets((a) => ({ ...a, nerModel: true }));
          }
        }

        setAssets((a) => ({ ...a, tessdata: true }));
        localStorage.setItem("hacienda-studio-visited", "true");

        if (!cancelled) onComplete();
      } catch (e) {
        if (!cancelled) {
          setError("Failed to load assets. Please refresh and try again.");
          setAssets({ xbergWasm: true, nerModel: true, tessdata: true });
        }
      }
    }

    load();
    return () => { cancelled = true; };
  }, [onComplete]);

  const completedCount = Object.values(assets).filter(Boolean).length;
  const totalCount = 3;
  const overallPercent = Math.round((completedCount / totalCount) * 100);

  return (
    <div className="flex min-h-screen flex-col items-center justify-center gap-6">
      <h1 className="text-2xl font-bold">Loading Hacienda Studio</h1>

      <div className="w-full max-w-md space-y-4">
        <div className="space-y-2">
          <div className="flex justify-between text-sm">
            <span>WASM Engine</span>
            <span>{assets.xbergWasm ? "✓" : "Loading..."}</span>
          </div>
        </div>

        <div className="space-y-2">
          <div className="flex justify-between text-sm">
            <span>NER Model</span>
            <span>{assets.nerModel ? "✓" : nerProgress ? `${nerProgress.percent}%` : "Loading..."}</span>
          </div>
          {!assets.nerModel && nerProgress && (
            <Progress value={nerProgress.percent} className="h-2" />
          )}
        </div>

        <div className="space-y-2">
          <div className="flex justify-between text-sm">
            <span>OCR Data</span>
            <span>{assets.tessdata ? "✓" : "Loading..."}</span>
          </div>
        </div>

        <Progress value={overallPercent} className="h-2" />
      </div>

      {error && <p className="text-sm text-destructive">{error}</p>}
    </div>
  );
}
```

- [ ] **Step 2: Create onboarding page**

```bash
mkdir -p "src/app/(dashboard)/studio/onboarding"
```

Create `src/app/(dashboard)/studio/onboarding/page.tsx`:

```tsx
"use client";

import { useRouter } from "next/navigation";
import { AssetLoadingScreen } from "@/components/studio/AssetLoadingScreen";

export default function OnboardingPage() {
  const router = useRouter();

  return (
    <AssetLoadingScreen onComplete={() => router.push("/studio")} />
  );
}
```

- [ ] **Step 3: Commit**

```bash
git add apps/hacienda-studio-next/src/components/studio/AssetLoadingScreen.tsx "apps/hacienda-studio-next/src/app/(dashboard)/studio/onboarding"
git commit -m "feat: create asset loading screen with progress tracking"
```

---

## Phase 3: Studio Core

### Task 8: Create Zustand Store

**Files:**
- Create: `apps/hacienda-studio-next/src/lib/store.ts`
- Create: `apps/hacienda-studio-next/src/lib/types.ts`

**Interfaces:**
- Consumes: Existing `apps/hacienda-studio/lib/types.ts` (reference)
- Produces: `useStudioStore` hook, all state types

- [ ] **Step 1: Create types**

Create `src/lib/types.ts`:

```ts
export interface FileInput {
  name: string;
  bytes: ArrayBuffer;
  type: string;
}

export interface Entity {
  name: string;
  type: string;
  slug: string;
  count: number;
  spans: Array<{ start: number; end: number }>;
  vertical?: string;
  sector?: string;
  roles?: string[];
}

export interface ProcessedFile {
  name: string;
  markdown: string;
  rawMarkdown: string;
  entities: Entity[];
  piiFindings: PiiEntity[];
  frontmatter: {
    source: string;
    type: string;
    processed: string;
    piiEntitiesFound: number;
    entities: Array<{
      name: string;
      type: string;
      slug: string;
      vertical?: string;
      sector?: string;
      roles?: string[];
    }>;
  };
}

export interface ProgressUpdate {
  file: string;
  stage: "queued" | "extract" | "ner" | "pii" | "link" | "complete" | "error";
  percent: number;
  message?: string;
}

export type NerCategory = "person" | "organization" | "location" | "date" | "time" | "money" | "percent" | "email" | "phone" | "url";

export interface AppConfig {
  nerCategories: NerCategory[];
  outputFormat: "markdown" | "plain" | "json";
  chunkSize: number;
  enableTranscription: boolean;
  transcriptionModel: "tiny.en" | "tiny" | "base.en" | "base" | "small.en" | "small";
  transcriptionLanguage: string;
  translateToEnglish: boolean;
  enablePiiDetection: boolean;
  redactPiiInOutput: boolean;
  enabledVerticals: ("m&a" | "financial_services" | "shared")[];
  redactionMode: "mask" | "pseudonymize";
  pseudonymPassphrase: string;
  pseudonymKeyId: string;
}

export interface PiiEntity {
  category: string;
  text: string;
  start: number;
  end: number;
  confidence: number;
  source: string;
  format_preserving: boolean;
  redact_template: string;
}

export type Screen = { kind: "upload" } | { kind: "queue" } | { kind: "browser" } | { kind: "detail"; inputName: string };

export const DEFAULT_CONFIG: AppConfig = {
  nerCategories: ["person", "organization", "location", "email", "phone"],
  outputFormat: "markdown",
  chunkSize: 1000,
  enableTranscription: false,
  transcriptionModel: "tiny.en",
  transcriptionLanguage: "auto",
  translateToEnglish: false,
  enablePiiDetection: true,
  redactPiiInOutput: false,
  enabledVerticals: ["m&a", "financial_services", "shared"],
  redactionMode: "mask",
  pseudonymPassphrase: "",
  pseudonymKeyId: "session",
};
```

- [ ] **Step 2: Create Zustand store**

```bash
pnpm add zustand
```

Create `src/lib/store.ts`:

```ts
import { create } from "zustand";
import type { Screen, AppConfig, ProcessedFile, ProgressUpdate, PiiEntity } from "./types";
import { DEFAULT_CONFIG } from "./types";

interface StudioState {
  screen: Screen;
  setScreen: (screen: Screen) => void;

  files: File[];
  addFiles: (files: File[]) => void;

  progress: Map<string, ProgressUpdate>;
  setProgress: (file: string, update: ProgressUpdate) => void;
  clearProgress: () => void;

  results: ProcessedFile[];
  addResult: (result: ProcessedFile) => void;
  setResults: (results: ProcessedFile[]) => void;

  config: AppConfig;
  setConfig: (config: Partial<AppConfig>) => void;

  showConfig: boolean;
  setShowConfig: (show: boolean) => void;

  error: string | null;
  setError: (error: string | null) => void;

  skipNotice: string | null;
  setSkipNotice: (notice: string | null) => void;

  editedFindings: Map<string, PiiEntity[]>;
  setEditedFindings: (resultName: string, findings: PiiEntity[]) => void;

  redactedDrafts: Map<string, string>;
  setRedactedDraft: (resultName: string, draft: string) => void;

  previewUrls: Map<string, string>;
  setPreviewUrl: (resultName: string, url: string) => void;

  contentHashes: Map<string, string>;
  setContentHash: (fileName: string, hash: string) => void;

  workerReady: boolean;
  setWorkerReady: (ready: boolean) => void;
}

export const useStudioStore = create<StudioState>((set) => ({
  screen: { kind: "upload" },
  setScreen: (screen) => set({ screen }),

  files: [],
  addFiles: (files) => set((state) => ({ files: [...state.files, ...files] })),

  progress: new Map(),
  setProgress: (file, update) =>
    set((state) => {
      const next = new Map(state.progress);
      next.set(file, update);
      return { progress: next };
    }),
  clearProgress: () => set({ progress: new Map() }),

  results: [],
  addResult: (result) => set((state) => ({ results: [...state.results, result] })),
  setResults: (results) => set({ results }),

  config: { ...DEFAULT_CONFIG },
  setConfig: (partial) =>
    set((state) => ({ config: { ...state.config, ...partial } })),

  showConfig: false,
  setShowConfig: (show) => set({ showConfig: show }),

  error: null,
  setError: (error) => set({ error }),

  skipNotice: null,
  setSkipNotice: (notice) => set({ skipNotice: notice }),

  editedFindings: new Map(),
  setEditedFindings: (resultName, findings) =>
    set((state) => {
      const next = new Map(state.editedFindings);
      next.set(resultName, findings);
      return { editedFindings: next };
    }),

  redactedDrafts: new Map(),
  setRedactedDraft: (resultName, draft) =>
    set((state) => {
      const next = new Map(state.redactedDrafts);
      next.set(resultName, draft);
      return { redactedDrafts: next };
    }),

  previewUrls: new Map(),
  setPreviewUrl: (resultName, url) =>
    set((state) => {
      const next = new Map(state.previewUrls);
      next.set(resultName, url);
      return { previewUrls: next };
    }),

  contentHashes: new Map(),
  setContentHash: (fileName, hash) =>
    set((state) => {
      const next = new Map(state.contentHashes);
      next.set(fileName, hash);
      return { contentHashes: next };
    }),

  workerReady: false,
  setWorkerReady: (ready) => set({ workerReady: ready }),
}));
```

- [ ] **Step 3: Commit**

```bash
git add apps/hacienda-studio-next/src/lib/types.ts apps/hacienda-studio-next/src/lib/store.ts
git commit -m "feat: create Zustand store and TypeScript types for studio state"
```

---

### Task 9: Create Web Worker Pipeline

**Files:**
- Create: `apps/hacienda-studio-next/public/worker/pipeline.ts`
- Create: `apps/hacienda-studio-next/src/lib/wasm/worker-manager.ts`

**Interfaces:**
- Consumes: Existing `apps/hacienda-studio/worker/pipeline.ts` (reference), Task 8 (types)
- Produces: `initWorker()`, `processFiles()`, `buildZip()`

- [ ] **Step 1: Create worker-manager.ts**

```bash
mkdir -p src/lib/wasm
```

Create `src/lib/wasm/worker-manager.ts`:

```ts
let worker: Worker | null = null;
let readyPromise: Promise<void> | null = null;

export function initWorker(): Promise<void> {
  if (readyPromise) return readyPromise;

  worker = new Worker(new URL("../../public/worker/pipeline.ts", import.meta.url), {
    type: "module",
  });

  readyPromise = new Promise((resolve) => {
    worker!.onmessage = (e) => {
      if (e.data.type === "ready") resolve();
    };
    worker!.postMessage({ type: "init" });
  });

  return readyPromise;
}

export function getWorker(): Worker | null {
  return worker;
}

export function terminateWorker(): void {
  worker?.terminate();
  worker = null;
  readyPromise = null;
}

export function onWorkerMessage(handler: (event: MessageEvent) => void): void {
  if (worker) {
    worker.onmessage = handler;
  }
}

export function postToWorker(message: unknown): void {
  worker?.postMessage(message);
}
```

- [ ] **Step 2: Port pipeline.ts to public/worker/**

Copy and adapt `apps/hacienda-studio/worker/pipeline.ts` to `apps/hacienda-studio-next/public/worker/pipeline.ts`, updating imports:

```ts
// Key import changes:
import { XbergEngine, WasmExtractInput, WasmExtractionConfig, WasmOutputFormat, WasmChunkingConfig, WasmOcrConfig, WasmNerConfig, WasmNerBackendKind } from "@xberg-io/xberg-wasm";
import initWasm from "@xberg-io/xberg-wasm";
import xbergWasmUrl from "@xberg-io/xberg-wasm/pkg/web/xberg_wasm_bg.wasm?url";

// ... rest of pipeline logic unchanged
```

- [ ] **Step 3: Verify worker compiles**

```bash
cd apps/hacienda-studio-next
npx tsc --noEmit
```

- [ ] **Step 4: Commit**

```bash
git add apps/hacienda-studio-next/public/worker apps/hacienda-studio-next/src/lib/wasm/worker-manager.ts
git commit -m "feat: port Web Worker pipeline with manager for WASM processing"
```

---

### Task 10: Create Upload Screen

**Files:**
- Create: `apps/hacienda-studio-next/src/components/studio/UploadScreen.tsx`

**Interfaces:**
- Consumes: Task 8 (store), Task 9 (worker-manager)
- Produces: File upload drop zone with folder support

- [ ] **Step 1: Create UploadScreen**

Create `src/components/studio/UploadScreen.tsx`:

```tsx
"use client";

import { useCallback, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { useStudioStore } from "@/lib/store";
import { validateFile } from "@/lib/file-filter";
import { computeContentHash } from "@/lib/content-hash";

export function UploadScreen() {
  const { addFiles, setScreen, setProgress, setContentHash, setError, setSkipNotice, workerReady } = useStudioStore();
  const [folderMode, setFolderMode] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const handleFiles = useCallback(async (fileList: FileList | File[]) => {
    const fileArray = Array.from(fileList);
    const validFiles: File[] = [];
    let skippedJunk = 0;
    const unsupported: string[] = [];

    for (const file of fileArray) {
      const validation = validateFile(file);
      if (!validation.valid) {
        unsupported.push(validation.error || "Invalid file");
        continue;
      }
      validFiles.push(file);
    }

    const totalSkipped = unsupported.length + skippedJunk;
    if (fileArray.length === 1 && unsupported.length === 1) {
      setError(unsupported[0]);
    } else if (totalSkipped > 0) {
      setSkipNotice(`Skipped ${totalSkipped} unsupported file${totalSkipped === 1 ? "" : "s"}.`);
    }

    if (validFiles.length === 0) return;

    addFiles(validFiles);

    const fileInputs = await Promise.all(
      validFiles.map(async (f) => {
        const bytes = await f.arrayBuffer();
        const hash = await computeContentHash(bytes);
        return { name: f.name, bytes, type: f.type || "application/octet-stream", hash };
      })
    );

    for (const fi of fileInputs) {
      setContentHash(fi.name, fi.hash);
    }

    setScreen({ kind: "queue" });
  }, [addFiles, setScreen, setContentHash, setError, setSkipNotice]);

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    const items = e.dataTransfer.items;
    if (items) {
      const files: File[] = [];
      for (let i = 0; i < items.length; i++) {
        const entry = items[i].webkitGetAsEntry?.();
        if (entry?.isDirectory) {
          // Handle folder
          const reader = entry.createReader();
          reader.readEntries((entries) => {
            entries.forEach((entry) => {
              if (entry.isFile) {
                entry.file((file) => files.push(file));
              }
            });
          });
        } else {
          const file = items[i].getAsFile();
          if (file) files.push(file);
        }
      }
      setTimeout(() => handleFiles(files), 100);
    } else {
      handleFiles(e.dataTransfer.files);
    }
  }, [handleFiles]);

  return (
    <section aria-label="Upload" className="mx-auto w-full max-w-[600px] flex-1 px-6 py-12">
      <div
        className="flex flex-col items-center justify-center gap-4 rounded-lg border-2 border-dashed border-muted-foreground/25 p-12 text-center transition-colors hover:border-primary/50"
        onDragOver={(e) => e.preventDefault()}
        onDrop={handleDrop}
      >
        <p className="text-lg text-muted-foreground">
          Drag and drop files or folders here
        </p>
        <p className="text-sm text-muted-foreground">
          or
        </p>
        <div className="flex gap-2">
          <Button
            onClick={() => {
              setFolderMode(false);
              fileInputRef.current?.click();
            }}
            disabled={!workerReady}
          >
            Select Files
          </Button>
          <Button
            variant="outline"
            onClick={() => {
              setFolderMode(true);
              fileInputRef.current?.click();
            }}
            disabled={!workerReady}
          >
            Select Folder
          </Button>
        </div>
        <input
          ref={fileInputRef}
          type="file"
          multiple
          className="hidden"
          // @ts-ignore
          webkitdirectory={folderMode || undefined}
          onChange={(e) => e.target.files && handleFiles(e.target.files)}
        />
      </div>
    </section>
  );
}
```

- [ ] **Step 2: Create file-filter utility**

Create `src/lib/file-filter.ts`:

```ts
const JUNK_PATTERNS = /^\./;

export function isJunkFile(name: string): boolean {
  return JUNK_PATTERNS.test(name);
}

export function validateFile(file: File): { valid: boolean; error?: string } {
  const SUPPORTED_TYPES = [
    "application/pdf",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    "text/plain",
    "text/markdown",
    "text/csv",
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "audio/",
    "video/",
  ];

  if (SUPPORTED_TYPES.some((t) => file.type.startsWith(t))) {
    return { valid: true };
  }

  return { valid: false, error: `Unsupported file type: ${file.type || "unknown"}` };
}
```

- [ ] **Step 3: Create content-hash utility**

Create `src/lib/content-hash.ts`:

```ts
export async function computeContentHash(bytes: ArrayBuffer): Promise<string> {
  const hashBuffer = await crypto.subtle.digest("SHA-256", bytes);
  const hashArray = Array.from(new Uint8Array(hashBuffer));
  return hashArray.map((b) => b.toString(16).padStart(2, "0")).join("");
}
```

- [ ] **Step 4: Commit**

```bash
git add apps/hacienda-studio-next/src/components/studio/UploadScreen.tsx apps/hacienda-studio-next/src/lib/file-filter.ts apps/hacienda-studio-next/src/lib/content-hash.ts
git commit -m "feat: create upload screen with drag-and-drop and file validation"
```

---

### Task 11: Create Queue and File Browser Screens

**Files:**
- Create: `apps/hacienda-studio-next/src/components/studio/QueueScreen.tsx`
- Create: `apps/hacienda-studio-next/src/components/studio/FileBrowserScreen.tsx`

**Interfaces:**
- Consumes: Task 8 (store), Task 9 (worker-manager)
- Produces: Queue with progress, File browser with extend-hq/ui

- [ ] **Step 1: Create QueueScreen**

Create `src/components/studio/QueueScreen.tsx`:

```tsx
"use client";

import { Progress } from "@/components/ui/progress";
import { useStudioStore } from "@/lib/store";

export function QueueScreen() {
  const { files, progress } = useStudioStore();

  return (
    <section aria-label="Processing queue" className="mx-auto w-full max-w-[800px] flex-1 px-6 py-12">
      <h2 className="text-lg font-semibold mb-4">Processing {files.length} files...</h2>
      <div className="space-y-4">
        {files.map((file) => {
          const update = progress.get(file.name);
          return (
            <div key={file.name} className="space-y-2">
              <div className="flex justify-between text-sm">
                <span className="truncate">{file.name}</span>
                <span className="text-muted-foreground">
                  {update ? `${update.stage} ${update.percent}%` : "Queued..."}
                </span>
              </div>
              {update && <Progress value={update.percent} className="h-2" />}
            </div>
          );
        })}
      </div>
    </section>
  );
}
```

- [ ] **Step 2: Create FileBrowserScreen**

Create `src/components/studio/FileBrowserScreen.tsx`:

```tsx
"use client";

import { Button } from "@/components/ui/button";
import { useStudioStore } from "@/lib/store";
import { FileBrowser, type FileRow } from "@/components/FileBrowser";

export function FileBrowserScreen() {
  const { files, progress, results, editedFindings, setScreen } = useStudioStore();

  const rows: FileRow[] = files.map((file) => {
    const result = results.find((r) => r.frontmatter.source === file.name);
    const update = progress.get(file.name);
    return {
      name: file.name,
      status: update?.stage ?? "queued",
      percent: update?.percent ?? 0,
      openable: !!result,
      edited: editedFindings.has(result?.name ?? ""),
      result,
    };
  });

  return (
    <section aria-label="File browser" className="mx-auto w-full max-w-[800px] flex-1 px-6 py-12">
      <div className="flex items-center justify-between gap-2 mb-4">
        <h2 className="text-sm font-semibold text-muted-foreground">Files ({rows.length})</h2>
        <div className="flex items-center gap-2">
          <Button
            variant="default"
            size="sm"
            className="download-zip"
            disabled={!rows.some((r) => r.openable)}
          >
            Download redacted zip
          </Button>
          <Button
            variant="outline"
            size="sm"
            className="add-more-files"
            onClick={() => setScreen({ kind: "upload" })}
          >
            Add more files
          </Button>
        </div>
      </div>
      <FileBrowser rows={rows} openName={null} onOpen={(name) => setScreen({ kind: "detail", inputName: name })} />
    </section>
  );
}
```

- [ ] **Step 3: Create FileBrowser component**

Create `src/components/FileBrowser.tsx`:

```tsx
"use client";

import { Button } from "@/components/ui/button";
import type { ProcessedFile } from "@/lib/types";

export interface FileRow {
  name: string;
  status: string;
  percent: number;
  openable: boolean;
  edited: boolean;
  result?: ProcessedFile;
}

interface FileBrowserProps {
  rows: FileRow[];
  openName: string | null;
  onOpen: (name: string) => void;
}

export function FileBrowser({ rows, openName, onOpen }: FileBrowserProps) {
  return (
    <div className="space-y-2">
      {rows.map((row) => (
        <div
          key={row.name}
          className={`flex items-center justify-between rounded-lg border p-3 ${
            openName === row.name ? "border-primary" : ""
          }`}
        >
          <div className="flex items-center gap-3">
            <span className="font-medium">{row.name}</span>
            {row.edited && (
              <span className="text-xs bg-primary/10 text-primary px-2 py-0.5 rounded">edited</span>
            )}
          </div>
          <div className="flex items-center gap-2">
            <span className="text-xs text-muted-foreground">{row.status} {row.percent}%</span>
            {row.openable && (
              <Button
                variant="ghost"
                size="sm"
                onClick={() => onOpen(row.name)}
              >
                Open
              </Button>
            )}
          </div>
        </div>
      ))}
    </div>
  );
}
```

- [ ] **Step 4: Commit**

```bash
git add apps/hacienda-studio-next/src/components/studio/QueueScreen.tsx apps/hacienda-studio-next/src/components/studio/FileBrowserScreen.tsx apps/hacienda-studio-next/src/components/FileBrowser.tsx
git commit -m "feat: create queue screen and file browser with extend-hq/ui integration"
```

---

### Task 12: Create Detail Screen

**Files:**
- Create: `apps/hacienda-studio-next/src/components/studio/DetailScreen.tsx`

**Interfaces:**
- Consumes: Task 8 (store), extend-hq/ui viewers, custom editors
- Produces: Split view with native viewer (left) + tabs (right)

- [ ] **Step 1: Create DetailScreen**

Create `src/components/studio/DetailScreen.tsx`:

```tsx
"use client";

import { ArrowLeftIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { PDFViewer } from "@/components/extend/pdf-viewer";
import { DocxViewer } from "@/components/extend/docx-viewer";
import { XlsxViewer } from "@/components/extend/xlsx-viewer";
import { PptxViewer } from "@/components/extend/pptx-viewer";
import { MarkdownEditor } from "@/components/MarkdownEditor";
import { RedactedEditor } from "@/components/RedactedEditor";
import { PiiPanel } from "@/components/PiiPanel";
import { useStudioStore } from "@/lib/store";

export function DetailScreen() {
  const { screen, results, previewUrls, setScreen, redactedDrafts, setRedactedDraft, editedFindings } = useStudioStore();

  if (screen.kind !== "detail") return null;

  const result = results.find((r) => r.frontmatter.source === screen.inputName);
  if (!result) return null;

  const previewUrl = previewUrls.get(result.name);
  const findings = editedFindings.get(result.name) ?? result.piiFindings;
  const redactedDraft = redactedDrafts.get(result.name) ?? result.markdown;

  const viewerKind = getViewerKind(result.frontmatter.source);

  return (
    <section aria-label="File detail" className="mx-auto w-full max-w-[1100px] flex-1 px-6 py-12">
      <div className="flex items-center justify-between text-sm mb-4">
        <Button variant="ghost" size="sm" onClick={() => setScreen({ kind: "browser" })}>
          <ArrowLeftIcon /> Back
        </Button>
        <span className="font-medium">{result.name}</span>
        <div className="flex items-center gap-2">
          <span className="text-muted-foreground">
            {result.entities.length} {result.entities.length === 1 ? "entity" : "entities"}
          </span>
          <Button size="sm" className="download-zip">Download zip</Button>
        </div>
      </div>

      {viewerKind && previewUrl ? (
        <ResizablePanelGroup orientation="horizontal" className="split-view h-[70vh] rounded-lg border">
          <ResizablePanel defaultSize="50" minSize="25" className="min-w-0 overflow-auto p-2">
            {viewerKind === "pdf" && <PDFViewer file={previewUrl} />}
            {viewerKind === "docx" && <DocxViewer src={previewUrl} />}
            {viewerKind === "xlsx" && <XlsxViewer src={previewUrl} />}
            {viewerKind === "pptx" && <PptxViewer src={previewUrl} />}
          </ResizablePanel>
          <ResizableHandle withHandle />
          <ResizablePanel defaultSize="50" minSize="25" className="min-w-0 overflow-auto p-2">
            <Tabs defaultValue="redacted" className="h-full">
              <TabsList>
                <TabsTrigger value="redacted">Redacted Markdown</TabsTrigger>
                <TabsTrigger value="findings">Findings</TabsTrigger>
              </TabsList>
              <TabsContent value="redacted">
                <RedactedEditor
                  value={redactedDraft}
                  onChange={(next) => setRedactedDraft(result.name, next)}
                />
              </TabsContent>
              <TabsContent value="findings">
                <MarkdownEditor value={result.rawMarkdown} findings={findings} />
                {findings.length > 0 && <PiiPanel findings={findings} />}
              </TabsContent>
            </Tabs>
          </ResizablePanel>
        </ResizablePanelGroup>
      ) : (
        <div className="mt-4">
          <MarkdownEditor value={result.rawMarkdown} findings={findings} />
          {findings.length > 0 && <PiiPanel findings={findings} />}
        </div>
      )}
    </section>
  );
}

function getViewerKind(source: string): string | null {
  const ext = source.split(".").pop()?.toLowerCase();
  switch (ext) {
    case "pdf": return "pdf";
    case "docx": case "doc": return "docx";
    case "xlsx": case "xls": return "xlsx";
    case "pptx": case "ppt": return "pptx";
    default: return null;
  }
}
```

- [ ] **Step 2: Create placeholder editor components**

Create `src/components/MarkdownEditor.tsx`:

```tsx
"use client";

import type { PiiEntity } from "@/lib/types";

interface MarkdownEditorProps {
  value: string;
  findings?: PiiEntity[];
}

export function MarkdownEditor({ value, findings }: MarkdownEditorProps) {
  return (
    <div className="cm-editor rounded border p-4 font-mono text-sm overflow-auto max-h-[60vh]">
      <pre className="whitespace-pre-wrap">{value}</pre>
    </div>
  );
}
```

Create `src/components/RedactedEditor.tsx`:

```tsx
"use client";

interface RedactedEditorProps {
  value: string;
  onChange: (value: string) => void;
}

export function RedactedEditor({ value, onChange }: RedactedEditorProps) {
  return (
    <textarea
      className="cm-redacted-editor w-full h-full min-h-[400px] font-mono text-sm p-4 border rounded resize-none"
      value={value}
      onChange={(e) => onChange(e.target.value)}
    />
  );
}
```

Create `src/components/PiiPanel.tsx`:

```tsx
"use client";

import type { PiiEntity } from "@/lib/types";

interface PiiPanelProps {
  findings: PiiEntity[];
}

export function PiiPanel({ findings }: PiiPanelProps) {
  return (
    <div className="space-y-2 mt-4">
      <h3 className="text-sm font-semibold">PII Findings ({findings.length})</h3>
      <div className="space-y-1">
        {findings.map((f, i) => (
          <div key={i} className="flex items-center justify-between text-xs p-2 rounded bg-muted">
            <span className="font-mono">{f.category}</span>
            <span className="text-muted-foreground">{f.confidence.toFixed(2)}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
```

- [ ] **Step 3: Commit**

```bash
git add apps/hacienda-studio-next/src/components/studio/DetailScreen.tsx apps/hacienda-studio-next/src/components/MarkdownEditor.tsx apps/hacienda-studio-next/src/components/RedactedEditor.tsx apps/hacienda-studio-next/src/components/PiiPanel.tsx
git commit -m "feat: create detail screen with split view and placeholder editors"
```

---

### Task 13: Create Main Studio Page

**Files:**
- Create: `apps/hacienda-studio-next/src/app/(dashboard)/studio/page.tsx`
- Create: `apps/hacienda-studio-next/src/app/(dashboard)/layout.tsx`

**Interfaces:**
- Consumes: Tasks 7-12 (all studio components)
- Produces: Complete studio page with screen routing

- [ ] **Step 1: Create dashboard layout**

```bash
mkdir -p "src/app/(dashboard)"
```

Create `src/app/(dashboard)/layout.tsx`:

```tsx
import type { ReactNode } from "react";
import Link from "next/link";

export default function DashboardLayout({ children }: { children: ReactNode }) {
  return (
    <div className="flex min-h-screen flex-col">
      <header className="flex items-center justify-between border-b border-border bg-card px-6 py-3">
        <Link href="/studio" className="text-xl font-semibold">
          Hacienda Studio
        </Link>
        <nav className="flex items-center gap-4">
          <Link href="/studio" className="text-sm text-muted-foreground hover:text-foreground">
            Studio
          </Link>
          <Link href="/settings" className="text-sm text-muted-foreground hover:text-foreground">
            Settings
          </Link>
        </nav>
      </header>
      <main className="flex flex-1 flex-col">{children}</main>
    </div>
  );
}
```

- [ ] **Step 2: Create studio page**

```bash
mkdir -p "src/app/(dashboard)/studio"
```

Create `src/app/(dashboard)/studio/page.tsx`:

```tsx
"use client";

import dynamic from "next/dynamic";
import { useEffect } from "react";
import { useRouter } from "next/navigation";
import { useStudioStore } from "@/lib/store";
import { initWorker, onWorkerMessage, postToWorker } from "@/lib/wasm/worker-manager";
import { UploadScreen } from "@/components/studio/UploadScreen";
import { QueueScreen } from "@/components/studio/QueueScreen";
import { FileBrowserScreen } from "@/components/studio/FileBrowserScreen";

const DetailScreen = dynamic(
  () => import("@/components/studio/DetailScreen").then((m) => m.DetailScreen),
  { ssr: false }
);

export default function StudioPage() {
  const router = useRouter();
  const { screen, setScreen, setWorkerReady, addResult, setProgress, error, setError, skipNotice, setSkipNotice } = useStudioStore();

  useEffect(() => {
    const visited = localStorage.getItem("hacienda-studio-visited");
    if (!visited) {
      router.push("/studio/onboarding");
      return;
    }

    let cancelled = false;

    async function init() {
      await initWorker();
      if (cancelled) return;

      setWorkerReady(true);

      onWorkerMessage((event) => {
        const { type, ...data } = event.data;
        switch (type) {
          case "progress":
            setProgress(data.file, data);
            break;
          case "file-complete":
            addResult(data);
            setProgress(data.name, { ...data, stage: "complete", percent: 100 });
            break;
          case "batch-complete":
            setScreen({ kind: "browser" });
            break;
          case "error":
            setError(`${data.file}: ${data.message}`);
            break;
        }
      });
    }

    init();
    return () => { cancelled = true; };
  }, [router, setWorkerReady, addResult, setProgress, setScreen, setError]);

  return (
    <>
      {error && (
        <div className="flex items-center justify-between bg-destructive/15 px-6 py-3 text-destructive" role="alert">
          <span>{error}</span>
          <button onClick={() => setError(null)} className="text-lg leading-none">✕</button>
        </div>
      )}

      {skipNotice && (
        <div className="flex items-center justify-between border-b border-border bg-card px-6 py-3 text-sm text-muted-foreground" role="status">
          <span>{skipNotice}</span>
          <button onClick={() => setSkipNotice(null)} className="text-lg leading-none">✕</button>
        </div>
      )}

      <main className="flex flex-1 flex-col">
        {screen.kind === "upload" && <UploadScreen />}
        {screen.kind === "queue" && <QueueScreen />}
        {screen.kind === "browser" && <FileBrowserScreen />}
        {screen.kind === "detail" && <DetailScreen />}
      </main>
    </>
  );
}
```

- [ ] **Step 3: Commit**

```bash
git add "apps/hacienda-studio-next/src/app/(dashboard)"
git commit -m "feat: create main studio page with screen routing and worker integration"
```

---

## Phase 4: Editors & PII Features

### Task 14: Enhance MarkdownEditor with CodeMirror 6

**Files:**
- Modify: `apps/hacienda-studio-next/src/components/MarkdownEditor.tsx`

**Interfaces:**
- Consumes: Task 12 (DetailScreen), Task 8 (types)
- Produces: CodeMirror 6 editor with PII finding decorations

- [ ] **Step 1: Install CodeMirror dependencies**

```bash
cd apps/hacienda-studio-next
pnpm add @codemirror/lang-markdown @codemirror/state @codemirror/view @codemirror/theme-one-dark @uiw/react-codemirror
```

- [ ] **Step 2: Update MarkdownEditor**

Replace `src/components/MarkdownEditor.tsx`:

```tsx
"use client";

import CodeMirror from "@uiw/react-codemirror";
import { markdown } from "@codemirror/lang-markdown";
import { oneDark } from "@codemirror/theme-one-dark";
import { EditorView } from "@codemirror/view";
import type { PiiEntity } from "@/lib/types";

interface MarkdownEditorProps {
  value: string;
  findings?: PiiEntity[];
}

export function MarkdownEditor({ value, findings }: MarkdownEditorProps) {
  const extensions = [
    markdown(),
    EditorView.lineWrapping,
  ];

  return (
    <div className="cm-editor rounded border overflow-auto max-h-[60vh]">
      <CodeMirror
        value={value}
        extensions={extensions}
        theme={oneDark}
        readOnly
        basicSetup={{ lineNumbers: false }}
      />
    </div>
  );
}
```

- [ ] **Step 3: Commit**

```bash
git add apps/hacienda-studio-next/src/components/MarkdownEditor.tsx
git commit -m "feat: enhance MarkdownEditor with CodeMirror 6"
```

---

## Phase 5: Server Sync & Auth Integration

### Task 15: Create Sync Engine

**Files:**
- Create: `apps/hacienda-studio-next/src/lib/sync/sync-engine.ts`
- Create: `apps/hacienda-studio-next/src/lib/sync/queue.ts`

**Interfaces:**
- Consumes: Task 5 (Prisma), Task 8 (store)
- Produces: Background sync of documents, audit entries, drafts to server

- [ ] **Step 1: Create sync queue**

Create `src/lib/sync/queue.ts`:

```ts
interface SyncOperation {
  type: "document" | "audit" | "draft";
  action: "upsert" | "append";
  data: unknown;
  timestamp: number;
}

const QUEUE_KEY = "hacienda-sync-queue";

export function getSyncQueue(): SyncOperation[] {
  if (typeof window === "undefined") return [];
  const raw = localStorage.getItem(QUEUE_KEY);
  return raw ? JSON.parse(raw) : [];
}

export function addToSyncQueue(op: SyncOperation): void {
  const queue = getSyncQueue();
  queue.push(op);
  localStorage.setItem(QUEUE_KEY, JSON.stringify(queue));
}

export function clearSyncQueue(): void {
  localStorage.removeItem(QUEUE_KEY);
}
```

- [ ] **Step 2: Create sync engine**

Create `src/lib/sync/sync-engine.ts`:

```ts
import { addToSyncQueue, getSyncQueue, clearSyncQueue } from "./queue";

let syncing = false;

export async function syncToServer(): Promise<void> {
  if (syncing || typeof window === "undefined") return;

  const session = await fetch("/api/auth/session").then((r) => r.json());
  if (!session?.user) return;

  syncing = true;
  const queue = getSyncQueue();

  try {
    for (const op of queue) {
      await fetch("/api/sync", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(op),
      });
    }
    clearSyncQueue();
  } catch (e) {
    console.error("Sync failed:", e);
  } finally {
    syncing = false;
  }
}

export function startSyncInterval(): NodeJS.Timeout {
  return setInterval(syncToServer, 30000);
}
```

- [ ] **Step 3: Create sync API route**

```bash
mkdir -p src/app/api/sync
```

Create `src/app/api/sync/route.ts`:

```ts
import { NextResponse } from "next/server";
import { auth } from "@/lib/auth/config";
import { prisma } from "@/lib/db/client";

export async function POST(request: Request) {
  const session = await auth();
  if (!session?.user) {
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }

  const body = await request.json();

  switch (body.type) {
    case "document":
      await prisma.document.upsert({
        where: { contentHash: body.data.contentHash },
        update: { markdown: body.data.markdown, rawMarkdown: body.data.rawMarkdown },
        create: {
          userId: session.user.id,
          name: body.data.name,
          originalType: body.data.originalType,
          markdown: body.data.markdown,
          rawMarkdown: body.data.rawMarkdown,
          contentHash: body.data.contentHash,
          piiCount: body.data.piiCount,
          entityCount: body.data.entityCount,
          entities: body.data.entities,
        },
      });
      break;

    case "audit":
      await prisma.auditEntry.create({
        data: {
          userId: session.user.id,
          documentId: body.data.documentId,
          category: body.data.category,
          action: body.data.action,
          spanHash: body.data.spanHash,
          spanLength: body.data.spanLength,
          confidence: body.data.confidence,
          source: body.data.source,
          pipelineVersion: body.data.pipelineVersion,
        },
      });
      break;

    case "draft":
      await prisma.draft.upsert({
        where: { userId_documentId: { userId: session.user.id, documentId: body.data.documentId } },
        update: { content: body.data.content, contentHash: body.data.contentHash },
        create: {
          userId: session.user.id,
          documentId: body.data.documentId,
          content: body.data.content,
          contentHash: body.data.contentHash,
        },
      });
      break;
  }

  return NextResponse.json({ ok: true });
}
```

- [ ] **Step 4: Commit**

```bash
git add apps/hacienda-studio-next/src/lib/sync apps/hacienda-studio-next/src/app/api/sync
git commit -m "feat: create sync engine with queue and API route"
```

---

## Phase 6: Polish & Deploy

### Task 16: Create Login Page

**Files:**
- Create: `apps/hacienda-studio-next/src/app/(auth)/login/page.tsx`
- Create: `apps/hacienda-studio-next/src/app/(auth)/layout.tsx`

**Interfaces:**
- Consumes: Task 3 (NextAuth)
- Produces: Login form with email/password and OAuth buttons

- [ ] **Step 1: Create auth layout**

```bash
mkdir -p "src/app/(auth)/login"
```

Create `src/app/(auth)/layout.tsx`:

```tsx
import type { ReactNode } from "react";

export default function AuthLayout({ children }: { children: ReactNode }) {
  return (
    <div className="flex min-h-screen items-center justify-center">
      {children}
    </div>
  );
}
```

- [ ] **Step 2: Create login page**

Create `src/app/(auth)/login/page.tsx`:

```tsx
"use client";

import { signIn } from "next-auth/react";
import { useState } from "react";
import { useRouter } from "next/navigation";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";

export default function LoginPage() {
  const router = useRouter();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setLoading(true);
    setError(null);

    const result = await signIn("credentials", { email, password, redirect: false });

    if (result?.error) {
      setError("Invalid email or password");
      setLoading(false);
    } else {
      router.push("/studio");
    }
  }

  return (
    <Card className="w-full max-w-md">
      <CardHeader>
        <CardTitle>Sign in to Hacienda Studio</CardTitle>
        <CardDescription>Enter your credentials to continue</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="email">Email</Label>
            <Input id="email" type="email" value={email} onChange={(e) => setEmail(e.target.value)} required />
          </div>
          <div className="space-y-2">
            <Label htmlFor="password">Password</Label>
            <Input id="password" type="password" value={password} onChange={(e) => setPassword(e.target.value)} required />
          </div>
          {error && <p className="text-sm text-destructive">{error}</p>}
          <Button type="submit" className="w-full" disabled={loading}>
            {loading ? "Signing in..." : "Sign in"}
          </Button>
        </form>
        <div className="relative">
          <div className="absolute inset-0 flex items-center">
            <span className="w-full border-t" />
          </div>
          <div className="relative flex justify-center text-xs uppercase">
            <span className="bg-card px-2 text-muted-foreground">Or continue with</span>
          </div>
        </div>
        <div className="grid grid-cols-2 gap-4">
          <Button variant="outline" onClick={() => signIn("github")}>GitHub</Button>
          <Button variant="outline" onClick={() => signIn("google")}>Google</Button>
        </div>
      </CardContent>
    </Card>
  );
}
```

- [ ] **Step 3: Commit**

```bash
git add "apps/hacienda-studio-next/src/app/(auth)"
git commit -m "feat: create login page with email/password and OAuth"
```

---

### Task 17: Configure Vercel Deployment

**Files:**
- Modify: `apps/hacienda-studio-next/vercel.json`
- Create: `apps/hacienda-studio-next/.gitignore` (update)

**Interfaces:**
- Consumes: All previous tasks
- Produces: Vercel-ready deployment configuration

- [ ] **Step 1: Create vercel.json**

```json
{
  "buildCommand": "pnpm build",
  "outputDirectory": ".next",
  "framework": "nextjs",
  "installCommand": "pnpm install"
}
```

- [ ] **Step 2: Update .gitignore**

Add to `.gitignore`:

```
# WASM build output
public/wasm/hacienda-wasm/*.wasm

# Environment
.env.local
.env*.local

# Prisma
prisma/migrations/

# Next.js
.next/
out/
```

- [ ] **Step 3: Commit**

```bash
git add apps/hacienda-studio-next/vercel.json apps/hacienda-studio-next/.gitignore
git commit -m "feat: configure Vercel deployment settings"
```

---

### Task 18: Create Settings Page

**Files:**
- Create: `apps/hacienda-studio-next/src/app/(dashboard)/settings/page.tsx`

**Interfaces:**
- Consumes: Task 5 (Prisma), Task 3 (NextAuth)
- Produces: User settings page for profile and preferences

- [ ] **Step 1: Create settings page**

```bash
mkdir -p "src/app/(dashboard)/settings"
```

Create `src/app/(dashboard)/settings/page.tsx`:

```tsx
"use client";

import { useSession } from "next-auth/react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";

export default function SettingsPage() {
  const { data: session } = useSession();

  return (
    <div className="mx-auto w-full max-w-[800px] flex-1 px-6 py-12">
      <h1 className="text-2xl font-bold mb-6">Settings</h1>

      <Card>
        <CardHeader>
          <CardTitle>Profile</CardTitle>
          <CardDescription>Your account information</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div>
            <p className="text-sm text-muted-foreground">Name</p>
            <p className="font-medium">{session?.user?.name ?? "Not set"}</p>
          </div>
          <div>
            <p className="text-sm text-muted-foreground">Email</p>
            <p className="font-medium">{session?.user?.email ?? "Not set"}</p>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
```

- [ ] **Step 2: Commit**

```bash
git add "apps/hacienda-studio-next/src/app/(dashboard)/settings"
git commit -m "feat: create settings page with user profile display"
```

---

## Implementation Summary

| Phase | Tasks | Description |
|-------|-------|-------------|
| Phase 1 | Tasks 1-5 | Foundation: Next.js, shadcn, extend-hq/ui, NextAuth, WASM, Prisma |
| Phase 2 | Tasks 6-7 | Onboarding & asset loading with progress |
| Phase 3 | Tasks 8-13 | Studio core: store, worker, upload, queue, browser, detail |
| Phase 4 | Task 14 | Enhance MarkdownEditor with CodeMirror 6 |
| Phase 5 | Task 15 | Server sync engine |
| Phase 6 | Tasks 16-18 | Polish: login page, Vercel config, settings |

**Total Tasks:** 18
**Estimated Time:** 8-12 hours for an experienced developer

---

*This plan was generated from the design spec at `docs/superpowers/specs/2026-08-10-hacienda-studio-nextjs-rebuild-design.md`.*
