<script lang="ts">
	import type { OnboardingState } from './types';

	interface Props {
		assets: OnboardingState['assets'];
		onComplete: () => void;
	}

	let { assets, onComplete }: Props = $props();

	function overallPercent(assets: OnboardingState['assets']): number {
		const values = Object.values(assets);
		return Math.round((values.filter(v => v).length / values.length) * 100);
	}

	function iconFor(key: string): string {
		return { xbergWasm: '⚙️', nerModel: '🧠', tessdata: '👁️' }[key] || '📦';
	}

	function labelFor(key: string): string {
		return { xbergWasm: 'xberg WASM Engine', nerModel: 'GLiNER2-Guardrails-PII', tessdata: 'Tesseract OCR Data' }[key] || key;
	}
</script>

<div class="onboarding-overlay" role="dialog" aria-modal="true" aria-labelledby="onboarding-title">
	<div class="onboarding-card">
		<header>
			<h1 id="onboarding-title">
				<span class="icon" aria-hidden="true">🔒</span>
				Hacienda Studio — 100% Local AI in Your Browser
			</h1>
			<p class="subtitle">Your files <strong>NEVER leave this tab</strong>. All processing runs locally via WebAssembly.</p>
		</header>

		<div class="progress-section">
			<div class="overall-progress">
				<div class="progress-bar" role="progressbar" aria-valuenow={overallPercent(assets)} aria-valuemin="0" aria-valuemax="100">
					<div class="progress-fill" style="width: {overallPercent(assets)}%"></div>
				</div>
				<span class="progress-text">{overallPercent(assets)}% — Preparing local models...</span>
			</div>

			<ul class="asset-list" role="list">
				{#each Object.entries(assets) as [key, ready]}
					<li class={ready ? 'ready' : 'loading'}>
						<span class="asset-icon" aria-hidden="true">{iconFor(key)}</span>
						<span class="asset-name">{labelFor(key)}</span>
						<span class="asset-status">{ready ? '✓ Cached' : '↓ Downloading...'}</span>
					</li>
				{/each}
			</ul>
		</div>

		<footer>
			<button class="btn-secondary" disabled={!(assets.xbergWasm && assets.nerModel && assets.tessdata)} onclick={onComplete}>
				Continue
			</button>
		</footer>
	</div>
</div>

<style>
	.onboarding-overlay {
		position: fixed;
		inset: 0;
		z-index: 1000;
		background: rgba(13, 17, 23, 0.95);
		display: flex;
		align-items: center;
		justify-content: center;
		padding: var(--spacing);
	}

	.onboarding-card {
		background: var(--color-surface);
		border: 1px solid var(--color-border);
		border-radius: 12px;
		padding: calc(var(--spacing) * 3);
		max-width: 520px;
		width: 100%;
		box-shadow: 0 8px 32px rgba(0,0,0,0.4);
	}

	header {
		text-align: center;
		margin-bottom: calc(var(--spacing) * 2);
	}

	h1 {
		font-size: 1.5rem;
		font-weight: 600;
		margin-bottom: var(--spacing);
	}

	.icon {
		margin-right: var(--spacing);
	}

	.subtitle {
		color: var(--color-muted);
		font-size: 0.95rem;
		line-height: 1.5;
	}

	.progress-bar {
		height: 8px;
		background: var(--color-border);
		border-radius: 4px;
		overflow: hidden;
		margin-bottom: var(--spacing);
	}

	.progress-fill {
		height: 100%;
		background: linear-gradient(90deg, var(--color-primary), var(--color-success));
		transition: width 0.3s ease;
	}

	.progress-text {
		font-size: 0.85rem;
		color: var(--color-muted);
	}

	.asset-list {
		list-style: none;
		display: flex;
		flex-direction: column;
		gap: var(--spacing);
		margin-bottom: calc(var(--spacing) * 2);
	}

	.asset-list li {
		display: flex;
		align-items: center;
		gap: var(--spacing);
		padding: var(--spacing);
		background: var(--color-bg);
		border-radius: var(--radius);
	}

	.asset-list li.ready {
		border-left: 3px solid var(--color-success);
	}

	.asset-list li.loading {
		border-left: 3px solid var(--color-warning);
	}

	.asset-icon {
		font-size: 1.2rem;
	}

	.asset-name {
		flex: 1;
		font-weight: 500;
	}

	.asset-status {
		font-size: 0.85rem;
		color: var(--color-muted);
	}

	/* `ready` sits on the <li>, not on the status span. */
	li.ready .asset-status {
		color: var(--color-success);
	}

	footer {
		text-align: center;
	}

	.btn-secondary {
		padding: var(--spacing) calc(var(--spacing) * 3);
		background: var(--color-border);
		color: var(--color-text);
		border-radius: var(--radius);
		font-weight: 500;
		transition: background 0.2s;
	}

	.btn-secondary:hover:not(:disabled) {
		background: var(--color-primary);
	}

	.btn-secondary:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
</style>