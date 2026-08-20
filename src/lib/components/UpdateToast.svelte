<script lang="ts">
	import { openUrl } from '@tauri-apps/plugin-opener';
	import { updates } from '$lib/update.svelte';

	// Bottom-right: the sidebar owns the bottom-left corner, and the response
	// pane is the one thing that shouldn't be covered while a request is open.
	async function download() {
		const url = updates.available?.url;
		if (!url) return;
		// Dismiss first — they've seen it, and the browser is about to take focus.
		updates.dismiss();
		await openUrl(url);
	}
</script>

{#if updates.available}
	<div
		class="fixed bottom-4 right-4 z-100 w-72 rounded-md border border-border bg-panel p-3 shadow-xl"
		role="status"
	>
		<div class="flex items-start gap-2">
			<span class="i-lucide-arrow-up-circle mt-0.5 shrink-0 text-3.5 text-accent"></span>
			<div class="min-w-0 flex-1">
				<p class="text-xs font-medium text-text">Fiber {updates.available.version} is available</p>
				<p class="mt-0.5 text-xs text-muted">You're on {updates.available.current}.</p>
			</div>
			<button
				class="i-lucide-x shrink-0 text-3 text-muted hover:text-text"
				aria-label="Dismiss"
				onclick={() => updates.dismiss()}
			></button>
		</div>

		<div class="mt-2.5 flex justify-end gap-1.5">
			<button class="btn-ghost text-xs" onclick={() => updates.dismiss()}>Not now</button>
			<button class="btn-primary text-xs" onclick={download}>Download</button>
		</div>
	</div>
{/if}
