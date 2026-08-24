<script lang="ts">
	import { ContextMenu, Dialog, Tooltip } from 'bits-ui';
	import {
		applyPathParams,
		LOOSE_SECTION_ID,
		methodColor,
		normalizeBaseUrl,
		statusColor,
		type SavedRequest,
		type Section
	} from '$lib/api';
	import { collections, NEW_REQUEST_NAME, type LoadedRow } from '$lib/collections.svelte';
	import { fuzzyScore, hasRun, order, type Scored } from '$lib/search';
	import {
		requestRow as dragRequest,
		sectionHeader,
		watchDrag,
		type DragHint
	} from '$lib/dnd.svelte';
	import { history, type HistoryEntry } from '$lib/history.svelte';
	import { theme } from '$lib/theme.svelte';
	import DotLoader from '$lib/components/DotLoader.svelte';
	import McpTab from '$lib/components/McpTab.svelte';
	import { getVersion } from '@tauri-apps/api/app';
	import type { Attachment } from 'svelte/attachments';
	import { urlField } from '$lib/urlfield';

	interface Props {
		onOpenSettings: (section: Section, intent?: 'sign-in' | null) => void;
		onPickHistory: (entry: HistoryEntry) => void;
	}

	let { onOpenSettings, onPickHistory }: Props = $props();

	let tab = $state<'collections' | 'history' | 'mcp'>('collections');

	// Read from the bundle rather than package.json, so what the footer shows is
	// the version that is actually running — which is the number to quote when
	// something misbehaves.
	let version = $state('');
	getVersion().then((value) => (version = value));
	let query = $state('');
	let historyQuery = $state('');
	let creating = $state(false);
	let newName = $state('');
	let newBaseUrl = $state('');
	let renamingId = $state<string | null>(null);
	let pendingDelete = $state.raw<Section | null>(null);

	interface VisibleSection {
		section: Section;
		/** Hand-written requests matching the search, or all of them. */
		requests: SavedRequest[];
		/** Loaded endpoints matching the search, or all of them. */
		rows: LoadedRow[];
		/** Everything the collection holds, before the search touched it. */
		total: number;
	}

	const label = (request: SavedRequest) =>
		`${request.name} ${request.method} ${request.path} ${request.tag ?? ''} ${request.description ?? ''}`;

	// A search hides the collapsed state — matches are no use if you can't see them.
	const searching = $derived(query.trim().length > 0);

	/**
	 * Everything the sidebar shows, worked out in one pass.
	 *
	 * `rowsFor` walks the whole loader cache and builds a row per endpoint, so
	 * it is called exactly once per *visible* collection here rather than the
	 * five times it used to be across the derives and the template. Collapsed
	 * collections only need a count, so they skip the allocation entirely.
	 *
	 * The search runs here too, and in two phases: score everything, then decide
	 * whether the scattered matches are worth showing. That decision spans the
	 * whole sidebar, which is why it cannot live inside a per-collection helper.
	 */
	const found = $derived.by(() => {
		const needle = query.trim();
		const loose = collections.looseSection?.requests ?? [];

		if (!needle) {
			return {
				sections: collections.collectionSections.map((section) => {
					// Collapsed: the header only needs a count. `rowsFor` builds a
					// request object per endpoint, which nobody can see until the
					// collection is opened.
					if (section.collapsed) {
						return {
							section,
							requests: [] as SavedRequest[],
							rows: [] as LoadedRow[],
							total: section.requests.length + collections.loadedCount(section)
						};
					}
					const all = collections.rowsFor(section);
					return {
						section,
						requests: section.requests,
						rows: all,
						total: section.requests.length + all.length
					};
				}),
				loose
			};
		}

		const walked = collections.collectionSections.map((section) => ({
			section,
			all: collections.rowsFor(section)
		}));

		const score = <T,>(items: T[], text: (item: T) => string): Scored<T>[] =>
			items
				.map((item) => ({ item, score: fuzzyScore(text(item), needle) }))
				.filter((match): match is Scored<T> => match.score !== null);

		const scored = walked.map(({ section, all }) => ({
			section,
			total: section.requests.length + all.length,
			requests: score(section.requests, label),
			rows: score(all, (row) => label(row.request))
		}));
		const looseScored = score(loose, label);

		const runsOnly =
			hasRun(looseScored) ||
			scored.some((entry) => hasRun(entry.requests) || hasRun(entry.rows));

		return {
			sections: scored.map((entry) => ({
				section: entry.section,
				total: entry.total,
				requests: order(entry.requests, runsOnly),
				rows: order(entry.rows, runsOnly)
			})),
			loose: order(looseScored, runsOnly)
		};
	});

	const scanned = $derived(found.sections);
	const looseRequests = $derived(found.loose);

	const visible = $derived(
		searching ? scanned.filter((entry) => entry.requests.length > 0 || entry.rows.length > 0) : scanned
	);

	/**
	 * A collection can have hundreds of imported or loader-provided endpoints.
	 * Each row carries interactive machinery (a context menu and, for saved
	 * requests, a drag target), so mounting all of them when a header opens
	 * makes that click noticeably slow. Keep the initial mount to a useful first
	 * page, then mount the next page as the sidebar reaches its end.
	 */
	const ENDPOINT_PAGE_SIZE = 100;
	let endpointLimits = $state<Record<string, number>>({});

	function endpointLimit(section: Section): number {
		return endpointLimits[section.id] ?? ENDPOINT_PAGE_SIZE;
	}

	function shownRequests(section: Section, requests: SavedRequest[]): SavedRequest[] {
		return requests.slice(0, endpointLimit(section));
	}

	/**
	 * The loader's endpoints as the sidebar shows them: grouped into folders,
	 * with the page budget spent only on the folders that are open.
	 *
	 * Grouping runs over every row, not just the mounted page, so a folder
	 * header carries the true count of what it holds — a collapsed folder
	 * costs one header and no rows. `remaining` is what a mounted folder still
	 * has waiting, which is what the load-more marker is for; rows hidden
	 * inside a closed folder are not waiting on anything.
	 */
	function pagedGroups(
		section: Section,
		requests: SavedRequest[],
		rows: LoadedRow[]
	): { groups: { tag: string; count: number; rows: LoadedRow[] }[]; remaining: number } {
		let budget = Math.max(0, endpointLimit(section) - requests.length);
		let remaining = Math.max(0, requests.length - endpointLimit(section));

		const groups = groupedLoaded(rows).map((group) => {
			if (!tagOpen(section.id, group.tag)) {
				return { tag: group.tag, count: group.rows.length, rows: [] as LoadedRow[] };
			}
			const shown = group.rows.slice(0, budget);
			budget -= shown.length;
			remaining += group.rows.length - shown.length;
			return { tag: group.tag, count: group.rows.length, rows: shown };
		});

		return { groups, remaining };
	}

	function groupedLoaded(rows: LoadedRow[]): { tag: string; rows: LoadedRow[] }[] {
		const hasTags = rows.some((row) => row.request.tag);
		if (!hasTags) return [{ tag: '', rows }];
		const groups = new Map<string, LoadedRow[]>();
		const order: string[] = [];
		for (const row of rows) {
			const tag = row.request.tag || '';
			if (!groups.has(tag)) {
				groups.set(tag, []);
				order.push(tag);
			}
			groups.get(tag)!.push(row);
		}
		order.sort((a, b) => Number(Boolean(a)) - Number(Boolean(b)) || a.localeCompare(b));
		return order.map((tag) => ({ tag, rows: groups.get(tag)! }));
	}

	/**
	 * Folders the user has opened. Closed is the default: a loader can report
	 * hundreds of endpoints across a dozen tags, and opening a collection to a
	 * wall of them is no better than not grouping at all. The folder names are
	 * the map; you open the one you want.
	 */
	let openTags = $state<Record<string, boolean>>({});

	function tagKey(sectionId: string, tag: string): string {
		return `${sectionId}\0${tag}`;
	}

	function tagOpen(sectionId: string, tag: string): boolean {
		if (searching || !tag) return true;
		return openTags[tagKey(sectionId, tag)] ?? false;
	}

	function toggleTag(sectionId: string, tag: string): void {
		const key = tagKey(sectionId, tag);
		openTags[key] = !openTags[key];
	}

	function loadMoreEndpoints(sectionId: string): void {
		endpointLimits[sectionId] = (endpointLimits[sectionId] ?? ENDPOINT_PAGE_SIZE) + ENDPOINT_PAGE_SIZE;
	}

	/**
	 * Extends a long collection just before its current final row reaches the
	 * sidebar. IntersectionObserver (with the sidebar scroller as `root`) is
	 * the right tool here: several collections share one overflow container,
	 * and a scroll listener would have to measure each sentinel itself.
	 *
	 * Stable attachment identity — `{@attach loadWhenVisible}` rather than a
	 * factory called during render — so a sidebar re-render does not tear the
	 * observer down and immediately fire another page.
	 */
	const loadWhenVisible: Attachment<HTMLElement> = (node) => {
		let loading = false;
		const scroller = node.closest('.sidebar-scroller');
		const observer = new IntersectionObserver(
			([entry]) => {
				if (!entry.isIntersecting || loading) return;
				const sectionId = node.dataset.section;
				if (!sectionId) return;
				loading = true;
				loadMoreEndpoints(sectionId);
				// Svelte applies the new rows before the next frame. The marker
				// moves below the scroller then, and can trigger another page
				// on a later scroll without one intersection producing several
				// pages at once.
				requestAnimationFrame(() => (loading = false));
			},
			{ root: scroller, rootMargin: '300px 0px' }
		);
		observer.observe(node);

		return () => {
			observer.disconnect();
		};
	};

	/**
	 * How much the search is keeping out of sight.
	 *
	 * Counted across every collection rather than the visible ones, because a
	 * collection with no match at all disappears entirely — and those endpoints
	 * are exactly the ones you would otherwise think you had lost.
	 */
	const hidden = $derived.by(() => {
		if (!searching) return 0;

		const total =
			scanned.reduce((sum, entry) => sum + entry.total, 0) +
			(collections.looseSection?.requests.length ?? 0);

		const shown =
			scanned.reduce((sum, entry) => sum + entry.requests.length + entry.rows.length, 0) +
			looseRequests.length;

		return Math.max(0, total - shown);
	});

	const themeIcon = $derived(theme.resolved === 'dark' ? 'i-lucide-moon' : 'i-lucide-sun');

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

	/**
	 * Same reason as above: the MCP tab isn't History either, so the entry you
	 * were looking at stops overriding the response pane.
	 */
	function showMcp() {
		tab = 'mcp';
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

	/**
	 * Collections closed during the search you are running.
	 *
	 * Separate from `section.collapsed` on purpose: while searching, whether a
	 * collection is open is a fact about the search, not about the collection,
	 * and closing one to get it out of the way should not be written to disk and
	 * still be there tomorrow.
	 */
	let closedWhileSearching = $state<Record<string, boolean>>({});

	function setQuery(value: string) {
		query = value;
		// A fresh search opens everything again; closing one is a fact about
		// this query, not the next.
		if (value.trim().length === 0) closedWhileSearching = {};
	}

	function toggle(section: Section) {
		if (searching) {
			closedWhileSearching[section.id] = !closedWhileSearching[section.id];
			return;
		}
		// Closing tears down the currently visible page. Start from the first
		// screen when it is next opened, even if the user had scrolled further.
		if (!section.collapsed) delete endpointLimits[section.id];
		section.collapsed = !section.collapsed;
		collections.touch(section);
	}

	/** Closing without creating anything, from Cancel, Escape or the overlay. */
	function cancelSection() {
		creating = false;
		newName = '';
		newBaseUrl = '';
	}

	async function addSection() {
		// Both fields empty reads as "changed my mind" rather than "make me an
		// untitled collection with nowhere to send anything".
		if (!newName.trim() && !newBaseUrl.trim()) {
			cancelSection();
			return;
		}
		const section = await collections.createSection(newName, newBaseUrl);
		cancelSection();
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
		const path = applyPathParams(request.path, request.pathParams);
		const absolute = /^https?:\/\//.test(path);
		navigator.clipboard.writeText(absolute ? path : `${base}/${path.replace(/^\/+/, '')}`);
	}

	/**
	 * Every request's name by id, built once per collections change.
	 *
	 * The history tab needs a name per entry, per row and again per keystroke of
	 * the filter — and `findRequest` walks every section to answer each one, so
	 * a few hundred entries over a few collections was a scan-per-scan-per-scan.
	 * One map answers all of them.
	 */
	const requestNames = $derived.by(() => {
		const names = new Map<string, string>();
		for (const section of collections.sections) {
			for (const request of section.requests) names.set(request.id, request.name);
			for (const entry of section.overlay) names.set(entry.id, entry.name);
		}
		return names;
	});

	/**
	 * The name of the request an entry came from, when there still is one.
	 *
	 * Plenty of entries outlive their request: sent from scratch, sent by a
	 * loader or the MCP server, or the request has since been deleted. Those get
	 * no name rather than an invented one — the URL underneath is their identity.
	 */
	function requestName(entry: HistoryEntry): string | null {
		return requestNames.get(entry.requestId) ?? null;
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

	/**
	 * How many history rows are rendered before asking. The list can hold 500
	 * entries, each a two-line row with its own context menu — most sessions
	 * never look past the first screenful, and the ones that do get the rest on
	 * request rather than everyone paying for them up front.
	 */
	const HISTORY_PAGE = 100;
	let showAllHistory = $state(false);
	const shownHistory = $derived(
		showAllHistory ? visibleHistory : visibleHistory.slice(0, HISTORY_PAGE)
	);

	let hint = $state.raw<DragHint>(null);

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

	/** Focus the name field when the new-collection dialog opens. */
	const focusName: Attachment<HTMLInputElement> = (node) => {
		node.focus();
	};
</script>

{#snippet requestRow(
	section: Section,
	request: SavedRequest,
	indent: string,
	rows: { id: string }[],
	index: number,
	targets: { id: string; name: string }[]
)}
	<div
		{@attach dragRequest({ sectionId: section.id, requestId: request.id })}
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
				{#if targets.length}
					<ContextMenu.Sub>
						<ContextMenu.SubTrigger class="menu-item">
							<span class="i-lucide-corner-down-right text-3"></span>
							Move to
							<span class="i-lucide-chevron-right text-3 ml-auto"></span>
						</ContextMenu.SubTrigger>
						<ContextMenu.SubContent class="menu-content">
							{#each targets as target (target.id)}
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

{#snippet loadedRow(section: Section, row: LoadedRow, indent: string)}
	<ContextMenu.Root>
		<ContextMenu.Trigger
			class="flex items-center gap-2 {indent} pr-4 py-1 w-full text-left cursor-default transition-colors hover:bg-raised
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
					: row.request.description || row.request.path}
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
{/snippet}

<!-- Deleting a section throws away a file, so it asks first. Requests don't. -->
<!--
	A dialog rather than a strip pushed into the sidebar. Creating a collection is
	a deliberate act with two fields and a decision, which is what a dialog is
	for; inline it competed with the list it was about to add to, and reflowed
	everything below it on the way in and out.
-->
<Dialog.Root
	open={creating}
	onOpenChange={(next) => {
		if (!next) cancelSection();
	}}
>
	<Dialog.Portal>
		<Dialog.Overlay class="dialog-scrim" />
		<Dialog.Content
			class="fixed left-1/2 top-1/2 z-60 w-[min(420px,92vw)] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-panel p-4 shadow-2xl"
		>
			<Dialog.Title class="text-sm font-semibold">New collection</Dialog.Title>
			<Dialog.Description class="mt-1 text-xs text-muted">
				A collection owns a base URL, so the requests inside it need only a path.
			</Dialog.Description>

			<div class="mt-3 flex flex-col gap-2">
				<label class="flex flex-col gap-1">
					<span class="text-xs text-muted">Name</span>
					<!-- Bits UI moves focus to the content on open, so the field has to
					     ask for it; `autofocus` alone loses the race. -->
					<input
						bind:value={newName}
						{@attach creating && focusName}
						placeholder="Payments API"
						class="input-base text-xs"
						onkeydown={(event) => event.key === 'Enter' && addSection()}
					/>
				</label>
				<label class="flex flex-col gap-1">
					<span class="text-xs text-muted">Base URL</span>
					<input
						bind:value={newBaseUrl}
						{@attach urlField}
						spellcheck="false"
						placeholder="https://api.example.com"
						class="input-base text-xs font-mono"
						onblur={() => (newBaseUrl = normalizeBaseUrl(newBaseUrl))}
						onkeydown={(event) => event.key === 'Enter' && addSection()}
					/>
				</label>
			</div>

			<div class="mt-4 flex justify-end gap-1">
				<button class="btn-ghost text-xs" onclick={cancelSection}>Cancel</button>
				<button class="btn-primary text-xs" onclick={addSection}>Create</button>
			</div>
		</Dialog.Content>
	</Dialog.Portal>
</Dialog.Root>

<Dialog.Root
	open={pendingDelete !== null}
	onOpenChange={(next) => {
		if (!next) pendingDelete = null;
	}}
>
	<Dialog.Portal>
		<Dialog.Overlay class="dialog-scrim" />
		<Dialog.Content
			class="fixed left-1/2 top-1/2 z-60 w-[min(420px,90vw)] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-panel p-4 shadow-2xl"
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
		<!--
			MCP is a setup tab rather than a place you keep coming back to, so it
			sits last and carries no actions of its own — the buttons live on the
			rows.
		-->
		<button
			class="px-2 py-1 rounded text-xs transition-colors {tab === 'mcp'
				? 'bg-raised text-text'
				: 'text-muted hover:text-text'}"
			onclick={() => showMcp()}
		>
			MCP
		</button>
		{#if tab === 'history'}
			<button
				class="ml-auto p-1 rounded text-muted hover:bg-raised hover:text-text transition-colors"
				title="Clear history"
				onclick={() => history.clear()}
			>
				<span class="i-lucide-trash-2"></span>
			</button>
		{:else if tab === 'collections'}
			<!--
				square-plus and folder-plus rather than file-plus and folder-plus: the
				geometric pair reads better at 24px, and it happens to solve the
				height problem the old pair had. file-plus drew 20 units tall against
				folder-plus's 17, which needed the two boxes deliberately mismatched
				to look level. square-plus draws 18 against the same 17 — close enough
				that a single size does, and the arithmetic can go.
			-->
			<!--
				These already had `title`, which is a tooltip — just the system's,
				after a second and a half, in its own styling. Bits UI's shows in
				200ms and looks like the rest of the app, which for two unlabelled
				icon buttons is the difference between a hint and a guess.
			-->
			<Tooltip.Provider delayDuration={200}>
				<Tooltip.Root>
					<Tooltip.Trigger
						class="ml-auto h-6 w-6 grid place-items-center rounded text-muted transition-colors hover:bg-border hover:text-text"
						onclick={addLooseRequest}
					>
						<span class="i-lucide-square-plus text-4"></span>
					</Tooltip.Trigger>
					<Tooltip.Portal>
						<Tooltip.Content class="tooltip" sideOffset={6}>
							New request — not in any collection
						</Tooltip.Content>
					</Tooltip.Portal>
				</Tooltip.Root>

				<Tooltip.Root>
					<Tooltip.Trigger
						class="h-6 w-6 grid place-items-center rounded text-muted transition-colors hover:bg-border hover:text-text"
						onclick={() => (creating = true)}
					>
						<span class="i-lucide-folder-plus text-4"></span>
					</Tooltip.Trigger>
					<Tooltip.Portal>
						<Tooltip.Content class="tooltip" sideOffset={6}>New collection</Tooltip.Content>
					</Tooltip.Portal>
				</Tooltip.Root>
			</Tooltip.Provider>
		{/if}
	</header>

	{#if tab === 'mcp'}
		<McpTab />
	{:else if tab === 'collections'}
		<div class="px-2 py-2 shrink-0">
			<div class="relative">
				<span
					class="i-lucide-search absolute left-2 top-1/2 -translate-y-1/2 text-muted text-3"
				></span>
				<input
					bind:value={() => query, setQuery}
					spellcheck="false"
					placeholder="Search endpoints…"
					class="input-base w-full text-xs pl-7"
				/>
			</div>
		</div>

		<!--
			The list itself is a context menu target, so the empty space below the
			last collection is somewhere to start from rather than dead pixels.

			Nesting is safe: Bits UI's trigger bails when the event is already
			defaultPrevented, and a row's own menu prevents it on the way up — so
			right-clicking a request opens that request's menu and not this one.
		-->
		<!-- One provider for the whole list: it renders no element of its own, so
		     it can sit outside the scroller without disturbing the layout. -->
		<Tooltip.Provider delayDuration={200}>
		<ContextMenu.Root>
			<ContextMenu.Trigger class="sidebar-scroller flex-1 overflow-y-auto min-h-0" data-sidebar-scroller>
				{#if collections.looseSection && looseRequests.length}
					{@const looseTargets = moveTargets(collections.looseSection)}
					<div class="border-b border-border/50 pb-1">
						{#each looseRequests as request, index (request.id)}
							{@render requestRow(
								collections.looseSection,
								request,
								'pl-4',
								looseRequests,
								index,
								looseTargets
							)}
						{/each}
					</div>
				{/if}

				{#each visible as { section, requests, rows, total } (section.id)}
					<!-- A search opens everything, because a match you cannot see is
					     no use — but only until you say otherwise. -->
					{@const open = searching
						? !closedWhileSearching[section.id]
						: !section.collapsed}
					{@const displayedRequests = shownRequests(section, requests)}
					{@const paged = pagedGroups(section, requests, rows)}
					{@const remaining = paged.remaining}
					<div class="border-b border-border/50">
						<div
							{@attach sectionHeader({ sectionId: section.id })}
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

								<!--
									Only when the collection wants a credential, and only ever
									about whether one is stored. Whether it still works is not
									knowable from here — an expired cookie is present and correct
									until the server says otherwise — so this flags the case that
									can be checked and is genuinely easy to hit: auth set up, and
									then nothing ever picked.
								-->
								{#if section.auth.kind !== 'none'}
									{@const held = collections.credential[section.id] === true}
									<Tooltip.Root>
										<Tooltip.Trigger>
											{#snippet child({ props })}
												<!-- A span, not the default button: this reports, it does
												     not do anything. -->
												<span
													{...props}
													role="img"
													aria-label={held
														? `Auth: ${section.auth.kind} — a credential is stored`
														: `Auth: ${section.auth.kind} — no credential stored yet`}
													class="shrink-0 text-3 {held
														? 'i-lucide-shield-check text-muted'
														: 'i-lucide-shield-alert text-warn'}"
												></span>
											{/snippet}
										</Tooltip.Trigger>
										<Tooltip.Portal>
											<Tooltip.Content class="tooltip" sideOffset={6}>
												{held
													? `Auth: ${section.auth.kind} — a credential is stored`
													: `Auth: ${section.auth.kind} — no credential stored yet`}
											</Tooltip.Content>
										</Tooltip.Portal>
									</Tooltip.Root>
								{/if}

								<!--
									The same drawer the context menu opens, one click away. Static like the
									count beside it — a cog that faded in on hover would shove the count
									sideways whenever the pointer crossed a row — and the negative margin
									buys a hit area without making the row any taller.
								-->
								<Tooltip.Root>
									<Tooltip.Trigger>
										{#snippet child({ props })}
											<button
												{...props}
												type="button"
												aria-label="Settings for {section.name}"
												onclick={(event) => {
													event.stopPropagation();
													onOpenSettings(section);
												}}
												class="flex shrink-0 items-center rounded p-1 -my-1 text-muted
													transition-colors hover:bg-border hover:text-text"
											>
												<span class="i-lucide-settings text-3"></span>
											</button>
										{/snippet}
									</Tooltip.Trigger>
									<Tooltip.Portal>
										<Tooltip.Content class="tooltip" sideOffset={6}>Section settings</Tooltip.Content>
									</Tooltip.Portal>
								</Tooltip.Root>

								<!--
									Refreshes happen on their own now — at startup and on focus — so
									without this they happen invisibly and endpoints appear to change
									by themselves. Same spinning icon the Refresh menu item uses.
								-->
								{#if collections.loading[section.id]}
									<Tooltip.Root>
										<Tooltip.Trigger>
											{#snippet child({ props })}
												<span
													{...props}
													role="img"
													aria-label="Refreshing endpoints"
													class="i-lucide-refresh-cw text-3 text-muted shrink-0 animate-spin"
												></span>
											{/snippet}
										</Tooltip.Trigger>
										<Tooltip.Portal>
											<Tooltip.Content class="tooltip" sideOffset={6}>
												Refreshing endpoints…
											</Tooltip.Content>
										</Tooltip.Portal>
									</Tooltip.Root>
								{/if}

								<!-- Static: nothing appears or disappears on hover, so nothing shifts. -->
								<Tooltip.Root>
									<Tooltip.Trigger>
										{#snippet child({ props })}
											<span
												{...props}
												role="img"
												aria-label="{total} {total === 1 ? 'endpoint' : 'endpoints'}"
												class="text-2.5 text-muted shrink-0 tabular-nums"
											>
												{total}
											</span>
										{/snippet}
									</Tooltip.Trigger>
									<Tooltip.Portal>
										<Tooltip.Content class="tooltip" sideOffset={6}>
											{total}
											{total === 1 ? 'endpoint' : 'endpoints'}
											{#if section.loader}— loaded and hand-written{/if}
										</Tooltip.Content>
									</Tooltip.Portal>
								</Tooltip.Root>
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
							{@const targets = moveTargets(section)}
							{#each displayedRequests as request, index (request.id)}
								{@render requestRow(section, request, 'pl-8', displayedRequests, index, targets)}
							{/each}

							<!-- Loader output. Regenerated on every refresh; the user's
							     bodies live in the section's overlay and survive it. -->
							{#each paged.groups as group (group.tag || '__none')}
								{#if group.tag}
									<button
										type="button"
										class="flex items-center gap-1 pl-8 pr-4 py-1 w-full text-left text-2.5 text-muted hover:bg-raised/60 hover:text-text transition-colors"
										onclick={() => toggleTag(section.id, group.tag)}
									>
										<span
											class="i-lucide-chevron-right text-3 transition-transform shrink-0 {tagOpen(
												section.id,
												group.tag
											)
												? 'rotate-90'
												: ''}"
										></span>
										<span class="truncate">{group.tag}</span>
										<span class="ml-auto tabular-nums">{group.count}</span>
									</button>
								{/if}
								{#each group.rows as row (row.request.id)}
									{@render loadedRow(section, row, group.tag ? 'pl-12' : 'pl-8')}
								{/each}
							{/each}

							{#if remaining > 0}
								<!-- An invisible marker, not a "load more" control: scrolling
								     stays continuous while only a bounded number of interactive
								     rows are mounted at each step. -->
								<div
									{@attach loadWhenVisible}
									data-section={section.id}
									class="h-px"
									aria-hidden="true"
								></div>
							{/if}

							{#if requests.length === 0 && rows.length === 0}
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

				{#if hidden > 0}
					<!--
						Sticky rather than simply last: the count is what tells you the
						list isn't everything, and that is no use parked at the far end
						of a scroll. Bottom-sticky sits under short results and pins
						itself once they outgrow the pane.
					-->
					<div
						class="sticky bottom-0 flex items-center gap-2 border-t border-border bg-panel px-3 py-1.5 text-2.5 text-muted"
					>
						<span>{hidden.toLocaleString()} more hidden by filters</span>
						<button
							class="ml-auto flex items-center gap-1 rounded px-1.5 py-0.5 -mr-1 hover:bg-raised hover:text-text transition-colors"
							onclick={() => setQuery('')}
						>
							<span class="i-lucide-x text-3"></span>
							Clear filters
						</button>
					</div>
				{/if}
			</ContextMenu.Trigger>

			<ContextMenu.Portal>
				<ContextMenu.Content class="menu-content">
					<ContextMenu.Item class="menu-item" onSelect={() => (creating = true)}>
						<span class="i-lucide-folder-plus text-3"></span>
						New collection
					</ContextMenu.Item>
					<ContextMenu.Item class="menu-item" onSelect={addLooseRequest}>
						<span class="i-lucide-square-plus text-3"></span>
						New request
					</ContextMenu.Item>
				</ContextMenu.Content>
			</ContextMenu.Portal>
		</ContextMenu.Root>
		</Tooltip.Provider>

		{#if collections.error}
			<p class="px-4 py-2 text-2.5 text-bad border-t border-border shrink-0">
				{collections.error}
			</p>
		{/if}

		<!--
			A loader failure, unlike the errors above it, belongs to a section and
			sometimes has a fix. Saying which section is the difference between an
			unattributable red line and something you can act on; the button is
			there for the one cause the user can actually clear.
		-->
		{#if collections.loaderFailure}
			{@const failure = collections.loaderFailure}
			<div class="px-4 py-2 border-t border-border shrink-0 flex flex-col gap-1">
				<!--
					Selectable, because the first thing anyone does with an error
					they can't act on is try to send it to someone who can.
				-->
				<p class="text-2.5 text-bad selectable">
					<span class="font-medium">{failure.sectionName}</span>
					· {failure.message}
				</p>
				<div class="flex items-center gap-2">
					<button
						class="btn-ghost text-2.5"
						onclick={() =>
							navigator.clipboard.writeText(`${failure.sectionName}: ${failure.message}`)}
					>
						<span class="i-lucide-copy"></span>
						Copy
					</button>
					{#if failure.canSignIn}
						<button
							class="btn-ghost text-2.5"
							onclick={() => {
								const section = collections.sections.find((it) => it.id === failure.sectionId);
								if (section) onOpenSettings(section, 'sign-in');
							}}
						>
							<span class="i-lucide-log-in"></span>
							Sign in again
						</button>
					{/if}
					<button class="btn-ghost text-2.5" onclick={() => (collections.loaderFailure = null)}>
						Dismiss
					</button>
				</div>
			</div>
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
			{#each shownHistory as entry (entry.id)}
				<ContextMenu.Root>
					<ContextMenu.Trigger
						class="block w-full text-left px-4 py-2 border-b border-border/50 hover:bg-raised transition-colors cursor-default"
						onclick={() => onPickHistory(entry)}
					>
						{@const name = requestName(entry)}
						<div class="flex items-center gap-2">
							<!--
								Name first, then method, status and time together on the right.
								Those three are the fixed-width part of the row, so keeping them
								adjacent lets the name have all the space that's left.

								Method and status share one tight group rather than each having
								its own column: the method used to sit in a `w-9` box wide
								enough for DELETE, which left GET floating well clear of its
								status code. The status slot keeps a fixed width of its own —
								spinner, error icon and code all occupy the same space, so a row
								doesn't jump about as a request settles.
							-->
							{#if name}
								<span class="min-w-0 flex-1 truncate text-xs text-text" title={name}>{name}</span>
							{:else}
								<!-- Unnamed entries still push the right-hand group over, so
								     every row lines up down the list. -->
								<span class="flex-1"></span>
							{/if}
							<!-- Method, status and time are one group on a single `gap-1`, so
							     the space before the timestamp matches the one between the
							     method and its status rather than the wider gap that separates
							     them from the name. -->
							<span class="flex items-center gap-1 shrink-0">
								<span class="font-mono text-2.5 font-bold {methodColor(entry.method)}">
									{entry.method}
								</span>
								<span class="w-8 shrink-0 font-mono text-2.5">
									{#if entry.pending}
										<DotLoader size={12} class="text-text" />
									{:else if entry.error}
										<span class="i-lucide-circle-alert text-bad text-3"></span>
									{:else if entry.response}
										<span class={statusColor(entry.response.status)}>{entry.response.status}</span>
									{/if}
								</span>
								<span class="text-2.5 text-muted">{clockTime(entry.at)}</span>
							</span>
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

			{#if !showAllHistory && visibleHistory.length > HISTORY_PAGE}
				<button
					class="w-full px-4 py-2 text-2.5 text-muted text-left hover:bg-raised hover:text-text transition-colors"
					onclick={() => (showAllHistory = true)}
				>
					Show all {visibleHistory.length.toLocaleString()} entries
				</button>
			{/if}
		</div>

		{#if history.error}
			<p class="px-4 py-2 text-2.5 text-bad border-t border-border shrink-0">
				{history.error}
			</p>
		{/if}
	{/if}

	<footer class="flex items-center gap-2 px-2 h-8 border-t border-border shrink-0">
		<ContextMenu.Root>
			<ContextMenu.Trigger
				class="flex items-center gap-1.5 px-1.5 py-1 rounded text-2.5 text-muted hover:bg-raised hover:text-text transition-colors cursor-default"
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
		<!-- The ⌘K hint used to live here. A shortcut that common does not need
		     explaining forever, and the version is the thing worth being able to
		     read at a glance when something misbehaves. -->
		<span class="ml-auto font-mono text-2.5 text-muted tabular-nums" title="Fiber {version}">
			v{version}
		</span>
	</footer>
</aside>
