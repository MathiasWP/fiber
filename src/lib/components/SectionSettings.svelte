<script lang="ts">
	import { Dialog } from 'bits-ui';
	import type { Section } from '$lib/api';
	import { collections } from '$lib/collections.svelte';

	interface Props {
		/** The section being edited, or null when closed. */
		section: Section | null;
		onClose: () => void;
	}

	let { section, onClose }: Props = $props();

	const open = $derived(section !== null);

	function close() {
		if (section) collections.flush(section);
		onClose();
	}
</script>

<Dialog.Root
	{open}
	onOpenChange={(next) => {
		if (!next) close();
	}}
>
	<Dialog.Portal>
		<Dialog.Overlay class="fixed inset-0 bg-black/50" />
		<Dialog.Content
			class="fixed left-1/2 top-1/2 w-[min(480px,90vw)] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-panel p-4 shadow-2xl"
		>
			<Dialog.Title class="text-sm font-semibold">Section settings</Dialog.Title>
			<Dialog.Description class="mt-1 text-xs text-muted">
				Requests in this section only need a path — the base URL is prepended to each one.
			</Dialog.Description>

			{#if section}
				<div class="mt-4 flex flex-col gap-3">
					<label class="flex flex-col gap-1">
						<span class="text-xs text-muted">Name</span>
						<input bind:value={section.name} class="input-base text-xs" />
					</label>

					<label class="flex flex-col gap-1">
						<span class="text-xs text-muted">Base URL</span>
						<input
							bind:value={section.baseUrl}
							spellcheck="false"
							placeholder="https://api.example.com"
							class="input-base text-xs font-mono selectable"
						/>
					</label>

					<p class="text-2.5 text-muted leading-relaxed">
						Auth and dynamic endpoint loaders will live here too.
					</p>
				</div>

				<div class="mt-5 flex justify-end gap-2">
					<button class="btn-primary text-xs" onclick={close}>Done</button>
				</div>
			{/if}
		</Dialog.Content>
	</Dialog.Portal>
</Dialog.Root>
