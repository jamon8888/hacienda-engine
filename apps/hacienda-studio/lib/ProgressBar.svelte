<script lang="ts">
	import type { ProgressUpdate } from './types';

	interface Props {
		/** Named loosely so both a DOM File and a worker FileInput satisfy it. */
		file: { name: string };
		update: ProgressUpdate;
	}

	let { file, update }: Props = $props();

	const stageLabels = {
		extract: 'Extracting text',
		ner: 'Finding entities',
		pii: 'Scanning for PII',
		link: 'Linking entities',
		complete: 'Complete',
		error: 'Error'
	};
</script>

<div class="progress-card">
	<div class="progress-header">
		<span class="file-name">{file.name}</span>
		<span class="stage-label">{stageLabels[update.stage]}</span>
	</div>
	<div class="progress-bar" role="progressbar" aria-valuenow={update.percent} aria-valuemin="0" aria-valuemax="100">
		<div class="progress-fill" style="width: {update.percent}%"></div>
	</div>
	{#if update.message}
		<p class="progress-message">{update.message}</p>
	{/if}
</div>

<style>
	.progress-card { background: var(--color-surface); border: 1px solid var(--color-border); border-radius: var(--radius); padding: var(--spacing); }
	.progress-header { display: flex; justify-content: space-between; margin-bottom: calc(var(--spacing) / 2); font-size: 0.85rem; }
	.file-name { font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 70%; }
	.stage-label { color: var(--color-muted); }
	.progress-bar { height: 6px; background: var(--color-bg); border-radius: 3px; overflow: hidden; }
	.progress-fill { height: 100%; background: linear-gradient(90deg, var(--color-primary), var(--color-success)); transition: width 0.3s ease; }
	.progress-message { font-size: 0.75rem; color: var(--color-muted); margin-top: calc(var(--spacing) / 2); }
</style>