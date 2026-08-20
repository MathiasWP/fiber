<script lang="ts">
	import { ContextMenu, Dialog } from 'bits-ui';
	import {
		LOOSE_SECTION_ID,
		methodColor,
		normalizeBaseUrl,
		statusColor,
		type SavedRequest,
		type Section
	} from '$lib/api';
	import {
		collections,
		fuzzyScore,
		NEW_REQUEST_NAME,
		type LoadedRow
	} from '$lib/collections.svelte';
	import {
		requestRow as dragRequest,
		sectionHeader,
		watchDrag,
		type DragHint
	} from '$lib/dnd.svelte';
	import { history, type HistoryEntry } from '$lib/history.svelte';
	import { theme } from '$lib/theme.svelte';
	import DotLoader from '$lib/components/DotLoader.svelte';
	import { getVersion } from '@tauri-apps/api/app';

	interface Props {
		onOpenSettings: (section: Section) => void;
		onPickHistory: (entry: HistoryEntry) => void;
	}

	let { onOpenSettings, onPickHistory }: Props = $props();

	let tab = $state<'collections' | 'history'>('collections');

	// Read from the bundle rather than package.json, so what the footer shows is
	// the version that is actually running — which is the number to quote when
	// something misbehaves.
	let version = $state('');
	$effect(() => {
		getVersion().then((value) => (version = value));
	});
	let query = $state('');
	let historyQuery = $state('');
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
			return collections.collectionSections.map((section) => ({
				section,
				requests: section.requests
			}));
		}

		return collections.collectionSections
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
			.filter(
				(entry) => entry.requests.length > 0 || loadedRows(entry.section).length > 0
			);
	});

	// A search hides the collapsed state — matches are no use if you can't see them.
	const searching = $derived(query.trim().length > 0);

	const themeIcon = $derived(theme.resolved === 'dark' ? 'i-lucide-moon' : 'i-lucide-sun');

	function loadedRows(section: Section): LoadedRow[] {
		const rows = collections.rowsFor(section);
		const needle = query.trim();
		if (!needle) return rows;
		return rows
			.map((row) => ({
				row,
				score: fuzzyScore(`${row.request.name} ${row.request.method} ${row.request.path}`, needle)
			}))
			.filter((match) => match.score !== null)
			.sort((a, b) => a.score! - b.score!)
			.map((match) => match.row);
	}

	/**
	 * Leaving History puts every request back on the response it was showing.
	 *
	 * The entry you opened in History is an override that lives only while you
	 * are looking at it; without dropping it here, coming back to Collections
	 * kept showing that entry rather than the request's own current response.
	 */
	function showCollections() {
		tab = 'collections';
		history.stopViewing();
	}

	function selectRequest(id: string) {
		history.stopViewing();
		collections.selectedRequestId = id;
	}

	function selectLoaded(section: Section, row: LoadedRow) {
		history.stopViewing();
		collections.selectLoaded(section, row);
	}

	const looseRequests = $derived.by(() => {
		const section = collections.looseSection;
		if (!section) return [];
		const needle = query.trim();
		if (!needle) return section.requests;
		return section.requests
			.map((request) => ({
				request,
				score: fuzzyScore(`${request.name} ${request.method} ${request.path}`, needle)
			}))
			.filter((match) => match.score !== null)
			.sort((a, b) => a.score! - b.score!)
			.map((match) => match.request);
	});

	async function addLooseRequest() {
		history.stopViewing();
		await collections.createLooseRequest();
	}

	/** Everywhere a request could move to, other than where it already is. */
	function moveTargets(from: Section): { id: string; name: string }[] {
		const targets = collections.collectionSections
			.filter((section) => section.id !== from.id)
			.map((section) => ({ id: section.id, name: section.name }));

		if (from.id !== LOOSE_SECTION_ID) {
			targets.push({ id: LOOSE_SECTION_ID, name: 'No collection' });
		}
		return targets;
	}

	function refresh(section: Section) {
		collections.refresh(section);
	}

	/** Drops user data for an endpoint the loader no longer reports. */
	function dropOverlay(section: Section, id: string) {
		section.overlay = section.overlay.filter((entry) => entry.id !== id);
		if (collections.selectedRequestId === id) collections.selectedRequestId = null;
		collections.flush(section);
	}

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
		await collections.createRequest(section, NEW_REQUEST_NAME);
	}

	function commitRename(section: Section) {
		renamingId = null;
		if (!section.name.trim()) section.name = 'Untitled';
		collections.flush(section);
	}

	function commitRequestRename(section: Section, request: SavedRequest) {
		renamingId = null;
		// Through `rename` rather than assigning: a name typed here is the user's,
		// and stops the name following the path from now on.
		collections.rename(request, request.name);
		collections.flush(section);
	}

	function copyUrl(section: Section, request: SavedRequest) {
		const base = normalizeBaseUrl(section.baseUrl);
		const path = request.path.trim();
		const absolute = /^https?:\/\//.test(path);
		navigator.clipboard.writeText(absolute ? path : `${base}/${path.replace(/^\/+/, '')}`);
	}

	/**
	 * The name of the request an entry came from, when there still is one.
	 *
	 * Plenty of entries outlive their request: sent from scratch, sent by a
	 * loader or the MCP server, or the request has since been deleted. Those get
	 * no name rather than an invented one — the URL underneath is their identity.
	 */
	function requestName(entry: HistoryEntry): string | null {
		return collections.findRequest(entry.requestId)?.request.name ?? null;
	}

	/**
	 * Matched against the request's name, method, URL and status — so `404`,
	 * `POST` and the name now on the row all work.
	 */
	const visibleHistory = $derived.by(() => {
		const needle = historyQuery.trim().toLowerCase();
		if (!needle) return history.entries;
		return history.entries.filter((entry) => {
			const status = entry.response ? String(entry.response.status) : entry.error ? 'error' : '';
			const name = requestName(entry) ?? '';
			return `${name} ${entry.method} ${entry.url} ${status}`.toLowerCase().includes(needle);
		});
	});

	let hint = $state<DragHint>(null);

	/**
	 * The class a row should wear, given where the drop would land.
	 *
	 * A hint arrives as "an edge of some row", but the gap between two rows has
	 * two such names — below the one above, above the one below — and honouring
	 * both made the line jump between them as the pointer crossed the midpoint.
	 * So an edge is first reduced to an insertion index, and exactly one row
	 * draws the line for that index: the row that would be pushed down, or the
	 * last row when the drop lands at the end.
	 */
	function lineFor(listId: string, rows: { id: string }[], index: number): string {
		// Captured, so the narrowing survives into the callbacks below.
		const current = hint;
		if (current?.kind !== 'request' || current.sectionId !== listId) return '';

		const anchor = rows.findIndex((row) => row.id === current.requestId);
		if (anchor < 0) return '';

		const insertAt = anchor + (current.edge === 'bottom' ? 1 : 0);
		if (insertAt === index) return 'drop-above';
		if (insertAt === rows.length && index === rows.length - 1) return 'drop-below';
		return '';
	}

	/** The same reduction, for the list of collections. */
	function sectionLineFor(sectionId: string): string {
		const current = hint;
		if (current?.kind === 'into') return current.sectionId === sectionId ? 'drop-into' : '';
		if (current?.kind !== 'section') return '';

		const ordered = collections.collectionSections;
		const anchor = ordered.findIndex((section) => section.id === current.sectionId);
		const index = ordered.findIndex((section) => section.id === sectionId);
		if (anchor < 0 || index < 0) return '';

		const insertAt = anchor + (current.edge === 'bottom' ? 1 : 0);
		if (insertAt === index) return 'drop-above';
		if (insertAt === ordered.length && index === ordered.length - 1) return 'drop-below';
		return '';
	}

	$effect(() =>
		watchDrag({
			onHint: (next) => (hint = next),
			onDrop: (outcome) => {
				if (outcome.request) {
					collections.moveRequest(outcome.request.from, outcome.request.to);
				} else if (outcome.section) {
					const { movedId, targetId, edge } = outcome.section;
					collections.reorderSections(movedId, targetId, edge);
				}
			}
		})
	);

	function clockTime(at: number) {
		return new Date(at).toLocaleTimeString(undefined, {
			hour: '2-digit',
			minute: '2-digit',
			second: '2-digit'
		});
	}
</script>

{#snippet requestRow(
	section: Section,
	request: SavedRequest,
	indent: string,
	rows: { id: string }[],
	index: number
)}
	<div
		use:dragRequest={{ sectionId: section.id, requestId: request.id }}
		class="draggable-row transition-shadow {lineFor(section.id, rows, index)}"
	>
		<ContextMenu.Root>
		<ContextMenu.Trigger
			class="flex items-center gap-2 {indent} pr-4 py-1 w-full text-left cursor-default transition-colors hover:bg-raised
				{collections.selectedRequestId === request.id ? 'bg-raised' : ''}"
			onclick={() => selectRequest(request.id)}
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

				<!-- Dragging is the quick way; this is the one that always works,
				     and the only one available from the keyboard. -->
				{#if moveTargets(section).length}
					<ContextMenu.Sub>
						<ContextMenu.SubTrigger class="menu-item">
							<span class="i-lucide-corner-down-right text-3"></span>
							Move to
							<span class="i-lucide-chevron-right text-3 ml-auto"></span>
						</ContextMenu.SubTrigger>
						<ContextMenu.SubContent class="menu-content">
							{#each moveTargets(section) as target (target.id)}
								<ContextMenu.Item
									class="menu-item"
									onSelect={() =>
										collections.moveRequest(
											{ sectionId: section.id, requestId: request.id },
											{ sectionId: target.id }
										)}
								>
									{target.name}
								</ContextMenu.Item>
							{/each}
						</ContextMenu.SubContent>
					</ContextMenu.Sub>
				{/if}

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
	</div>
{/snippet}

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
			onclick={() => showCollections()}
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
		{#if tab === 'history'}
			<button
				class="ml-auto p-1 rounded text-muted hover:(bg-raised text-text) transition-colors"
				title="Clear history"
				onclick={() => history.clear()}
			>
				<span class="i-lucide-trash-2"></span>
			</button>
		{:else}
			<!--
				Both icons sit in a 24x24 box, but lucide draws them to different
				heights inside it: file-plus spans y2-22, folder-plus only y3-20. At
				equal box sizes the file looks the taller of the two, which is what
				made this row read as uneven.

				So the boxes are deliberately unequal — 15px and 17.6px, in the ratio
				20:17 — which lands both drawn glyphs on the same 12.5px height. The
				buttons themselves are identical squares, so only the artwork is
				being corrected.
			-->
			<button
				class="ml-auto h-6 w-6 grid place-items-center rounded text-muted hover:(bg-raised text-text) transition-colors"
				title="New request — not in any collection"
				onclick={addLooseRequest}
			>
				<span class="i-lucide-file-plus text-[15px]"></span>
			</button>
			<button
				class="h-6 w-6 grid place-items-center rounded text-muted hover:(bg-raised text-text) transition-colors"
				title="New collection"
				onclick={() => (creating = true)}
			>
				<span class="i-lucide-folder-plus text-[17.6px]"></span>
			</button>
		{/if}
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
					onblur={() => (newBaseUrl = normalizeBaseUrl(newBaseUrl))}
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

		<!--
			The list itself is a context menu target, so the empty space below the
			last collection is somewhere to start from rather than dead pixels.

			Nesting is safe: Bits UI's trigger bails when the event is already
			defaultPrevented, and a row's own menu prevents it on the way up — so
			right-clicking a request opens that request's menu and not this one.
		-->
		<ContextMenu.Root>
			<ContextMenu.Trigger class="flex-1 overflow-y-auto min-h-0">
				{#if collections.looseSection && looseRequests.length}
					<div class="border-b border-border/50 pb-1">
						{#each looseRequests as request, index (request.id)}
							{@render requestRow(collections.looseSection, request, 'pl-4', looseRequests, index)}
						{/each}
					</div>
				{/if}

				{#each visible as { section, requests } (section.id)}
					{@const open = searching || !section.collapsed}
					<div class="border-b border-border/50">
						<div
							use:sectionHeader={{ sectionId: section.id }}
							class="draggable-row transition-shadow {sectionLineFor(section.id)}"
						>
						<ContextMenu.Root>
							<ContextMenu.Trigger
								class="flex items-center gap-1 px-4 py-1.5 w-full text-left hover:bg-raised/60 transition-colors cursor-default"
								onclick={() => toggle(section)}
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
									{section.requests.length + collections.rowsFor(section).length}
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
									{#if section.loader}
										<ContextMenu.Item class="menu-item" onSelect={() => refresh(section)}>
											<span
												class="i-lucide-refresh-cw text-3 {collections.loading[section.id]
													? 'animate-spin'
													: ''}"
											></span>
											Refresh endpoints
										</ContextMenu.Item>
									{/if}
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
						</div>

						{#if open}
							{#each requests as request, index (request.id)}
								{@render requestRow(section, request, 'pl-8', requests, index)}
							{/each}

							<!-- Loader output. Regenerated on every refresh; the user's
							     bodies live in the section's overlay and survive it. -->
							{#each loadedRows(section) as row (row.request.id)}
								<ContextMenu.Root>
									<ContextMenu.Trigger
										class="flex items-center gap-2 pl-8 pr-4 py-1 w-full text-left cursor-default transition-colors hover:bg-raised
											{collections.selectedRequestId === row.request.id ? 'bg-raised' : ''}"
										onclick={() => selectLoaded(section, row)}
									>
										<span
											class="font-mono text-2.5 font-bold shrink-0 w-9 {methodColor(row.request.method)}"
										>
											{row.request.method}
										</span>
										<span
											class="truncate text-xs flex-1 {row.missing ? 'text-muted line-through' : ''}"
											title={row.missing
												? `${row.request.path} — no longer reported by the loader`
												: row.request.path}
										>
											{row.request.name}
										</span>
										{#if row.missing}
											<span
												class="i-lucide-unlink text-3 text-warn shrink-0"
												title="No longer reported by the loader"
											></span>
										{/if}
									</ContextMenu.Trigger>

									<ContextMenu.Portal>
										<ContextMenu.Content class="menu-content">
											<ContextMenu.Item class="menu-item" onSelect={() => refresh(section)}>
												<span class="i-lucide-refresh-cw text-3"></span>
												Refresh endpoints
											</ContextMenu.Item>
											<ContextMenu.Item class="menu-item" onSelect={() => copyUrl(section, row.request)}>
												<span class="i-lucide-link text-3"></span>
												Copy URL
											</ContextMenu.Item>
											{#if row.missing}
												<ContextMenu.Separator class="menu-separator" />
												<ContextMenu.Item
													class="menu-item-bad"
													onSelect={() => dropOverlay(section, row.request.id)}
												>
													<span class="i-lucide-trash-2 text-3"></span>
													Forget this endpoint
												</ContextMenu.Item>
											{/if}
										</ContextMenu.Content>
									</ContextMenu.Portal>
								</ContextMenu.Root>
							{/each}

							{#if requests.length === 0 && loadedRows(section).length === 0}
								<p class="pl-8 pr-4 py-1 text-2.5 text-muted">
									{section.loader ? 'No endpoints loaded yet.' : 'No requests yet.'}
								</p>
							{/if}
						{/if}
					</div>
				{:else}
					<p class="px-4 py-2 text-xs text-muted leading-relaxed">
						{#if searching}
							Nothing matches “{query}”.
						{:else}
							No sections yet. A section holds a base URL — requests inside it just need a path.
							Right-click anything for options.
						{/if}
					</p>
				{/each}
			</ContextMenu.Trigger>

			<ContextMenu.Portal>
				<ContextMenu.Content class="menu-content">
					<ContextMenu.Item class="menu-item" onSelect={() => (creating = true)}>
						<span class="i-lucide-folder-plus text-3"></span>
						New collection
					</ContextMenu.Item>
					<ContextMenu.Item class="menu-item" onSelect={addLooseRequest}>
						<span class="i-lucide-file-plus text-3"></span>
						New request
					</ContextMenu.Item>
				</ContextMenu.Content>
			</ContextMenu.Portal>
		</ContextMenu.Root>

		{#if collections.error}
			<p class="px-4 py-2 text-2.5 text-bad border-t border-border shrink-0">
				{collections.error}
			</p>
		{/if}
	{:else}
		<div class="px-2 py-2 shrink-0">
			<div class="relative">
				<span
					class="i-lucide-search absolute left-2 top-1/2 -translate-y-1/2 text-muted text-3"
				></span>
				<input
					bind:value={historyQuery}
					spellcheck="false"
					placeholder="Search history…"
					class="input-base w-full text-xs pl-7"
				/>
			</div>
		</div>

		<div class="flex-1 overflow-y-auto min-h-0">
			{#each visibleHistory as entry (entry.id)}
				<ContextMenu.Root>
					<ContextMenu.Trigger
						class="block w-full text-left px-4 py-2 border-b border-border/50 hover:bg-raised transition-colors cursor-default"
						onclick={() => onPickHistory(entry)}
					>
						{@const name = requestName(entry)}
						<div class="flex items-center gap-2">
							<span class="font-mono text-2.5 font-bold w-9 shrink-0 {methodColor(entry.method)}">
								{entry.method}
							</span>
							<!-- Fixed-width status slot: spinner, error icon and status code
							     all occupy the same space, so rows never reflow. -->
							<span class="w-8 shrink-0 font-mono text-2.5">
								{#if entry.pending}
									<DotLoader size={12} class="text-muted" />
								{:else if entry.error}
									<span class="i-lucide-circle-alert text-bad text-3"></span>
								{:else if entry.response}
									<span class={statusColor(entry.response.status)}>{entry.response.status}</span>
								{/if}
							</span>
							{#if name}
								<span class="min-w-0 flex-1 truncate text-xs text-text" title={name}>{name}</span>
							{/if}
							<span class="ml-auto pl-2 text-2.5 text-muted shrink-0">{clockTime(entry.at)}</span>
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
				<p class="px-4 py-2 text-xs text-muted leading-relaxed">
					{#if historyQuery.trim()}
						Nothing matches “{historyQuery}”.
					{:else}
						No requests yet. Hit <kbd class="font-mono">⌘↵</kbd> to send.
					{/if}
				</p>
			{/each}
		</div>
	{/if}

	<footer class="flex items-center gap-2 px-2 h-8 border-t border-border shrink-0">
		<ContextMenu.Root>
			<ContextMenu.Trigger
				class="flex items-center gap-1.5 px-1.5 py-1 rounded text-2.5 text-muted hover:(bg-raised text-text) transition-colors cursor-default"
				title="Switch to {theme.resolved === 'dark' ? 'light' : 'dark'}{theme.mode === 'system'
					? ' — currently following the system'
					: ''}"
				onclick={() => theme.toggle()}
			>
				<span class={themeIcon}></span>
				<span class="capitalize">{theme.resolved}</span>
			</ContextMenu.Trigger>
			<ContextMenu.Portal>
				<ContextMenu.Content class="menu-content">
					<ContextMenu.Item class="menu-item" onSelect={() => theme.followSystem()}>
						<span class="i-lucide-monitor text-3"></span>
						Follow the system
					</ContextMenu.Item>
				</ContextMenu.Content>
			</ContextMenu.Portal>
		</ContextMenu.Root>
		<!-- `mx-auto` rather than a fixed width: equal auto margins park it in the
		     middle of whatever room the two ends leave. -->
		<span class="mx-auto font-mono text-2.5 text-muted tabular-nums" title="Fiber {version}">
			v{version}
		</span>
		<span class="text-2.5 text-muted">
			<kbd class="font-mono">⌘K</kbd> search
		</span>
	</footer>
</aside>
