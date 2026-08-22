// Cold-start caching for Wasm modules (Tier 2.2)
// Provides explicit caching of compiled Wasm modules using Cache API / IndexedDB
// to skip recompilation on repeat visits, reinforcing browser's own compile cache.

import type { ProgressUpdate } from "./types";

interface CacheConfig {
  /** Cache name for Wasm modules */
  cacheName: string;
  /** Maximum age of cached entries in milliseconds (default: 30 days) */
  maxAgeMs: number;
}

const DEFAULT_CACHE_CONFIG: CacheConfig = {
  cacheName: "hacienda-wasm-cache",
  maxAgeMs: 30 * 24 * 60 * 60 * 1000, // 30 days
};

/**
 * Cache a Wasm module response using the Cache API
 * Falls back to IndexedDB if Cache API unavailable
 */
export async function cacheWasmModule(
  url: string,
  response: Response,
  config: Partial<CacheConfig> = {}
): Promise<void> {
  const { cacheName, maxAgeMs } = { ...DEFAULT_CACHE_CONFIG, ...config };
  
  try {
    // Try Cache API first (modern browsers) — `globalThis` works in both Window and Worker
    const g: any = globalThis as any;
    if (typeof g.caches !== "undefined" && g.caches) {
      const cache = await g.caches.open(cacheName);
      const clonedResponse = response.clone();
      
      // Add cache metadata headers
      const headers = new Headers(clonedResponse.headers);
      headers.set("x-cached-at", Date.now().toString());
      headers.set("x-cache-max-age", maxAgeMs.toString());
      
      const cachedResponse = new Response(await clonedResponse.blob(), {
        status: clonedResponse.status,
        statusText: clonedResponse.statusText,
        headers,
      });
      
      await cache.put(url, cachedResponse);
      console.log(`[WasmCache] Cached ${url} via Cache API`);
      return;
    }
  } catch (e) {
    console.warn("[WasmCache] Cache API failed, falling back to IndexedDB:", e);
  }

  // Fallback to IndexedDB
  await cacheWasmModuleIDB(url, response, { cacheName, maxAgeMs });
}

/**
 * Cache Wasm module using IndexedDB as fallback
 */
async function cacheWasmModuleIDB(
  url: string,
  response: Response,
  config: CacheConfig
): Promise<void> {
  const dbName = config.cacheName;
  const storeName = "wasm-modules";
  
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(dbName, 1);
    
    request.onupgradeneeded = (event) => {
      const db = (event.target as IDBOpenDBRequest).result;
      if (!db.objectStoreNames.contains(storeName)) {
        db.createObjectStore(storeName, { keyPath: "url" });
      }
    };
    
    request.onsuccess = async (event) => {
      const db = (event.target as IDBOpenDBRequest).result;
      try {
        const blob = await response.blob();
        const arrayBuffer = await blob.arrayBuffer();
        
        const transaction = db.transaction(storeName, "readwrite");
        const store = transaction.objectStore(storeName);
        
        await store.put({
          url,
          data: arrayBuffer,
          contentType: response.headers.get("content-type") || "application/wasm",
          cachedAt: Date.now(),
          maxAge: config.maxAgeMs,
        });
        
        transaction.oncomplete = () => {
          db.close();
          console.log(`[WasmCache] Cached ${url} via IndexedDB`);
          resolve();
        };
        transaction.onerror = () => reject(transaction.error);
      } catch (e) {
        reject(e);
      }
    };
    
    request.onerror = () => reject(request.error);
  });
}

/**
 * Retrieve a cached Wasm module response
 * Returns null if not cached or expired
 */
export async function getCachedWasmModule(
  url: string,
  config: Partial<CacheConfig> = {}
): Promise<Response | null> {
  const { cacheName, maxAgeMs } = { ...DEFAULT_CACHE_CONFIG, ...config };
  
  // Try Cache API first — Worker global is `self`, not `window`
  try {
    const g: any = globalThis as any;
    if (typeof g.caches !== "undefined" && g.caches) {
      const cache = await g.caches.open(cacheName);
      const cachedResponse = await cache.match(url);
      
      if (cachedResponse) {
        const cachedAt = parseInt(cachedResponse.headers.get("x-cached-at") || "0", 10);
        const cacheMaxAge = parseInt(cachedResponse.headers.get("x-cache-max-age") || maxAgeMs.toString(), 10);
        
        if (Date.now() - cachedAt < cacheMaxAge) {
          console.log(`[WasmCache] Cache hit for ${url} (Cache API)`);
          return cachedResponse;
        } else {
          // Expired - delete and fall through
          await cache.delete(url);
          console.log(`[WasmCache] Cache expired for ${url}`);
        }
      }
    }
  } catch (e) {
    console.warn("[WasmCache] Cache API read failed:", e);
  }

  // Fallback to IndexedDB
  return getCachedWasmModuleIDB(url, { cacheName, maxAgeMs });
}

/**
 * Retrieve cached Wasm module from IndexedDB
 */
async function getCachedWasmModuleIDB(
  url: string,
  config: CacheConfig
): Promise<Response | null> {
  const dbName = config.cacheName;
  const storeName = "wasm-modules";
  
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(dbName, 1);
    
    request.onupgradeneeded = (event) => {
      const db = (event.target as IDBOpenDBRequest).result;
      if (!db.objectStoreNames.contains(storeName)) {
        db.createObjectStore(storeName, { keyPath: "url" });
      }
    };
    
    request.onsuccess = (event) => {
      const db = (event.target as IDBOpenDBRequest).result;
      const transaction = db.transaction(storeName, "readonly");
      const store = transaction.objectStore(storeName);
      const getRequest = store.get(url);
      
      getRequest.onsuccess = () => {
        db.close();
        const entry = getRequest.result;
        
        if (!entry) {
          resolve(null);
          return;
        }
        
        // Check expiration
        if (Date.now() - entry.cachedAt >= entry.maxAge) {
          // Expired - delete and return null
          const deleteDb = indexedDB.open(dbName, 1);
          deleteDb.onsuccess = (e) => {
            const delDb = (e.target as IDBOpenDBRequest).result;
            const delTx = delDb.transaction(storeName, "readwrite");
            delTx.objectStore(storeName).delete(url);
            delTx.oncomplete = () => delDb.close();
          };
          resolve(null);
          return;
        }
        
        // Create response from cached data
        const blob = new Blob([entry.data], { type: entry.contentType });
        const response = new Response(blob, {
          status: 200,
          statusText: "OK",
          headers: {
            "content-type": entry.contentType,
            "x-from-cache": "indexeddb",
          },
        });
        
        console.log(`[WasmCache] Cache hit for ${url} (IndexedDB)`);
        resolve(response);
      };
      
      getRequest.onerror = () => {
        db.close();
        resolve(null);
      };
    };
    
    request.onerror = () => resolve(null);
  });
}

/**
 * Clear all cached Wasm modules
 */
export async function clearWasmCache(config: Partial<CacheConfig> = {}): Promise<void> {
  const { cacheName } = { ...DEFAULT_CACHE_CONFIG, ...config };
  
  try {
    const g: any = globalThis as any;
    if (typeof g.caches !== "undefined" && g.caches) {
      await g.caches.delete(cacheName);
      console.log("[WasmCache] Cleared Cache API cache");
    }
  } catch (e) {
    console.warn("[WasmCache] Failed to clear Cache API:", e);
  }
  
  // Clear IndexedDB
  try {
    const deleteRequest = indexedDB.deleteDatabase(cacheName);
    await new Promise((resolve, reject) => {
      deleteRequest.onsuccess = () => {
        console.log("[WasmCache] Cleared IndexedDB cache");
        resolve(void 0);
      };
      deleteRequest.onerror = () => reject(deleteRequest.error);
    });
  } catch (e) {
    console.warn("[WasmCache] Failed to clear IndexedDB:", e);
  }
}

/**
 * Initialize Wasm module with caching
 * Wraps fetch with cache check, falls back to network
 */
export async function initializeWasmWithCache(
  wasmUrl: string,
  onProgress?: (progress: ProgressUpdate) => void
): Promise<Response> {
  // Try cache first
  const cached = await getCachedWasmModule(wasmUrl);
  if (cached) {
    if (onProgress) {
      onProgress({ file: wasmUrl, stage: "wasm-load", percent: 100 });
    }
    return cached;
  }
  
  // Fetch from network with progress
  const response = await fetch(wasmUrl);
  
  if (!response.ok) {
    throw new Error(`Failed to fetch Wasm module: ${response.status}`);
  }
  
  // Clone for caching (we need to consume the original for instantiateStreaming)
  const responseClone = response.clone();
  
  // Cache in background
  cacheWasmModule(wasmUrl, responseClone).catch((e) => {
    console.warn("[WasmCache] Background caching failed:", e);
  });
  
  return response;
}

export type { CacheConfig };
