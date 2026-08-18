<script lang="ts">
	import { ContextMenu, Dialog } from 'bits-ui';
	import { methodColor, statusColor, type SavedRequest, type Section } from '$lib/api';
	import { collections, fuzzyScore } from '$lib/collections.svelte';
	import { history, type HistoryEntry } from '$lib/history.svelte';
	import { theme } from '$lib/theme.svelte';

	interface Props {
		onOpenSettings: (section: Section) => void;
		onPickHistory: (entry: HistoryEntry) => void;
	}

	let { onOpenSettings, onPickHistory }: Props = $props();

	let tab = $state<'collections' | 'history'>('collections');
	let query = $state('');
	let creating = $state(false);
	let newName = $state('');
	let newBaseUrl = $state('');
	let renamingId = $state<string | null>(null);
	let pendingDelete = $state<Section | null>(null);

	interface VisibleSection {
		section: Section;
		requests: SavedRequest[];
	}

	const visible = $derived.by<VisibleSection[]>(() => {
		const needle = query.trim();
		if (!needle) {
			return collections.sections.map((section) => ({ section, requests: section.requests }));
		}

		return collections.sections
			.map((section) => ({
				section,
				requests: section.requests
					.map((request) => ({
						request,
						score: fuzzyScore(`${request.name} ${request.method} ${request.path}`, needle)
					}))
					.filter((match) => match.score !== null)
					.sort((a, b) => a.score! - b.score!)
					.map((match) => match.request)
			}))
			.filter((entry) => entry.requests.length > 0);
	});

	// A search hides the collapsed state — matches are no use if you can't see them.
	const searching = $derived(query.trim().length > 0);

	const themeIcon = $derived(
		theme.mode === 'system'
			? 'i-lucide-monitor'
			: theme.mode === 'dark'
				? 'i-lucide-moon'
				: 'i-lucide-sun'
	);

	function toggle(section: Section) {
		section.collapsed = !section.collapsed;
		collections.touch(section);
	}

	async function addSection() {
		if (!newName.trim() && !newBaseUrl.trim()) {
			creating = false;
			return;
		}
		const section = await collections.createSection(newName, newBaseUrl);
		newName = '';
		newBaseUrl = '';
		creating = false;
		await collections.createRequest(section, 'New request');
	}

	function commitRename(section: Section) {
		renamingId = null;
		if (!section.name.trim()) section.name = 'Untitled';
		collections.flush(section);
	}

	function commitRequestRename(section: Section, request: SavedRequest) {
		renamingId = null;
		if (!request.name.trim()) request.name = 'Untitled';
		collections.flush(section);
	}

	function copyUrl(section: Section, request: SavedRequest) {
		const base = section.baseUrl.trim().replace(/\/+$/, '');
		const path = request.path.trim();
		const absolute = /^https?:\/\//.test(path);
		navigator.clipboard.writeText(absolute ? path : `${base}/${path.replace(/^\/+/, '')}`);
	}

	function clockTime(at: number) {
		return new Date(at).toLocaleTimeString(undefined, {
			hour: '2-digit',
			minute: '2-digit',
			second: '2-digit'
		});
	}
</script>

<!-- Deleting a section throws away a file, so it asks first. Requests don't. -->
<Dialog.Root
	open={pendingDelete !== null}
	onOpenChange={(next) => {
		if (!next) pendingDelete = null;
	}}
>
	<Dialog.Portal>
		<Dialog.Overlay class="fixed inset-0 bg-black/50" />
		<Dialog.Content
			class="fixed left-1/2 top-1/2 w-[min(420px,90vw)] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-panel p-4 shadow-2xl"
		>
			<Dialog.Title class="text-sm font-semibold">Delete section</Dialog.Title>
			<Dialog.Description class="mt-2 text-xs text-muted leading-relaxed">
				“{pendingDelete?.name}” and its {pendingDelete?.requests.length} request{pendingDelete
					?.requests.length === 1
					? ''
					: 's'} will be removed from disk. This can't be undone.
			</Dialog.Description>
			<div class="mt-5 flex justify-end gap-2">
				<button class="btn-ghost text-xs" onclick={() => (pendingDelete = null)}>Cancel</button>
				<button
					class="btn-base bg-bad text-white hover:bg-bad/85 text-xs"
					onclick={() => {
						if (pendingDelete) collections.removeSection(pendingDelete);
						pendingDelete = null;
					}}
				>
					Delete
				</button>
			</div>
		</Dialog.Content>
	</Dialog.Portal>
</Dialog.Root>

<aside class="flex flex-col border-r border-border bg-panel min-h-0 h-full">
	<header class="flex items-center gap-1 px-2 h-11 border-b border-border shrink-0">
		<button
			class="px-2 py-1 rounded text-xs transition-colors {tab === 'collections'
				? 'bg-raised text-text'
				: 'text-muted hover:text-text'}"
			onclick={() => (tab = 'collections')}
		>
			Collections
		</button>
		<button
			class="px-2 py-1 rounded text-xs transition-colors {tab === 'history'
				? 'bg-raised text-text'
				: 'text-muted hover:text-text'}"
			onclick={() => (tab = 'history')}
		>
			History
		</button>
		<button
			class="ml-auto p-1 rounded text-muted hover:(bg-raised text-text) transition-colors"
			title={tab === 'history' ? 'Clear history' : 'New section'}
			onclick={() => (tab === 'history' ? history.clear() : (creating = true))}
		>
			<span class={tab === 'history' ? 'i-lucide-trash-2' : 'i-lucide-folder-plus'}></span>
		</button>
	</header>

	{#if tab === 'collections'}
		<div class="px-2 py-2 shrink-0">
			<div class="relative">
				<span
					class="i-lucide-search absolute left-2 top-1/2 -translate-y-1/2 text-muted text-3"
				></span>
				<input
					bind:value={query}
					spellcheck="false"
					placeholder="Search endpoints…"
					class="input-base w-full text-xs pl-7"
				/>
			</div>
		</div>

		{#if creating}
			<div class="px-2 pb-2 flex flex-col gap-1 shrink-0">
				<!-- svelte-ignore a11y_autofocus -->
				<input
					bind:value={newName}
					autofocus
					placeholder="Section name"
					class="input-base text-xs"
					onkeydown={(e) => e.key === 'Enter' && addSection()}
				/>
				<input
					bind:value={newBaseUrl}
					spellcheck="false"
					placeholder="https://api.example.com"
					class="input-base text-xs font-mono"
					onkeydown={(e) => e.key === 'Enter' && addSection()}
				/>
				<div class="flex gap-1">
					<button class="btn-primary text-xs flex-1 justify-center" onclick={addSection}>
						Create
					</button>
					<button class="btn-ghost text-xs" onclick={() => (creating = false)}>Cancel</button>
				</div>
			</div>
		{/if}

		<div class="flex-1 overflow-y-auto min-h-0">
			{#each visible as { section, requests } (section.id)}
				{@const open = searching || !section.collapsed}
				<div class="border-b border-border/50">
					<ContextMenu.Root>
						<ContextMenu.Trigger
							class="flex items-center gap-1 px-2 py-1.5 w-full text-left hover:bg-raised/60 transition-colors cursor-default"
							onclick={() => toggle(section)}
							ondblclick={() => (renamingId = section.id)}
						>
							<span
								class="i-lucide-chevron-right text-3 text-muted transition-transform shrink-0 {open
									? 'rotate-90'
									: ''}"
							></span>

							{#if renamingId === section.id}
								<!-- svelte-ignore a11y_autofocus -->
								<input
									bind:value={section.name}
									autofocus
									class="input-base text-xs py-0.5 flex-1 min-w-0"
									onclick={(e) => e.stopPropagation()}
									onblur={() => commitRename(section)}
									onkeydown={(e) => e.key === 'Enter' && commitRename(section)}
								/>
							{:else}
								<span class="truncate text-xs font-medium flex-1">{section.name}</span>
							{/if}

							<!-- Static: nothing appears or disappears on hover, so nothing shifts. -->
							<span class="text-2.5 text-muted shrink-0 tabular-nums">
								{section.requests.length}
							</span>
						</ContextMenu.Trigger>

						<ContextMenu.Portal>
							<ContextMenu.Content class="menu-content">
								<ContextMenu.Item
									class="menu-item"
									onSelect={() => collections.createRequest(section)}
								>
									<span class="i-lucide-plus text-3"></span>
									New request
								</ContextMenu.Item>
								<ContextMenu.Item class="menu-item" onSelect={() => (renamingId = section.id)}>
									<span class="i-lucide-pencil text-3"></span>
									Rename
								</ContextMenu.Item>
								<ContextMenu.Item class="menu-item" onSelect={() => onOpenSettings(section)}>
									<span class="i-lucide-settings text-3"></span>
									Section settings…
								</ContextMenu.Item>
								<ContextMenu.Separator class="menu-separator" />
								<ContextMenu.Item
									class="menu-item-bad"
									onSelect={() => (pendingDelete = section)}
								>
									<span class="i-lucide-trash-2 text-3"></span>
									Delete section
								</ContextMenu.Item>
							</ContextMenu.Content>
						</ContextMenu.Portal>
					</ContextMenu.Root>

					{#if open}
						{#each requests as request (request.id)}
							<ContextMenu.Root>
								<ContextMenu.Trigger
									class="flex items-center gap-2 pl-6 pr-2 py-1 w-full text-left cursor-default transition-colors hover:bg-raised
										{collections.selectedRequestId === request.id ? 'bg-raised' : ''}"
									onclick={() => (collections.selectedRequestId = request.id)}
									ondblclick={() => (renamingId = request.id)}
								>
									<span
										class="font-mono text-2.5 font-bold shrink-0 w-9 {methodColor(request.method)}"
									>
										{request.method}
									</span>
									{#if renamingId === request.id}
										<!-- svelte-ignore a11y_autofocus -->
										<input
											bind:value={request.name}
											autofocus
											class="input-base text-xs py-0.5 flex-1 min-w-0"
											onclick={(e) => e.stopPropagation()}
											onblur={() => commitRequestRename(section, request)}
											onkeydown={(e) => e.key === 'Enter' && commitRequestRename(section, request)}
										/>
									{:else}
										<span class="truncate text-xs flex-1" title={request.path}>{request.name}</span>
									{/if}
								</ContextMenu.Trigger>

								<ContextMenu.Portal>
									<ContextMenu.Content class="menu-content">
										<ContextMenu.Item class="menu-item" onSelect={() => (renamingId = request.id)}>
											<span class="i-lucide-pencil text-3"></span>
											Rename
										</ContextMenu.Item>
										<ContextMenu.Item
											class="menu-item"
											onSelect={() => collections.duplicateRequest(section, request)}
										>
											<span class="i-lucide-copy text-3"></span>
											Duplicate
										</ContextMenu.Item>
										<ContextMenu.Item class="menu-item" onSelect={() => copyUrl(section, request)}>
											<span class="i-lucide-link text-3"></span>
											Copy URL
										</ContextMenu.Item>
										<ContextMenu.Separator class="menu-separator" />
										<ContextMenu.Item
											class="menu-item-bad"
											onSelect={() => collections.removeRequest(section, request)}
										>
											<span class="i-lucide-trash-2 text-3"></span>
											Delete request
										</ContextMenu.Item>
									</ContextMenu.Content>
								</ContextMenu.Portal>
							</ContextMenu.Root>
						{:else}
							<p class="pl-6 pr-2 py-1 text-2.5 text-muted">No requests yet.</p>
						{/each}
					{/if}
				</div>
			{:else}
				<p class="px-3 py-2 text-xs text-muted leading-relaxed">
					{#if searching}
						Nothing matches “{query}”.
					{:else}
						No sections yet. A section holds a base URL — requests inside it just need a path.
						Right-click anything for options.
					{/if}
				</p>
			{/each}
		</div>

		{#if collections.error}
			<p class="px-3 py-2 text-2.5 text-bad border-t border-border shrink-0">
				{collections.error}
			</p>
		{/if}
	{:else}
		<div class="flex-1 overflow-y-auto min-h-0">
			{#each history.entries as entry (entry.id)}
				<ContextMenu.Root>
					<ContextMenu.Trigger
						class="block w-full text-left px-3 py-2 border-b border-border/50 hover:bg-raised transition-colors cursor-default"
						onclick={() => onPickHistory(entry)}
					>
						<div class="flex items-center gap-2">
							<span class="font-mono text-2.5 font-bold w-9 shrink-0 {methodColor(entry.method)}">
								{entry.method}
							</span>
							<!-- Fixed-width status slot: spinner, error icon and status code
							     all occupy the same space, so rows never reflow. -->
							<span class="w-8 shrink-0 font-mono text-2.5">
								{#if entry.pending}
									<span class="i-lucide-loader-circle animate-spin text-muted text-3"></span>
								{:else if entry.error}
									<span class="i-lucide-circle-alert text-bad text-3"></span>
								{:else if entry.response}
									<span class={statusColor(entry.response.status)}>{entry.response.status}</span>
								{/if}
							</span>
							<span class="ml-auto text-2.5 text-muted shrink-0">{clockTime(entry.at)}</span>
						</div>
						<div class="truncate text-2.5 text-muted mt-0.5" title={entry.url}>{entry.url}</div>
					</ContextMenu.Trigger>

					<ContextMenu.Portal>
						<ContextMenu.Content class="menu-content">
							<ContextMenu.Item class="menu-item" onSelect={() => onPickHistory(entry)}>
								<span class="i-lucide-corner-up-left text-3"></span>
								Open
							</ContextMenu.Item>
							<ContextMenu.Item
								class="menu-item"
								onSelect={() => navigator.clipboard.writeText(entry.url)}
							>
								<span class="i-lucide-link text-3"></span>
								Copy URL
							</ContextMenu.Item>
							<ContextMenu.Separator class="menu-separator" />
							<ContextMenu.Item class="menu-item-bad" onSelect={() => history.remove(entry.id)}>
								<span class="i-lucide-trash-2 text-3"></span>
								Remove entry
							</ContextMenu.Item>
						</ContextMenu.Content>
					</ContextMenu.Portal>
				</ContextMenu.Root>
			{:else}
				<p class="px-3 py-2 text-xs text-muted leading-relaxed">
					No requests yet. Hit <kbd class="font-mono">⌘↵</kbd> to send.
				</p>
			{/each}
		</div>
	{/if}

	<footer class="flex items-center gap-2 px-2 h-8 border-t border-border shrink-0">
		<button
			class="flex items-center gap-1.5 px-1.5 py-1 rounded text-2.5 text-muted hover:(bg-raised text-text) transition-colors"
			title="Theme: {theme.mode}"
			onclick={() => theme.cycle()}
		>
			<span class={themeIcon}></span>
			<span class="capitalize">{theme.mode}</span>
		</button>
		<span class="ml-auto text-2.5 text-muted">
			<kbd class="font-mono">⌘K</kbd> search
		</span>
	</footer>
</aside>
