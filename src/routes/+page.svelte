<script lang="ts">
	import { ContextMenu, Tabs } from 'bits-ui';
	import { Pane, PaneGroup, PaneResizer } from 'paneforge';
	import CommandPalette from '$lib/components/CommandPalette.svelte';
	import Editor from '$lib/components/Editor.svelte';
	import MethodSelect from '$lib/components/MethodSelect.svelte';
	import SectionSettings from '$lib/components/SectionSettings.svelte';
	import Sidebar from '$lib/components/Sidebar.svelte';
	import {
		cancelRequest,
		formatBytes,
		resolveUrl,
		sendRequest,
		statusColor,
		tryFormatJson,
		type SavedRequest,
		type Section
	} from '$lib/api';
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
		path: 'https://httpbin.org/anything',
		body: '{\n  "hello": "world"\n}',
		headers: []
	});

	let requestTab = $state('body');
	let responseTab = $state('pretty');
	let inflightId = $state<string | null>(null);
	let paletteOpen = $state(false);
	let settingsFor = $state<Section | null>(null);
	let editingBase = $state(false);
	let resolved = $state('');
	let bodyEditor = $state<Editor>();

	const selection = $derived<Selection | null>(collections.selected);
	const draft = $derived<SavedRequest>(selection?.request ?? scratch);
	const requestKey = $derived(selection?.request.id ?? SCRATCH_ID);
	const baseUrl = $derived(selection?.section.baseUrl ?? '');
	const bodilessMethod = $derived(draft.method === 'GET' || draft.method === 'HEAD');
	const canSend = $derived(resolved.trim().length > 0 && !inflightId);

	// Each request only ever shows its own responses.
	const shown = $derived(history.selectedFor(requestKey));
	const mine = $derived(history.forRequest(requestKey));

	$effect(() => {
		collections.load();
		return theme.init();
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

	// Reading the whole section deeply is what subscribes this effect to every
	// field the user can edit; `touch` no-ops when nothing actually changed.
	$effect(() => {
		const section = selection?.section;
		if (section) collections.touch(section);
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

	const filledHeaders = $derived(draft.headers.filter((header) => header.name.trim().length > 0));

	/** An empty or non-JSON body shouldn't be linted as JSON. */
	const responseLanguage = $derived.by<'json' | 'text'>(() => {
		const response = shown?.response;
		if (!response || response.isBinary) return 'text';
		const contentType =
			response.headers.find((header) => header.name.toLowerCase() === 'content-type')?.value ?? '';
		return /json/i.test(contentType) ? 'json' : 'text';
	});

	const responseText = $derived.by(() => {
		const response = shown?.response;
		if (!response || response.isBinary) return '';
		return responseTab === 'pretty' ? tryFormatJson(response.body) : response.body;
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

	/** Jumps to the request a history entry belongs to and shows that response. */
	function openHistory(entry: HistoryEntry) {
		collections.selectedRequestId = entry.requestId === SCRATCH_ID ? null : entry.requestId;
		history.select(entry.requestId, entry.id);
		if (entry.requestId === SCRATCH_ID) {
			scratch.method = entry.method;
			scratch.path = entry.url;
			if (entry.requestBody) scratch.body = entry.requestBody;
		}
	}

	function commitBaseUrl() {
		editingBase = false;
		if (selection) collections.flush(selection.section);
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
	onSelect={(match) => (collections.selectedRequestId = match.request.id)}
/>

<SectionSettings section={settingsFor} onClose={() => (settingsFor = null)} />

<div class="h-screen bg-bg text-text">
	<PaneGroup direction="horizontal" autoSaveId="fetch:sidebar">
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
			<main class="grid grid-rows-[auto_auto_1fr] min-h-0 min-w-0 h-full">
				<!-- URL bar -->
				<div class="flex items-center gap-2 px-3 h-11 border-b border-border bg-panel shrink-0">
					<MethodSelect bind:value={draft.method} />

					<div class="flex-1 flex items-stretch min-w-0">
						{#if selection}
							<!-- The cascade, shown where you type: the section owns the base,
							     the request owns the path. -->
							{#if editingBase}
								<!-- svelte-ignore a11y_autofocus -->
								<input
									bind:value={selection.section.baseUrl}
									autofocus
									spellcheck="false"
									placeholder="https://api.example.com"
									class="input-base rounded-r-none border-r-0 font-mono text-xs w-64 selectable"
									onblur={commitBaseUrl}
									onkeydown={(e) => e.key === 'Enter' && commitBaseUrl()}
								/>
							{:else}
								<button
									class="shrink-0 max-w-64 truncate bg-raised/60 border border-border border-r-0 rounded-l px-2 font-mono text-xs text-muted hover:text-text transition-colors"
									title="{selection.section.name} base URL — click to edit, or right-click the section for settings"
									onclick={() => (editingBase = true)}
								>
									{selection.section.baseUrl || 'set base URL'}
								</button>
							{/if}
							<input
								bind:value={draft.path}
								spellcheck="false"
								placeholder="/user/get"
								class="input-base rounded-l-none flex-1 min-w-0 font-mono selectable"
							/>
						{:else}
							<input
								bind:value={draft.path}
								spellcheck="false"
								placeholder="https://api.example.com/users"
								class="input-base flex-1 min-w-0 font-mono selectable"
							/>
						{/if}
					</div>

					{#if inflightId}
						<button class="btn-base bg-bad text-white hover:bg-bad/85" onclick={cancel}>
							<span class="i-lucide-square"></span>
							Cancel
						</button>
					{:else}
						<button class="btn-primary" disabled={!canSend} onclick={send}>
							<span class="i-lucide-send"></span>
							Send
						</button>
					{/if}
				</div>

				<!-- What will actually be sent. -->
				<div
					class="flex items-center gap-2 px-3 h-6 border-b border-border bg-panel/50 shrink-0 text-2.5"
				>
					<span class="font-mono text-muted truncate selectable">{resolved || '—'}</span>
					<span class="ml-auto text-muted shrink-0">
						{selection ? selection.section.name : 'Scratch'}
					</span>
				</div>

				<!-- Input | output. This split is the app. -->
				<PaneGroup direction="horizontal" autoSaveId="fetch:main" class="min-h-0 min-w-0">
					<Pane defaultSize={50} minSize={20}>
						<section class="flex flex-col min-h-0 min-w-0 h-full">
							<Tabs.Root bind:value={requestTab} class="flex flex-col h-full min-h-0">
								<Tabs.List
									class="flex items-center gap-1 px-2 h-9 border-b border-border bg-panel shrink-0"
								>
									<Tabs.Trigger
										value="body"
										class="px-2 py-1 rounded text-xs text-muted data-[state=active]:(bg-raised text-text) hover:text-text transition-colors"
									>
										Body
									</Tabs.Trigger>
									<Tabs.Trigger
										value="headers"
										class="px-2 py-1 rounded text-xs text-muted data-[state=active]:(bg-raised text-text) hover:text-text transition-colors"
									>
										Headers{filledHeaders.length ? ` (${filledHeaders.length})` : ''}
									</Tabs.Trigger>

									{#if requestTab === 'body' && !bodilessMethod}
										<button
											class="btn-ghost ml-auto text-xs px-2 py-1"
											onclick={() => bodyEditor?.format()}
										>
											Format
										</button>
									{/if}
								</Tabs.List>

								<Tabs.Content value="body" class="flex-1 min-h-0">
									{#if bodilessMethod}
										<p class="p-3 text-xs text-muted">
											{draft.method} requests are sent without a body.
										</p>
									{:else}
										{#key draft.id}
											<Editor bind:this={bodyEditor} bind:value={draft.body} placeholder={'{}'} />
										{/key}
									{/if}
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
								<div class="flex-1 grid place-items-center text-muted text-xs px-4 text-center">
									{mine.length ? 'Pick a response.' : 'Send a request to see the response.'}
								</div>
							{:else if shown.pending}
								<div class="flex-1 grid place-items-center text-muted text-xs">
									<span class="flex items-center gap-2">
										<span class="i-lucide-loader-circle animate-spin"></span>
										Waiting…
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
											class="px-2 py-1 rounded text-xs text-muted data-[state=active]:(bg-raised text-text) hover:text-text transition-colors"
										>
											Pretty
										</Tabs.Trigger>
										<Tabs.Trigger
											value="raw"
											class="px-2 py-1 rounded text-xs text-muted data-[state=active]:(bg-raised text-text) hover:text-text transition-colors"
										>
											Raw
										</Tabs.Trigger>
										<Tabs.Trigger
											value="headers"
											class="px-2 py-1 rounded text-xs text-muted data-[state=active]:(bg-raised text-text) hover:text-text transition-colors"
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

									<!-- This request's earlier responses. Never another request's. -->
									{#if mine.length > 1}
										<div
											class="flex items-center gap-1 px-2 h-7 border-b border-border bg-panel/50 shrink-0 overflow-x-auto"
										>
											{#each mine as entry (entry.id)}
												<button
													class="shrink-0 px-1.5 py-0.5 rounded font-mono text-2.5 transition-colors
														{entry.id === shown.id ? 'bg-raised text-text' : 'text-muted hover:bg-raised/60'}"
													title={new Date(entry.at).toLocaleString()}
													onclick={() => history.select(requestKey, entry.id)}
												>
													{#if entry.pending}
														<span class="i-lucide-loader-circle animate-spin text-3"></span>
													{:else if entry.error}
														<span class="i-lucide-circle-alert text-bad text-3"></span>
													{:else if entry.response}
														<span class={statusColor(entry.response.status)}>
															{entry.response.status}
														</span>
													{/if}
													<span class="ml-1 text-muted">{clockTime(entry.at)}</span>
												</button>
											{/each}
										</div>
									{/if}

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
												<Editor
													value={response.isBinary ? response.body : responseText}
													readonly
													language="text"
												/>
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
													onSelect={() => navigator.clipboard.writeText(response.body)}
												>
													<span class="i-lucide-copy text-3"></span>
													Copy response
												</ContextMenu.Item>
												<ContextMenu.Item
													class="menu-item"
													onSelect={() =>
														navigator.clipboard.writeText(tryFormatJson(response.body))}
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
