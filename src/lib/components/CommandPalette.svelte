<script lang="ts">
	import { Dialog } from 'bits-ui';
	import { methodColor, type Section } from '$lib/api';
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

	interface Group {
		section: Section;
		items: Selection[];
		/** Flat index of this group's first item, so arrow keys can index into it. */
		offset: number;
	}

	// Group the ranked matches by collection. Order is preserved: a group first
	// appears where its best-scoring match does, and its items keep that ranking.
	const groups = $derived.by<Group[]>(() => {
		const byId = new Map<string, Group>();
		const order: Group[] = [];
		for (const match of matches) {
			let group = byId.get(match.section.id);
			if (!group) {
				group = { section: match.section, items: [], offset: 0 };
				byId.set(match.section.id, group);
				order.push(group);
			}
			group.items.push(match);
		}
		let offset = 0;
		for (const group of order) {
			group.offset = offset;
			offset += group.items.length;
		}
		return order;
	});

	/** Every match in render order, so arrow keys move through them as one list. */
	const flat = $derived(groups.flatMap((group) => group.items));

	// Keep the highlight in range as the result set shrinks under typing.
	$effect(() => {
		if (active >= flat.length) active = Math.max(0, flat.length - 1);
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
			active = Math.min(active + 1, flat.length - 1);
		} else if (event.key === 'ArrowUp') {
			event.preventDefault();
			active = Math.max(active - 1, 0);
		} else if (event.key === 'Enter') {
			event.preventDefault();
			choose(flat[active]);
		}
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Portal>
		<Dialog.Overlay class="dialog-scrim" />
		<Dialog.Content
			class="fixed left-1/2 top-[18%] z-60 w-[min(620px,90vw)] -translate-x-1/2 rounded-lg border border-border bg-panel shadow-2xl overflow-hidden"
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
				{#each groups as group (group.section.id)}
					<div
						class="sticky top-0 z-10 bg-panel border-b border-border/50 px-4 py-1 text-2.5 font-medium text-muted"
					>
						{group.section.name}
					</div>
					{#each group.items as match, index (match.section.id + ' ' + match.request.id)}
						{@const flatIndex = group.offset + index}
						<button
							class="w-full flex items-center gap-3 px-4 py-2 text-left transition-colors
								{flatIndex === active ? 'bg-raised' : 'hover:bg-raised/50'}"
							onclick={() => choose(match)}
							onmouseenter={() => (active = flatIndex)}
						>
							<span class="font-mono text-2.5 font-bold w-10 shrink-0 {methodColor(match.request.method)}">
								{match.request.method}
							</span>
							<span class="truncate text-xs">{match.request.name}</span>
							<span class="truncate font-mono text-2.5 text-muted">{match.request.path}</span>
						</button>
					{/each}
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
