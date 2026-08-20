<script lang="ts">
	import DotLoader from '$lib/components/DotLoader.svelte';
	import { updates } from '$lib/update.svelte';

	// Bottom-right: the sidebar owns the bottom-left corner, and the response
	// pane is the one thing that shouldn't be covered while a request is open.
	const percent = $derived(
		updates.progress === null ? null : Math.round(updates.progress * 100)
	);
</script>

{#if updates.stage !== 'idle'}
	<div
		class="fixed bottom-4 right-4 z-100 w-72 rounded-md border border-border bg-panel p-3 shadow-xl"
		role="status"
	>
		{#if updates.stage === 'available'}
			<div class="flex items-start gap-2">
				<span class="i-lucide-arrow-up-circle mt-0.5 shrink-0 text-3.5 text-accent"></span>
				<div class="min-w-0 flex-1">
					<p class="text-xs font-medium text-text">Fiber {updates.version} is available</p>
					<p class="mt-0.5 text-xs text-muted">Installing restarts the app.</p>
				</div>
				<button
					class="i-lucide-x shrink-0 text-3 text-muted hover:text-text"
					aria-label="Dismiss"
					onclick={() => updates.dismiss()}
				></button>
			</div>
			<div class="mt-2.5 flex justify-end gap-1.5">
				<button class="btn-ghost text-xs" onclick={() => updates.dismiss()}>Not now</button>
				<button class="btn-primary text-xs" onclick={() => updates.install()}>Update</button>
			</div>
		{:else if updates.stage === 'downloading'}
			<div class="flex items-center gap-2">
				<DotLoader size={16} class="text-accent" />
				<p class="flex-1 text-xs text-text">Downloading {updates.version}…</p>
				{#if percent !== null}
					<span class="font-mono text-2.5 text-muted tabular-nums">{percent}%</span>
				{/if}
			</div>
			<!-- Indeterminate when the server sent no content-length: a bar that
			     sits at zero reads as a hang. -->
			<div class="mt-2 h-1 overflow-hidden rounded-full bg-raised">
				{#if percent === null}
					<div class="h-full w-1/3 animate-pulse rounded-full bg-accent"></div>
				{:else}
					<div
						class="h-full rounded-full bg-accent transition-[width] duration-150"
						style="width:{percent}%"
					></div>
				{/if}
			</div>
		{:else if updates.stage === 'installing'}
			<div class="flex items-center gap-2">
				<DotLoader size={16} class="text-accent" />
				<p class="text-xs text-text">Installing {updates.version} — restarting…</p>
			</div>
		{:else if updates.stage === 'failed'}
			<div class="flex items-start gap-2">
				<span class="i-lucide-circle-alert mt-0.5 shrink-0 text-3.5 text-bad"></span>
				<div class="min-w-0 flex-1">
					<p class="text-xs font-medium text-text">The update didn't install</p>
					<p class="mt-0.5 break-words text-2.5 text-muted">{updates.error}</p>
				</div>
			</div>
			<div class="mt-2.5 flex justify-end">
				<button class="btn-ghost text-xs" onclick={() => updates.reset()}>Close</button>
			</div>
		{/if}
	</div>
{/if}
