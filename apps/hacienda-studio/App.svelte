<script lang="ts">
	import { onMount } from 'svelte';
	import Onboarding from './lib/Onboarding.svelte';
	import ProgressBar from './lib/ProgressBar.svelte';
	import ConfigPanel from './lib/ConfigPanel.svelte';
	import { loadNerModel, isModelCached, preloadXbergWasm, validateFile } from './lib/asset-loader';
	import { DEFAULT_CONFIG } from './lib/types';
	import type { AppConfig, FileInput, ProcessedFile, ProgressUpdate } from './lib/types';

	let onboardingComplete = $state(false);
	let assets = $state({ xbergWasm: false, nerModel: false, tessdata: false });
	let files = $state<File[]>([]);
	let progress = $state<Map<string, ProgressUpdate>>(new Map());
	let results = $state<ProcessedFile[]>([]);
	let config = $state<AppConfig>({ ...DEFAULT_CONFIG });
	let showConfig = $state(false);
	let error = $state<string | null>(null);
	// The drop zone renders before the worker finishes its handshake, and the
	// handshake is slow — it compiles a 48 MB WASM module. Dropping a file into
	// that window used to throw on a null worker and silently do nothing.
	let workerReady = $state(false);
	let worker: Worker | null = null;

	onMount(async () => {
		// Check if onboarding was completed before
		const visited = localStorage.getItem('xberg-studio-visited');
		if (visited) {
			onboardingComplete = true;
			assets = { xbergWasm: true, nerModel: true, tessdata: true };
		} else {
			await preloadAssets();
		}

		// Initialize worker
		worker = new Worker(new URL('./worker/pipeline.ts', import.meta.url), { type: 'module' });
		await new Promise<void>(resolve => {
			worker!.onmessage = (e) => {
				if (e.data.type === 'ready') resolve();
			};
			worker!.postMessage({ type: 'init' });
		});
		worker.onmessage = handleWorkerMessage;
		workerReady = true;
	});

	async function preloadAssets() {
		try {
			console.log('[App] preloadAssets started');
			assets.xbergWasm = true;
			console.log('[App] assets.xbergWasm = true');
			await preloadXbergWasm();
			console.log('[App] preloadXbergWasm done');

			if (await isModelCached()) {
				assets.nerModel = true;
			} else {
				try {
					await loadNerModel();
					assets.nerModel = true;
				} catch (e) {
					console.warn('[App] NER model download failed, using fallback:', e);
					assets.nerModel = true;
				}
			}

			assets.tessdata = true;
			localStorage.setItem('xberg-studio-visited', 'true');
		} catch (e) {
			console.error('[App] preloadAssets error:', e);
			error = 'Failed to load models. Some features may be limited.';
			assets.xbergWasm = true;
			assets.nerModel = true;
			assets.tessdata = true;
			localStorage.setItem('xberg-studio-visited', 'true');
		}
	}

	function handleWorkerMessage(event: MessageEvent) {
		const { type, ...data } = event.data;
		switch (type) {
			case 'progress':
				progress = new Map(progress).set(data.file, data);
				break;
			case 'file-complete':
				results = [...results, data];
				progress = new Map(progress).set(data.name, { ...data, stage: 'complete', percent: 100 });
				break;
			case 'batch-complete':
				downloadZip(data.zip);
				// Clear progress after a short delay to show completion
				setTimeout(() => {
					progress.clear();
					files = [];
				}, 1000);
				break;
			case 'error':
				error = `${data.file}: ${data.message}`;
				break;
		}
	}

	async function handleFiles(fileList: FileList | FileList | File[]): Promise<void> {
		console.log('[App] handleFiles called with', fileList.length, 'files');
		const fileArray = Array.from(fileList);
		const validFiles: File[] = [];

		for (const file of fileArray) {
			const validation = validateFile(file);
			console.log('[App] validateFile:', file.name, file.type, validation);
			if (!validation.valid) {
				error = validation.error || 'Invalid file';
				continue;
			}
			validFiles.push(file);
		}

		if (validFiles.length === 0) return;

		files = [...files, ...validFiles];
		error = null;

		// Send to worker
		const fileInputs = await Promise.all(validFiles.map(async f => ({
			name: f.name,
			bytes: await f.arrayBuffer(),
			type: f.type || 'application/octet-stream'
		})));

		console.log('[App] posting to worker:', fileInputs.length, 'files');
		worker!.postMessage({ type: 'process', files: fileInputs, config: JSON.parse(JSON.stringify(config)) });
	}

	function onDrop(event: DragEvent): void {
		event.preventDefault();
		if (!workerReady) return;
		if (event.dataTransfer?.files) handleFiles(event.dataTransfer.files);
	}

	function onDragOver(event: DragEvent): void {
		event.preventDefault();
		event.dataTransfer!.dropEffect = 'copy';
	}

	function downloadZip(blob: Blob): void {
		const url = URL.createObjectURL(blob);
		const a = document.createElement('a');
		a.href = url;
		a.download = `xberg-output-${Date.now()}.zip`;
		a.click();
		URL.revokeObjectURL(url);
	}

	function clearError(): void {
		error = null;
	}
</script>

<div class="app">
	{#if !onboardingComplete}
		<Onboarding {assets} onComplete={() => { onboardingComplete = true; localStorage.setItem('xberg-studio-visited', 'true'); }} />
	{:else}
		<header class="header">
			<h1>Hacienda Studio</h1>
			<button class="config-toggle" onclick={() => showConfig = !showConfig} aria-expanded={showConfig}>
				⚙ Config
			</button>
		</header>

		{#if error}
			<div class="error-banner" role="alert">
				<span>❌ {error}</span>
				<button onclick={clearError} aria-label="Dismiss">✕</button>
			</div>
		{/if}

		<main class="main">
			<section class="drop-zone" role="group" aria-label="Upload documents" ondrop={onDrop} ondragover={onDragOver}>
				<input type="file" id="file-input" multiple disabled={!workerReady} accept=".pdf,.docx,.xlsx,.pptx,.odt,.ods,.odp,.eml,.msg,.pst,.png,.jpg,.jpeg,.gif,.webp,.tiff,.bmp,.svg,.srt,.vtt,.txt,.md,.json,.csv,.xml,.html" class="file-input" onchange={(e) => { const files = (e.target as HTMLInputElement).files; if (files) handleFiles(files); }} aria-label="Choose files" />
				<label for="file-input" class="drop-label">
					<span class="drop-icon" aria-hidden="true">📄</span>
					<p>{workerReady ? 'Drop files here or click to browse' : 'Starting the local engine…'}</p>
					<p class="drop-hint">PDF, Office, Email, Images, Subtitles, Code — up to 50MB each</p>
				</label>
			</section>

			{#if files.length > 0}
				<section class="progress-section" aria-live="polite">
					{#each files as file}
						{#if progress.has(file.name)}
							<ProgressBar file={file} update={progress.get(file.name)!} />
						{/if}
					{/each}
				</section>
			{/if}

			{#if results.length > 0}
				<footer class="footer">
					<button class="btn-primary" disabled>
						Processing...
					</button>
				</footer>
			{/if}
		</main>

		{#if showConfig}
			<ConfigPanel {config} />
		{/if}
	{/if}
</div>

<style>
	.app { min-height: 100vh; display: flex; flex-direction: column; }
	.header { display: flex; justify-content: space-between; align-items: center; padding: var(--spacing) calc(var(--spacing) * 2); border-bottom: 1px solid var(--color-border); background: var(--color-surface); }
	h1 { font-size: 1.25rem; font-weight: 600; }
	.config-toggle { padding: var(--spacing) calc(var(--spacing) * 1.5); background: var(--color-border); border-radius: var(--radius); font-size: 0.85rem; color: var(--color-text); transition: background 0.2s; }
	.config-toggle:hover { background: var(--color-primary); }
	.error-banner { display: flex; justify-content: space-between; align-items: center; padding: var(--spacing) calc(var(--spacing) * 2); background: rgba(248, 81, 73, 0.15); border-bottom: 1px solid var(--color-error); color: var(--color-error); }
	.error-banner button { color: inherit; font-size: 1.2rem; line-height: 1; }
	.main { flex: 1; padding: calc(var(--spacing) * 3) calc(var(--spacing) * 2); max-width: 800px; margin: 0 auto; width: 100%; }
	.drop-zone { border: 2px dashed var(--color-border); border-radius: 12px; padding: calc(var(--spacing) * 4) var(--spacing); text-align: center; transition: border-color 0.2s, background 0.2s; background: var(--color-surface); cursor: pointer; }
	.drop-zone:hover { border-color: var(--color-primary); background: rgba(88, 166, 255, 0.05); }
	.file-input { position: absolute; width: 0.1px; height: 0.1px; opacity: 0; overflow: hidden; z-index: -1; }
	.drop-label { display: block; cursor: pointer; }
	.drop-icon { font-size: 3rem; margin-bottom: var(--spacing); display: block; }
	.drop-label p { color: var(--color-muted); margin-bottom: calc(var(--spacing) / 2); }
	.drop-hint { font-size: 0.85rem !important; }
	.progress-section { margin-top: calc(var(--spacing) * 2); display: flex; flex-direction: column; gap: var(--spacing); }
	.footer { padding-top: calc(var(--spacing) * 2); border-top: 1px solid var(--color-border); display: flex; justify-content: flex-end; }
	.btn-primary { padding: var(--spacing) calc(var(--spacing) * 3); background: var(--color-primary); color: #fff; border-radius: var(--radius); font-weight: 600; }
	.btn-primary:disabled { opacity: 0.6; cursor: not-allowed; }
</style>