<script lang="ts">
	import type { AppConfig } from './types';

	interface Props {
		config: AppConfig;
	}

	let { config }: Props = $props();

	const allCategories = [
		{ key: 'person', label: 'Person', group: 'Person' },
		{ key: 'full_name', label: 'Full Name', group: 'Person' },
		{ key: 'first_name', label: 'First Name', group: 'Person' },
		{ key: 'last_name', label: 'Last Name', group: 'Person' },
		{ key: 'organization', label: 'Organization', group: 'Organization' },
		{ key: 'company', label: 'Company', group: 'Organization' },
		{ key: 'location', label: 'Location', group: 'Location' },
		{ key: 'city', label: 'City', group: 'Location' },
		{ key: 'state_or_region', label: 'State/Region', group: 'Location' },
		{ key: 'country', label: 'Country', group: 'Location' },
		{ key: 'email', label: 'Email', group: 'Contact' },
		{ key: 'phone_number', label: 'Phone', group: 'Contact' },
		{ key: 'address', label: 'Address', group: 'Contact' },
		{ key: 'date', label: 'Date', group: 'Temporal' },
		{ key: 'money', label: 'Money', group: 'Financial' }
	];

	const grouped = Object.groupBy(allCategories, (c) => c.group);

	let showConfig = $state(false);

	function toggleCategory(key: string) {
		const idx = config.nerCategories.indexOf(key);
		if (idx >= 0) config.nerCategories.splice(idx, 1);
		else config.nerCategories.push(key);
		config = config; // trigger reactivity
	}
</script>

<div class="config-panel" role="dialog" aria-label="Configuration">
	<header>
		<h2>⚙ Configuration</h2>
		<p class="local-badge">🔒 All processing runs locally in your browser</p>
	</header>

	<section>
		<h3>NER Categories</h3>
		<div class="category-grid">
			{#each Object.entries(grouped) as [group, categories]}
				<fieldset>
					<legend>{group}</legend>
					{#each categories as cat}
						<label>
							<input type="checkbox" checked={config.nerCategories.includes(cat.key)} onchange={() => toggleCategory(cat.key)} />
							<span>{cat.label}</span>
						</label>
					{/each}
				</fieldset>
			{/each}
		</div>
	</section>

	<section>
		<h3>Output</h3>
		<div class="field">
			<label for="output-format">Format</label>
			<select id="output-format" bind:value={config.outputFormat}>
				<option value="markdown">Markdown</option>
				<option value="plain">Plain Text</option>
				<option value="json">JSON</option>
			</select>
		</div>
		<div class="field">
			<label for="chunk-size">Chunk Size (tokens)</label>
			<input id="chunk-size" type="number" bind:value={config.chunkSize} min="100" max="8000" step="100" />
		</div>
	</section>

	<footer>
		<button class="btn-secondary" onclick={() => showConfig = false}>Done</button>
	</footer>
</div>

<style>
	.config-panel { position: fixed; top: 0; right: 0; bottom: 0; width: 360px; max-width: 100vw; background: var(--color-surface); border-left: 1px solid var(--color-border); z-index: 100; padding: calc(var(--spacing) * 2); overflow-y: auto; box-shadow: -8px 0 32px rgba(0,0,0,0.3); animation: slideIn 0.2s ease; }
	@keyframes slideIn { from { transform: translateX(100%); } to { transform: translateX(0); } }
	header { margin-bottom: calc(var(--spacing) * 2); padding-bottom: var(--spacing); border-bottom: 1px solid var(--color-border); }
	h2 { font-size: 1.1rem; margin-bottom: calc(var(--spacing) / 2); }
	.local-badge { font-size: 0.8rem; color: var(--color-success); font-weight: 500; }
	section { margin-bottom: calc(var(--spacing) * 2); }
	h3 { font-size: 0.9rem; text-transform: uppercase; letter-spacing: 0.05em; color: var(--color-muted); margin-bottom: var(--spacing); }
	.category-grid { display: flex; flex-direction: column; gap: var(--spacing); }
	fieldset { border: 1px solid var(--color-border); border-radius: var(--radius); padding: var(--spacing); }
	legend { font-size: 0.75rem; text-transform: uppercase; color: var(--color-muted); padding: 0 calc(var(--spacing) / 2); }
	label { display: flex; align-items: center; gap: var(--spacing); cursor: pointer; font-size: 0.85rem; }
	input[type="checkbox"] { width: 16px; height: 16px; accent-color: var(--color-primary); }
	.field { display: flex; flex-direction: column; gap: calc(var(--spacing) / 2); }
	.field label { font-size: 0.8rem; color: var(--color-muted); }
	select, input[type="number"] { padding: var(--spacing) calc(var(--spacing) * 1.5); background: var(--color-bg); border: 1px solid var(--color-border); border-radius: var(--radius); color: var(--color-text); font-family: inherit; }
	footer { margin-top: calc(var(--spacing) * 3); padding-top: var(--spacing); border-top: 1px solid var(--color-border); text-align: right; }
</style>