<script lang="ts">
	import { ContextMenu, Tabs } from 'bits-ui';
	import { Pane, PaneGroup, PaneResizer } from 'paneforge';
	import CommandPalette from '$lib/components/CommandPalette.svelte';
	import DotLoader from '$lib/components/DotLoader.svelte';
	import Editor from '$lib/components/Editor.svelte';
	import MethodSelect from '$lib/components/MethodSelect.svelte';
	import SectionSettings from '$lib/components/SectionSettings.svelte';
	import Sidebar from '$lib/components/Sidebar.svelte';
	import {
		cancelRequest,
		formatBytes,
		resolveUrl,
		sendRequest,
		parseQuery,
		statusColor,
		tryFormatJson,
		withQuery,
		type QueryParam,
		type SavedRequest,
		type Section
	} from '$lib/api';
	import { LOOSE_SECTION_ID } from '$lib/api';
	import { collections, type Selection } from '$lib/collections.svelte';
	import { history, SCRATCH_ID, type HistoryEntry } from '$lib/history.svelte';
	import { theme } from '$lib/theme.svelte';

	/**
	 * The unsaved request you get before picking anything from the sidebar.
	 * Its `path` is a full URL — there's no section to hang a base off.
	 */
	let scratch = $state<SavedRequest>({
		id: SCRATCH_ID,
		name: 'Scratch',
		method: 'GET',
		path: '',
		body: '{\n  "hello": "world"\n}',
		headers: []
	});

	let requestTab = $state('body');
	let responseTab = $state('pretty');
	let inflightId = $state<string | null>(null);
	let paletteOpen = $state(false);
	let settingsFor = $state<Section | null>(null);
	let resolved = $state('');
	let bodyEditor = $state<Editor>();

	const selection = $derived<Selection | null>(collections.selected);
	/** A loose request has no collection to prefix it with. */
	const inCollection = $derived(
		selection !== null && selection.section.id !== LOOSE_SECTION_ID
	);
	const draft = $derived<SavedRequest>(selection?.request ?? scratch);
	const requestKey = $derived(selection?.request.id ?? SCRATCH_ID);
	const baseUrl = $derived(selection?.section.baseUrl ?? '');
	const bodilessMethod = $derived(draft.method === 'GET' || draft.method === 'HEAD');
	const canSend = $derived(resolved.trim().length > 0 && !inflightId);

	// An entry opened from the History tab wins; otherwise a request shows its
	// own most recent response and never another request's.
	const shown = $derived(history.viewing ?? history.selectedFor(requestKey));

	$effect(() => {
		// Loaders with a TTL refresh after the cached endpoints are on screen,
		// never before — a slow discovery endpoint mustn't delay startup.
		collections.load().then(() => collections.refreshStale());
		history.load();
		return theme.init();
	});

	// Bodies aren't loaded with the history list; pull one in when it's about
	// to be shown.
	$effect(() => {
		if (shown) history.ensureBody(shown);
	});

	$effect(() => {
		theme.apply();
	});

	// Rust owns URL resolution, so the string previewed here is exactly the one
	// that goes out. The token guards against out-of-order replies while typing.
	let resolveToken = 0;
	$effect(() => {
		const base = baseUrl;
		const path = draft.path;
		const token = ++resolveToken;
		resolveUrl(base, path).then((value) => {
			if (token === resolveToken) resolved = value;
		});
	});

	// An unnamed request is named after whatever URL you type into it, until you
	// name it yourself. Runs before `touch` below, so the rename is part of the
	// same debounced write rather than provoking a second one.
	$effect(() => {
		const request = draft;
		// Read so this re-runs on every keystroke in the URL bar.
		void request.path;
		collections.followPath(request);
	});

	// Reading the whole section deeply is what subscribes this effect to every
	// field the user can edit; `touch` no-ops when nothing actually changed.
	$effect(() => {
		const section = selection?.section;
		if (section) collections.touch(section);
	});

	// A method that lost its body tab must not leave the pane on it, and vice
	// versa — Tabs with no matching trigger renders nothing at all.
	$effect(() => {
		if (bodilessMethod && requestTab === 'body') requestTab = 'params';
		if (!bodilessMethod && requestTab === 'params') requestTab = 'body';
	});

	// Keep one blank row at the end of the header table to type into. Blank rows
	// are stripped on the way to disk, so they never show up in a diff.
	$effect(() => {
		const headers = draft.headers;
		const last = headers[headers.length - 1];
		if (!last || last.name.trim() || last.value.trim()) {
			headers.push({ name: '', value: '' });
		}
	});

	/**
	 * The query string, as a table.
	 *
	 * Bodiless methods get this where the body tab would be, because a GET with
	 * a body is a dead tab and its parameters are the thing you actually edit.
	 *
	 * Kept as local state rather than derived straight from the path: an input
	 * bound to a derived value fights whoever is typing into it. The path stays
	 * the single source of truth, and `writtenPath` records what this pane last
	 * wrote so the sync back in can tell a URL someone else changed from an echo
	 * of its own edit.
	 */
	let queryParams = $state<QueryParam[]>([]);
	let writtenPath = '';

	$effect(() => {
		const path = draft.path;
		if (path === writtenPath) return;
		queryParams = parseQuery(path);
	});

	// One blank row to type into, the same way the headers table does it.
	$effect(() => {
		const last = queryParams[queryParams.length - 1];
		if (!last || last.name.trim() || last.value.trim()) {
			queryParams.push({ name: '', value: '' });
		}
	});

	function commitParams() {
		const next = withQuery(draft.path, queryParams);
		if (next === draft.path) return;
		writtenPath = next;
		draft.path = next;
	}

	const filledParams = $derived(queryParams.filter((param) => param.name.trim().length > 0));

	const filledHeaders = $derived(draft.headers.filter((header) => header.name.trim().length > 0));

	/** An empty or non-JSON body shouldn't be linted as JSON. */
	const responseLanguage = $derived.by<'json' | 'text'>(() => {
		const response = shown?.response;
		if (!response || response.isBinary) return 'text';
		const contentType =
			response.headers.find((header) => header.name.toLowerCase() === 'content-type')?.value ?? '';
		return /json/i.test(contentType) ? 'json' : 'text';
	});

	/**
	 * What the response pane says while a request is out.
	 *
	 * Ordered, not shuffled: the first is the plain one almost everyone sees,
	 * because most requests are done before the second arrives. The rest only show
	 * up when something is genuinely slow, which is when a little company helps.
	 */
	const WAITING_MESSAGES = [
		'Waiting for the server.',
		'Still waiting.',
		'Somewhere, a database is thinking.',
		'The request has been sent. The rest is up to them.',
		'Holding the connection open.',
		'This one is taking its time.',
		'Any moment now.',
		'You could have made tea by now.'
	];
	const MESSAGE_MS = 3000;

	let waitingMessage = $state(WAITING_MESSAGES[0]);

	$effect(() => {
		if (!shown?.pending) return;

		// Restart from the top for each request, so a slow one earlier doesn't leave
		// the next starting mid-sequence.
		let at = 0;
		waitingMessage = WAITING_MESSAGES[0];
		const timer = setInterval(() => {
			at = (at + 1) % WAITING_MESSAGES.length;
			waitingMessage = WAITING_MESSAGES[at];
		}, MESSAGE_MS);
		return () => clearInterval(timer);
	});

	const shownBody = $derived(shown?.body ?? '');

	const responseText = $derived.by(() => {
		if (!shown?.response || shown.response.isBinary) return '';
		return responseTab === 'pretty' ? tryFormatJson(shownBody) : shownBody;
	});

	async function send() {
		if (!canSend) return;

		const id = crypto.randomUUID();
		const url = resolved.trim();
		const outgoing = filledHeaders.map((header) => ({ ...header }));

		// A JSON body with no declared type is almost always a mistake, so we
		// fill it in — but only when the user hasn't said otherwise.
		const hasContentType = outgoing.some((h) => h.name.toLowerCase() === 'content-type');
		const sendBody = !bodilessMethod && draft.body.trim().length > 0;
		if (sendBody && !hasContentType) {
			outgoing.push({ name: 'Content-Type', value: 'application/json' });
		}

		inflightId = id;
		history.start({
			id,
			requestId: requestKey,
			at: Date.now(),
			method: draft.method,
			url,
			requestBody: sendBody ? draft.body : ''
		});

		try {
			const response = await sendRequest({
				id,
				requestId: requestKey,
				sectionId: selection?.section.id ?? null,
				method: draft.method,
				url,
				headers: outgoing,
				body: sendBody ? draft.body : null,
				timeoutMs: 60_000,
				followRedirects: true,
				acceptInvalidCerts: false
			});
			history.settle(id, { response });
		} catch (error) {
			history.settle(id, { error: String(error) });
		} finally {
			if (inflightId === id) inflightId = null;
		}
	}

	async function cancel() {
		if (inflightId) await cancelRequest(inflightId);
	}

	/**
	 * Shows a history entry, and jumps to the request it belongs to when there
	 * still is one.
	 *
	 * Plenty of entries have no live request: sent from scratch, sent by a
	 * loader or the MCP server, or their request has since been deleted. Those
	 * load into scratch so the request pane shows what you actually opened —
	 * previously the id simply failed to resolve and you were left looking at an
	 * unrelated scratch request.
	 */
	function openHistory(entry: HistoryEntry) {
		// Only `viewingId`. Emphatically not `select` — that is the request's own
		// active response, which the Collections tab reads, and looking at
		// something in History must not overwrite it. Doing both is what made a
		// request come back showing whichever entry you last inspected.
		history.viewingId = entry.id;

		if (collections.findRequest(entry.requestId)) {
			collections.selectedRequestId = entry.requestId;
			return;
		}

		collections.selectedRequestId = null;
		scratch.method = entry.method;
		scratch.path = entry.url;
		// Unconditionally, including when empty: what's on screen has to be what
		// would be sent, and a leftover body would quietly change that.
		scratch.body = entry.requestBody;
	}

	function onKeydown(event: KeyboardEvent) {
		const meta = event.metaKey || event.ctrlKey;
		if (meta && event.key === 'Enter') {
			event.preventDefault();
			send();
		} else if (meta && event.key.toLowerCase() === 'k') {
			event.preventDefault();
			paletteOpen = true;
		}
	}

	function clockTime(at: number) {
		return new Date(at).toLocaleTimeString(undefined, {
			hour: '2-digit',
			minute: '2-digit',
			second: '2-digit'
		});
	}
</script>

<svelte:window onkeydown={onKeydown} />

<CommandPalette
	bind:open={paletteOpen}
	onSelect={(match) => {
		history.stopViewing();
		collections.select(match);
	}}
/>

<SectionSettings section={settingsFor} onClose={() => (settingsFor = null)} />

<div class="h-screen bg-bg text-text">
	<PaneGroup direction="horizontal" autoSaveId="fiber:sidebar">
		<Pane defaultSize={20} minSize={12} maxSize={40}>
			<Sidebar
				onOpenSettings={(section) => (settingsFor = section)}
				onPickHistory={openHistory}
			/>
		</Pane>

		<PaneResizer
			class="w-1 shrink-0 cursor-col-resize bg-border/50 transition-colors hover:bg-accent data-[active]:bg-accent"
		/>

		<Pane defaultSize={80}>
			<main class="grid grid-rows-[auto_1fr] min-h-0 min-w-0 h-full">
				<!-- URL bar -->
				<div class="flex items-center gap-2 px-3 h-11 border-b border-border bg-panel shrink-0">
					<MethodSelect bind:value={draft.method} />

					<div class="flex-1 flex items-stretch min-w-0" title={resolved}>
						{#if inCollection && selection}
							<!-- The cascade, shown where you type. Read-only: the base URL
							     belongs to the collection, so it's edited in its settings. -->
							<!--
								Sized by its own text, not by a fixed cap. It used to be
								`shrink-0 max-w-64`, which is the wrong way round: the width was
								pinned regardless of the URL, so anything past 16rem was
								truncated even with room to spare beside it.

								`w-max` takes the natural width of the base URL; dropping
								`shrink-0` lets it give that up again when the row is genuinely
								too narrow, at which point `truncate` does its job. The path
								input keeps `min-w-0`, so it can always yield first.
							-->
							<span
								class="w-max min-w-0 truncate flex items-center bg-raised/60 border border-border border-r-0 rounded-l px-2 font-mono text-xs text-muted select-none"
								title="{selection.section.name} · base URL is set in section settings"
							>
								{selection.section.baseUrl || 'no base URL'}
							</span>
							<input
								bind:value={draft.path}
								spellcheck="false"
								placeholder="/user/get"
								class="input-base h-8 rounded-l-none flex-1 min-w-0 font-mono selectable"
							/>
						{:else}
							<input
								bind:value={draft.path}
								spellcheck="false"
								placeholder="https://api.example.com/users"
								class="input-base h-8 flex-1 min-w-0 font-mono selectable"
							/>
						{/if}
					</div>

					{#if inflightId}
						<button class="btn-base h-8 bg-bad text-white hover:bg-bad/85" onclick={cancel}>
							<span class="i-lucide-square"></span>
							Cancel
						</button>
					{:else}
						<button class="btn-primary h-8" disabled={!canSend} onclick={send}>
							<span class="i-lucide-send"></span>
							Send
						</button>
					{/if}
				</div>

				<!-- Input | output. This split is the app. -->
				<PaneGroup direction="horizontal" autoSaveId="fiber:main" class="min-h-0 min-w-0">
					<Pane defaultSize={50} minSize={20}>
						<section class="flex flex-col min-h-0 min-w-0 h-full">
							<Tabs.Root bind:value={requestTab} class="flex flex-col h-full min-h-0">
								<Tabs.List
									class="flex items-center gap-1 px-2 h-9 border-b border-border bg-panel shrink-0"
								>
									<!-- One or the other, never both: a GET has no body to edit and
									 a POST's parameters belong in the URL bar. -->
									{#if bodilessMethod}
										<Tabs.Trigger
											value="params"
											class="px-2 py-1 rounded text-xs text-muted data-[state=active]:bg-raised data-[state=active]:text-text hover:text-text transition-colors"
										>
											Params{filledParams.length ? ` (${filledParams.length})` : ''}
										</Tabs.Trigger>
									{:else}
										<Tabs.Trigger
											value="body"
											class="px-2 py-1 rounded text-xs text-muted data-[state=active]:bg-raised data-[state=active]:text-text hover:text-text transition-colors"
										>
											Body
										</Tabs.Trigger>
									{/if}
									<Tabs.Trigger
										value="headers"
										class="px-2 py-1 rounded text-xs text-muted data-[state=active]:bg-raised data-[state=active]:text-text hover:text-text transition-colors"
									>
										Headers{filledHeaders.length ? ` (${filledHeaders.length})` : ''}
									</Tabs.Trigger>

									{#if requestTab === 'body'}
										<button
											class="btn-ghost ml-auto text-xs px-2 py-1"
											onclick={() => bodyEditor?.format()}
										>
											Format
										</button>
									{/if}
								</Tabs.List>

								<Tabs.Content value="body" class="flex-1 min-h-0">
									{#key draft.id}
										<Editor bind:this={bodyEditor} bind:value={draft.body} placeholder={'{}'} />
									{/key}
								</Tabs.Content>

								<!-- Editing a row rewrites the query on `draft.path`, so the URL
								     bar above updates as you type and the two never disagree. -->
								<Tabs.Content value="params" class="flex-1 min-h-0 overflow-y-auto p-2">
									<div class="flex flex-col gap-1">
										{#each queryParams as param, index (index)}
											<div class="flex gap-1">
												<input
													bind:value={param.name}
													oninput={commitParams}
													spellcheck="false"
													placeholder="Parameter"
													class="input-base flex-1 font-mono text-xs"
												/>
												<input
													bind:value={param.value}
													oninput={commitParams}
													spellcheck="false"
													placeholder="Value"
													class="input-base flex-[2] font-mono text-xs"
												/>
											</div>
										{/each}
									</div>
									<p class="mt-2 text-2.5 text-muted">
										Appended to the URL as a query string. Values are encoded for you.
									</p>
								</Tabs.Content>

								<Tabs.Content value="headers" class="flex-1 min-h-0 overflow-y-auto p-2">
									<div class="flex flex-col gap-1">
										{#each draft.headers as header, index (index)}
											<div class="flex gap-1">
												<input
													bind:value={header.name}
													spellcheck="false"
													placeholder="Header"
													class="input-base flex-1 font-mono text-xs"
												/>
												<input
													bind:value={header.value}
													spellcheck="false"
													placeholder="Value"
													class="input-base flex-[2] font-mono text-xs"
												/>
											</div>
										{/each}
									</div>
									<p class="mt-2 text-2.5 text-muted">
										Content-Type defaults to application/json when a body is present.
									</p>
								</Tabs.Content>
							</Tabs.Root>
						</section>
					</Pane>

					<PaneResizer
						class="w-1 shrink-0 cursor-col-resize bg-border/50 transition-colors hover:bg-accent data-[active]:bg-accent"
					/>

					<Pane defaultSize={50} minSize={20}>
						<section class="flex flex-col min-h-0 min-w-0 h-full">
							{#if !shown}
								<!-- `selectedFor` falls back to the newest entry, so reaching
								     here at all means this request has no history yet. The old
								     "Pick a response." alternative was unreachable. -->
								<div class="flex-1 grid place-items-center text-muted text-xs px-4 text-center">
									Send a request to see the response.
								</div>
							{:else if shown.pending}
								<!-- The whole pane is empty while this shows, so the loader is
								     sized for the space rather than squeezed onto the text
								     baseline: stacked, and large enough to actually read as
								     the helix it is. `text-text` rather than the accent, so it
								     stays the foreground colour in either theme. -->
								<div class="flex-1 grid place-items-center">
									<span class="flex flex-col items-center gap-5">
										<DotLoader size={56} class="text-text" />
										<span class="text-sm text-muted">{waitingMessage}</span>
									</span>
								</div>
							{:else if shown.error}
								<div class="flex-1 p-4 min-h-0 overflow-y-auto">
									<div class="flex items-center gap-2 text-bad font-medium">
										<span class="i-lucide-circle-alert"></span>
										Request failed
									</div>
									<p class="mt-2 font-mono text-xs text-muted selectable">{shown.error}</p>
								</div>
							{:else if shown.response}
								{@const response = shown.response}
								<Tabs.Root bind:value={responseTab} class="flex flex-col h-full min-h-0">
									<Tabs.List
										class="flex items-center gap-1 px-2 h-9 border-b border-border bg-panel shrink-0"
									>
										<Tabs.Trigger
											value="pretty"
											class="px-2 py-1 rounded text-xs text-muted data-[state=active]:bg-raised data-[state=active]:text-text hover:text-text transition-colors"
										>
											Pretty
										</Tabs.Trigger>
										<Tabs.Trigger
											value="raw"
											class="px-2 py-1 rounded text-xs text-muted data-[state=active]:bg-raised data-[state=active]:text-text hover:text-text transition-colors"
										>
											Raw
										</Tabs.Trigger>
										<Tabs.Trigger
											value="headers"
											class="px-2 py-1 rounded text-xs text-muted data-[state=active]:bg-raised data-[state=active]:text-text hover:text-text transition-colors"
										>
											Headers ({response.headers.length})
										</Tabs.Trigger>

										<div class="ml-auto flex items-center gap-3 font-mono text-2.5 text-muted pr-1">
											<span class={statusColor(response.status)}>
												{response.status}
												{response.statusText}
											</span>
											<span title="Time to first byte / total">
												{response.timing.ttfbMs}ms / {response.timing.totalMs}ms
											</span>
											<span>{formatBytes(response.sizeBytes)}</span>
										</div>
									</Tabs.List>

									{#if response.truncated}
										<p
											class="px-3 py-1 text-2.5 text-warn bg-warn/10 border-b border-border shrink-0"
										>
											Response truncated at 32 MB — {formatBytes(response.sizeBytes)} received.
										</p>
									{/if}

									<ContextMenu.Root>
										<ContextMenu.Trigger class="flex-1 min-h-0 flex flex-col">
											<Tabs.Content value="pretty" class="flex-1 min-h-0">
												{#if response.isBinary}
													<p class="p-3 text-xs text-muted">
														Binary response ({formatBytes(response.sizeBytes)}). Switch to Raw for
														base64.
													</p>
												{:else}
													<Editor value={responseText} readonly language={responseLanguage} />
												{/if}
											</Tabs.Content>

											<Tabs.Content value="raw" class="flex-1 min-h-0">
												<Editor value={shownBody} readonly language="text" />
											</Tabs.Content>

											<Tabs.Content value="headers" class="flex-1 min-h-0 overflow-y-auto p-3">
												<dl
													class="grid grid-cols-[minmax(0,auto)_1fr] gap-x-4 gap-y-1 font-mono text-xs selectable"
												>
													{#each response.headers as header (header.name + header.value)}
														<dt class="text-muted">{header.name}</dt>
														<dd class="m-0 break-all">{header.value}</dd>
													{/each}
												</dl>
												{#if response.finalUrl !== shown.url}
													<p class="mt-3 text-2.5 text-muted">
														Redirected to <span class="font-mono">{response.finalUrl}</span>
													</p>
												{/if}
											</Tabs.Content>
										</ContextMenu.Trigger>

										<ContextMenu.Portal>
											<ContextMenu.Content class="menu-content">
												<ContextMenu.Item
													class="menu-item"
													onSelect={() => navigator.clipboard.writeText(shownBody)}
												>
													<span class="i-lucide-copy text-3"></span>
													Copy response
												</ContextMenu.Item>
												<ContextMenu.Item
													class="menu-item"
													onSelect={() => navigator.clipboard.writeText(tryFormatJson(shownBody))}
												>
													<span class="i-lucide-braces text-3"></span>
													Copy formatted
												</ContextMenu.Item>
												<ContextMenu.Item
													class="menu-item"
													onSelect={() => navigator.clipboard.writeText(response.finalUrl)}
												>
													<span class="i-lucide-link text-3"></span>
													Copy URL
												</ContextMenu.Item>
												<ContextMenu.Separator class="menu-separator" />
												<ContextMenu.Item
													class="menu-item-bad"
													onSelect={() => history.clearFor(requestKey)}
												>
													<span class="i-lucide-trash-2 text-3"></span>
													Clear this request's history
												</ContextMenu.Item>
											</ContextMenu.Content>
										</ContextMenu.Portal>
									</ContextMenu.Root>
								</Tabs.Root>
							{/if}
						</section>
					</Pane>
				</PaneGroup>
			</main>
		</Pane>
	</PaneGroup>
</div>
