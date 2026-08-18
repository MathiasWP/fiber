<script lang="ts">
	import { Dialog } from 'bits-ui';
	import { methodColor } from '$lib/api';
	import { allRequests, collections, fuzzyScore, type Selection } from '$lib/collections.svelte';

	interface Props {
		open?: boolean;
		onSelect: (selection: Selection) => void;
	}

	let { open = $bindable(false), onSelect }: Props = $props();

	let query = $state('');
	let active = $state(0);

	const matches = $derived.by<Selection[]>(() => {
		const entries = allRequests(collections.sections);
		const needle = query.trim();
		if (!needle) return entries.slice(0, 50);

		return entries
			.map((entry) => ({
				entry,
				score: fuzzyScore(
					`${entry.section.name} ${entry.request.name} ${entry.request.method} ${entry.request.path}`,
					needle
				)
			}))
			.filter((match) => match.score !== null)
			.sort((a, b) => a.score! - b.score!)
			.slice(0, 50)
			.map((match) => match.entry);
	});

	// Keep the highlight in range as the result set shrinks under typing.
	$effect(() => {
		if (active >= matches.length) active = Math.max(0, matches.length - 1);
	});

	$effect(() => {
		if (open) {
			query = '';
			active = 0;
		}
	});

	function choose(selection: Selection | undefined) {
		if (!selection) return;
		onSelect(selection);
		open = false;
	}

	function onKeydown(event: KeyboardEvent) {
		if (event.key === 'ArrowDown') {
			event.preventDefault();
			active = Math.min(active + 1, matches.length - 1);
		} else if (event.key === 'ArrowUp') {
			event.preventDefault();
			active = Math.max(active - 1, 0);
		} else if (event.key === 'Enter') {
			event.preventDefault();
			choose(matches[active]);
		}
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Portal>
		<Dialog.Overlay class="fixed inset-0 bg-black/50" />
		<Dialog.Content
			class="fixed left-1/2 top-[18%] w-[min(620px,90vw)] -translate-x-1/2 rounded-lg border border-border bg-panel shadow-2xl overflow-hidden"
		>
			<Dialog.Title class="sr-only">Search endpoints</Dialog.Title>
			<Dialog.Description class="sr-only">
				Find a saved request across every section.
			</Dialog.Description>

			<!-- svelte-ignore a11y_autofocus -->
			<input
				bind:value={query}
				autofocus
				spellcheck="false"
				placeholder="Search endpoints…"
				class="w-full bg-transparent border-0 border-b border-border px-4 py-3 text-text outline-none"
				onkeydown={onKeydown}
			/>

			<div class="max-h-[50vh] overflow-y-auto">
				{#each matches as match, index (match.request.id)}
					<button
						class="w-full flex items-center gap-3 px-4 py-2 text-left transition-colors
							{index === active ? 'bg-raised' : 'hover:bg-raised/50'}"
						onclick={() => choose(match)}
						onmouseenter={() => (active = index)}
					>
						<span class="font-mono text-2.5 font-bold w-10 shrink-0 {methodColor(match.request.method)}">
							{match.request.method}
						</span>
						<span class="truncate text-xs">{match.request.name}</span>
						<span class="truncate font-mono text-2.5 text-muted">{match.request.path}</span>
						<span class="ml-auto text-2.5 text-muted shrink-0">{match.section.name}</span>
					</button>
				{:else}
					<p class="px-4 py-3 text-xs text-muted">
						{collections.sections.length
							? 'Nothing matches.'
							: 'No saved requests yet — create a section first.'}
					</p>
				{/each}
			</div>
		</Dialog.Content>
	</Dialog.Portal>
</Dialog.Root>
