<script lang="ts">
	import { ContextMenu, Tabs } from 'bits-ui';
	import { Pane, PaneGroup, PaneResizer } from 'paneforge';
	import CommandPalette from '$lib/components/CommandPalette.svelte';
	import DotLoader from '$lib/components/DotLoader.svelte';
	import Editor from '$lib/components/Editor.svelte';
	import MethodSelect from '$lib/components/MethodSelect.svelte';
	import SectionSettings from '$lib/components/SectionSettings.svelte';
	import Sidebar from '$lib/components/Sidebar.svelte';
	import { listen } from '@tauri-apps/api/event';
	import { getCurrentWindow } from '@tauri-apps/api/window';
	import {
		applyPathParams,
		BODY_KINDS,
		browserClose,
		cancelRequest,
		flushComplete,
		formatBytes,
		JSON_TOOLING_LIMIT,
		loaderSchema,
		parseQuery,
		pathParamNames,
		resolveUrl,
		sendRequest,
		statusColor,
		tryFormatJson,
		withQuery,
		type BodyKind,
		type QueryParam,
		type SavedRequest,
		type Section
	} from '$lib/api';
	import { validateJsonBody } from '$lib/json-schema';
	import { LOOSE_SECTION_ID } from '$lib/api';
	import { collections, type Selection } from '$lib/collections.svelte';
	import { editorFont } from '$lib/editor.svelte';
	import { history, SCRATCH_ID, type HistoryEntry } from '$lib/history.svelte';
	import { theme } from '$lib/theme.svelte';
	import { urlField } from '$lib/urlfield';
	import { untrack } from 'svelte';

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
		bodyKind: 'json',
		form: [],
		file: '',
		pathParams: [],
		headers: []
	});

	let requestTab = $state('body');
	let responseTab = $state('pretty');
	let inflightId = $state<string | null>(null);
	let paletteOpen = $state(false);

	// E2E: Chromium swallows Ctrl+K for its own find-in-page, so Playwright
	// cannot type the real shortcut. Registered here rather than in an effect
	// so the hook is in place as soon as the page component is created.
	const paletteHooks = (
		window as unknown as { __FIBER_TEST__?: { openPalette?: () => void } }
	).__FIBER_TEST__;
	if (paletteHooks) {
		paletteHooks.openPalette = () => {
			paletteOpen = true;
		};
	}
	let settingsFor = $state.raw<Section | null>(null);
	let resolved = $state('');
	let bodyEditor = $state<Editor>();

	const selection = $derived<Selection | null>(collections.selected);
	/** A loose request has no collection to prefix it with. */
	const inCollection = $derived(
		selection !== null && selection.section.id !== LOOSE_SECTION_ID
	);
	const draft = $derived<SavedRequest>(selection?.request ?? scratch);
	/** The loader's generated body for the selected endpoint, if it has one. */
	const manifestBody = $derived(selection ? collections.manifestBodyFor(selection) : null);
	let bodySchema = $state<unknown | null>(null);
	let responseSchema = $state<unknown | null>(null);
	let schemaToken = 0;
	/** Last section/request the schema effect armed for, so a loader refresh
	 *  can refetch without flashing the banner off first. */
	let schemaKey = '';
	const bodyKind = $derived<BodyKind>(draft.bodyKind ?? 'json');
	const bodySchemaErrors = $derived(
		bodyKind === 'json' ? validateJsonBody(bodySchema, draft.body) : []
	);
	const requestKey = $derived(selection?.request.id ?? SCRATCH_ID);
	const baseUrl = $derived(selection?.section.baseUrl ?? '');
	const bodilessMethod = $derived(draft.method === 'GET' || draft.method === 'HEAD');
	const canSend = $derived(resolved.trim().length > 0 && !inflightId);

	/**
	 * The sidebar remains usable while its settings drawer is open. Persist the
	 * section being left before replacing it; otherwise only the newly opened
	 * section is flushed when the drawer eventually closes.
	 */
	async function openSettings(next: Section) {
		const previous = settingsFor;
		if (previous && previous.id !== next.id) {
			await collections.flush(previous);
			await browserClose(previous.id);
		}
		settingsFor = next;
	}

	/**
	 * The request pane's visible tab.
	 *
	 * Params is always available — query strings are not GET-only. Body is
	 * hidden for GET/HEAD, which have nothing to send; if the last choice was
	 * Body, Params is shown instead rather than writing the user's preference
	 * back.
	 */
	const shownRequestTab = $derived(
		requestTab === 'headers'
			? 'headers'
			: requestTab === 'params' || bodilessMethod
				? 'params'
				: 'body'
	);

	// An entry opened from the History tab wins; otherwise a request shows its
	// own most recent response and never another request's.
	const shown = $derived(history.viewing ?? history.selectedFor(requestKey));

	$effect(() => {
		// Loaders with a TTL refresh after the cached endpoints are on screen,
		// never before — a slow discovery endpoint mustn't delay startup.
		collections.load().then(() => collections.refreshStale());
		history.load();
		editorFont.init();

		const stopTheme = theme.init();
		return stopTheme;
	});

	// Schemas are deliberately fetched endpoint-by-endpoint rather than with the
	// loader cache. A large OpenAPI document often repeats the same component
	// hundreds of times; pulling it across the bridge only when its body opens
	// keeps collection startup and expansion quick.
	$effect(() => {
		const selected = selection;
		// `loadedAt` is the stamp `refresh` writes. Reading it here is what
		// picks up a new schema after a loader run without changing selection.
		const loadedAt = selected ? (collections.loaderCaches[selected.section.id]?.loadedAt ?? 0) : 0;
		const key = selected ? `${selected.section.id}\0${selected.request.id}` : '';
		const token = ++schemaToken;
		if (key !== schemaKey) {
			schemaKey = key;
			bodySchema = null;
			responseSchema = null;
		}
		if (!selected?.section.loader) return;
		void loadedAt;
		loaderSchema(selected.section.id, selected.request.id)
			.then((schemas) => {
				if (token === schemaToken) {
					bodySchema = schemas.request;
					responseSchema = schemas.response;
				}
			})
			.catch(() => {
				// A loader schema is an enhancement. The request stays editable if
				// its cache predates schemas or has been removed from disk.
			});
	});

	/**
	 * The debounced saves are the one thing quitting can lose, so both ways out
	 * wait for them.
	 *
	 * Cmd+Q never reaches the webview — Rust intercepts it, emits
	 * `flush-before-exit`, and holds the exit until `flush_complete` arrives (or
	 * a grace period runs out, so a wedged frontend can't make the app unquittable).
	 * Cmd+W closes the window from this side: the close is cancelled only when
	 * something is actually pending, flushed, and then the window is destroyed
	 * for real — `close()` again would just re-enter this handler.
	 */
	$effect(() => {
		const stopFlush = listen('flush-before-exit', async () => {
			await collections.flushAll();
			await flushComplete();
		});
		const stopClose = getCurrentWindow().onCloseRequested(async (event) => {
			if (!collections.pending) return;
			event.preventDefault();
			await collections.flushAll();
			getCurrentWindow().destroy();
		});
		return () => {
			stopFlush.then((stop) => stop());
			stopClose.then((stop) => stop());
		};
	});

	// Bodies aren't loaded with the history list; pull one in when it's about
	// to be shown.
	$effect(() => {
		/*
		 * `ensureBody` flips `bodyLoaded` while it works and resets it when the
		 * read fails. Those are implementation details, not reasons to run this
		 * effect again: tracking them turns a failed read into a tight retry
		 * loop. A different `shown` entry still re-runs the effect, so leaving
		 * and reopening an entry remains the deliberate retry path.
		 */
		if (shown) untrack(() => history.ensureBody(shown));
	});

	/**
	 * And let go of the one no longer shown. A settled response keeps its full
	 * body on the entry so the pane needs no round trip — but bodies run to
	 * 32 MB, and holding every one for the session is a slow leak. The entry
	 * re-fetches through `ensureBody` if it is ever opened again.
	 */
	let lastShownId: string | null = null;
	$effect(() => {
		const id = shown?.id ?? null;
		if (lastShownId !== null && lastShownId !== id) history.release(lastShownId);
		lastShownId = id;
	});

	$effect(() => {
		theme.apply();
	});

	// Rust owns URL resolution, so the string previewed here is exactly the one
	// that goes out. The token guards against out-of-order replies while typing.
	let resolveToken = 0;
	$effect(() => {
		const base = baseUrl;
		const path = applyPathParams(draft.path, draft.pathParams);
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

	/**
	 * Visits every field without keeping any of it. The visit is the point:
	 * reading a property inside an effect is what subscribes the effect to it,
	 * and a read touches only the reference — unlike the JSON.stringify this
	 * replaces, which copied and escaped every character of every body on every
	 * keystroke just to notice that something had changed.
	 */
	function readDeep(node: unknown): void {
		if (node === null || typeof node !== 'object') return;
		if (Array.isArray(node)) {
			for (const item of node) readDeep(item);
			return;
		}
		for (const key of Object.keys(node)) {
			readDeep((node as Record<string, unknown>)[key]);
		}
	}

	/**
	 * The request the save-watcher last ran for. The effect below fires for two
	 * reasons — a field changed, or the selection moved — and only the first is
	 * an edit. Picking a request must not mark it dirty on arrival.
	 */
	let watchedRequestId: string | null = null;

	// Reading the whole section deeply is what subscribes this effect to every
	// field the user can edit; the effect firing again for the same request is
	// then itself the "something changed" signal `touch` used to recompute.
	$effect(() => {
		const section = selection?.section;
		const requestId = selection?.request.id ?? null;
		if (!section) {
			watchedRequestId = null;
			return;
		}
		readDeep(section);
		if (watchedRequestId !== requestId) {
			watchedRequestId = requestId;
			return;
		}
		collections.touch(section);
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
	 * Removes a header row. The last one is always the blank one waiting to be
	 * typed into, so clearing it is the only sensible reading of deleting it —
	 * and removing it outright would just have the effect above put it straight
	 * back.
	 */
	function removeHeader(index: number): void {
		if (index === draft.headers.length - 1) {
			draft.headers[index].name = '';
			draft.headers[index].value = '';
			return;
		}
		draft.headers.splice(index, 1);
	}

	/** As `removeHeader`, then write the survivors back onto the URL. */
	function removeParam(index: number): void {
		if (index === queryParams.length - 1) {
			queryParams[index].name = '';
			queryParams[index].value = '';
		} else {
			queryParams.splice(index, 1);
		}
		commitParams();
	}

	function removePathParam(index: number): void {
		const params = draft.pathParams ?? [];
		if (index >= params.length) return;
		params[index].value = '';
	}

	function removeFormField(index: number): void {
		if (!draft.form) return;
		if (index === draft.form.length - 1) {
			draft.form[index] = { name: '', value: '', file: '', isFile: draft.form[index].isFile };
			return;
		}
		draft.form.splice(index, 1);
	}

	/**
	 * Whether a row is worth offering to delete.
	 *
	 * A single empty row is the one the table always keeps for typing into, and
	 * there is nothing there to remove — an X beside it only invites a click that
	 * does nothing.
	 */
	function removable(rows: { name: string; value: string }[], index: number): boolean {
		if (rows.length > 1) return true;
		const only = rows[index];
		return Boolean(only && (only.name.trim() || only.value.trim()));
	}

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

	$effect(() => {
		const names = pathParamNames(draft.path);
		const current = draft.pathParams ?? [];
		const held = new Map(current.map((param) => [param.name, param.value]));
		const next = names.map((name) => ({ name, value: held.get(name) ?? '' }));
		if (next.length === 0 && current.length === 0) return;
		const same =
			next.length === current.length &&
			next.every((param, index) => {
				const existing = current[index];
				return existing?.name === param.name && existing.value === param.value;
			});
		if (!same) draft.pathParams = next;
	});

	$effect(() => {
		if (bodyKind !== 'form' && bodyKind !== 'multipart') return;
		if (!draft.form) draft.form = [];
		const last = draft.form[draft.form.length - 1];
		if (!last || last.name.trim() || last.value.trim() || last.file?.trim()) {
			draft.form.push({ name: '', value: '', file: '', isFile: false });
		}
	});

	function commitParams() {
		const next = withQuery(draft.path, queryParams);
		if (next === draft.path) return;
		writtenPath = next;
		draft.path = next;
	}

	const filledParams = $derived(queryParams.filter((param) => param.name.trim().length > 0));
	const filledPathParams = $derived(
		(draft.pathParams ?? []).filter((param) => param.name.trim().length > 0)
	);

	const filledHeaders = $derived(draft.headers.filter((header) => header.name.trim().length > 0));

	/** An empty, non-JSON or oversized body shouldn't be parsed as JSON. */
	const responseLanguage = $derived.by<'json' | 'text'>(() => {
		const response = shown?.response;
		if (!response || response.isBinary || oversized) return 'text';
		const contentType =
			response.headers.find((header) => header.name.toLowerCase() === 'content-type')?.value ?? '';
		return /json/i.test(contentType) ? 'json' : 'text';
	});

	/**
	 * What the response pane says while a request is out.
	 *
	 * One line per request, chosen when it is sent and held until it lands —
	 * changing it mid-wait would draw the eye back to a pane that has nothing new
	 * to report. The novelty is meant to be per request, not per second.
	 *
	 * Every line has to read sensibly after 40ms as well as after 40 seconds,
	 * which rules out anything about how long this is taking.
	 */
	const WAITING_MESSAGES = [
		'Waiting for the server.',
		'Somewhere, a database is thinking.',
		'Packets away.',
		'Asking nicely.',
		'The request is out there.',
		'Holding the connection open.',
		'Off it goes.',
		'Listening for a reply.',
		'Sent. Now we find out.',
		'Over to them.'
	];

	let waitingMessage = $state(WAITING_MESSAGES[0]);

	/** Never the same line twice running — repetition is what makes it feel canned. */
	function differentMessage(previous: string): string {
		const others = WAITING_MESSAGES.filter((message) => message !== previous);
		return others[Math.floor(Math.random() * others.length)];
	}

	const shownBody = $derived(shown?.body ?? '');

	/**
	 * Whether the shown body is past the point where JSON tooling helps.
	 * Everything downstream keys off this one answer: no pretty-print, no
	 * syntax tree, no lint pass — just the text, which is the only thing that
	 * stays interactive at that size.
	 */
	const oversized = $derived(shownBody.length > JSON_TOOLING_LIMIT);

	const responseSchemaErrors = $derived.by(() => {
		if (!responseSchema || !shown?.response || shown.response.isBinary || oversized) return [];
		return validateJsonBody(responseSchema, shownBody);
	});

	const responseImage = $derived.by(() => {
		const response = shown?.response;
		if (!response?.isBinary) return null;
		const type =
			response.headers.find((header) => header.name.toLowerCase() === 'content-type')?.value ??
			'';
		const mime = type.split(';')[0].trim() || 'application/octet-stream';
		if (!mime.startsWith('image/')) return null;
		return `data:${mime};base64,${shownBody}`;
	});

	/** Body streamed so far, while the request is still in flight. */
	const streaming = $derived(shown?.pending ? (shown.body ?? '') : '');

	const responseText = $derived.by(() => {
		if (!shown?.response || shown.response.isBinary) return '';
		// `tryFormatJson` refuses oversized input on its own; skipping the call
		// keeps this from even branching on a string that large.
		return responseTab === 'pretty' && !oversized ? tryFormatJson(shownBody) : shownBody;
	});

	async function send() {
		if (!canSend) return;

		const id = crypto.randomUUID();
		const url = resolved.trim();
		const outgoing = filledHeaders.map((header) => ({ ...header }));

		// A JSON body with no declared type is almost always a mistake, so we
		// fill it in — but only when the user hasn't said otherwise.
		const hasContentType = outgoing.some((h) => h.name.toLowerCase() === 'content-type');
		const kind = bodyKind;
		const sendTextBody =
			!bodilessMethod && (kind === 'json' || kind === 'text') && draft.body.trim().length > 0;
		const outgoingForm = bodilessMethod
			? []
			: (draft.form ?? [])
					.filter((field) => field.name.trim().length > 0)
					.map((field) =>
						kind === 'form'
							? { ...field, file: '', isFile: false }
							: { ...field }
					);
		if (sendTextBody && kind === 'json' && !hasContentType) {
			outgoing.push({ name: 'Content-Type', value: 'application/json' });
		}

		inflightId = id;
		waitingMessage = differentMessage(waitingMessage);
		history.start({
			id,
			requestId: requestKey,
			at: Date.now(),
			method: draft.method,
			url,
			requestBody: sendTextBody ? draft.body : ''
		});

		// Chunks can land faster than the editor can usefully repaint, so they are
		// coalesced into one update a frame. Without this a fast stream spends
		// more time re-rendering than reading the socket.
		let buffered = '';
		let frame = 0;
		const flush = () => {
			frame = 0;
			if (!buffered) return;
			history.stream(id, buffered);
			buffered = '';
		};

		try {
			const response = await sendRequest(
				{
					id,
					requestId: requestKey,
					sectionId: selection?.section.id ?? null,
					method: draft.method,
					url,
					headers: outgoing,
					body: sendTextBody ? draft.body : null,
					bodyKind: kind,
					form: outgoingForm,
					file: bodilessMethod ? '' : (draft.file ?? ''),
					pathParams: (draft.pathParams ?? []).map((param) => ({ ...param })),
					timeoutMs: selection?.section.timeoutMs ?? 60_000,
					followRedirects: selection?.section.followRedirects ?? true,
					acceptInvalidCerts: selection?.section.acceptInvalidCerts ?? false,
					proxy: selection?.section.proxy ?? ''
				},
				(event) => {
					if (event.event === 'start') {
						// A retry after a 401. Drop what the first attempt streamed,
						// including anything still waiting for a frame.
						buffered = '';
						history.restartBody(id);
						return;
					}
					buffered += event.data.text;
					frame ||= requestAnimationFrame(flush);
				}
			);
			history.settle(id, { response });
		} catch (error) {
			history.settle(id, { error: String(error) });
		} finally {
			// Whatever is still buffered is about to be replaced by the settled
			// body, so the pending frame has nothing left to do.
			if (frame) cancelAnimationFrame(frame);
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

	// ⌘+ arrives as '=' unshifted and '+' shifted, and on some layouts as
	// 'Add' from the numeric keypad. Same story for ⌘-.
	const ZOOM_IN = new Set(['=', '+', 'Add']);
	const ZOOM_OUT = new Set(['-', '_', 'Subtract']);

	function onKeydown(event: KeyboardEvent) {
		const meta = event.metaKey || event.ctrlKey;
		if (meta && event.key === 'Enter') {
			// Enter belongs to the open dialog (create, settings, picker, palette),
			// not to the request hidden behind it.
			if ((event.target as Element | null)?.closest?.('[role="dialog"]')) return;
			event.preventDefault();
			send();
		} else if (meta && event.key.toLowerCase() === 'k') {
			event.preventDefault();
			paletteOpen = true;
		} else if (meta && ZOOM_IN.has(event.key)) {
			// preventDefault matters here beyond the usual: without it the webview
			// takes ⌘+ as its own page zoom and scales the entire interface.
			event.preventDefault();
			editorFont.bigger();
		} else if (meta && ZOOM_OUT.has(event.key)) {
			event.preventDefault();
			editorFont.smaller();
		} else if (meta && event.key === '0') {
			event.preventDefault();
			editorFont.reset();
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

<svelte:window onkeydown={onKeydown} onfocus={() => collections.refreshStale()} />

<CommandPalette
	bind:open={paletteOpen}
	onSelect={(match) => {
		history.stopViewing();
		collections.select(match);
	}}
/>


<div class="h-screen bg-bg text-text">
	<PaneGroup direction="horizontal" autoSaveId="fiber:sidebar">
		<Pane defaultSize={20} minSize={12} maxSize={40}>
			<Sidebar
				onOpenSettings={openSettings}
				onPickHistory={openHistory}
			/>
		</Pane>

		<PaneResizer
			class="w-1 shrink-0 cursor-col-resize bg-border/50 transition-colors hover:bg-accent data-[active]:bg-accent"
		/>

		<Pane defaultSize={80}>
			<!-- `relative` is what the settings drawer anchors to: it opens from this
			     pane's left edge, which is the sidebar's right edge. -->
			<main class="relative grid grid-rows-[auto_1fr] min-h-0 min-w-0 h-full">
				<SectionSettings section={settingsFor} onClose={() => (settingsFor = null)} />
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
								{@attach urlField}
								spellcheck="false"
								placeholder="/user/get"
								class="input-base h-8 rounded-l-none flex-1 min-w-0 font-mono selectable"
							/>
						{:else}
							<input
								bind:value={draft.path}
								{@attach urlField}
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
				{#if draft.description}
					<p
						class="px-3 py-1.5 text-2.5 text-muted leading-relaxed line-clamp-2 border-b border-border bg-panel"
						title={draft.description}
					>
						{draft.description}
					</p>
				{/if}

				<!-- Input | output. This split is the app. -->
				<PaneGroup direction="horizontal" autoSaveId="fiber:main" class="min-h-0 min-w-0">
					<Pane defaultSize={50} minSize={20}>
						<section class="flex flex-col min-h-0 min-w-0 h-full">
							<Tabs.Root
								value={shownRequestTab}
								onValueChange={(next) => {
									if (next) requestTab = next;
								}}
								class="flex flex-col h-full min-h-0"
							>
								<Tabs.List
									class="flex items-center gap-1 px-2 h-9 border-b border-border bg-panel shrink-0"
								>
									<Tabs.Trigger
										value="params"
										class="px-2 py-1 rounded text-xs text-muted data-[state=active]:bg-raised data-[state=active]:text-text hover:text-text transition-colors"
									>
										Params{filledParams.length + filledPathParams.length
											? ` (${filledParams.length + filledPathParams.length})`
											: ''}
									</Tabs.Trigger>
									{#if !bodilessMethod}
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

									{#if shownRequestTab === 'body'}
										<select
											class="input-base ml-auto h-7 w-auto text-xs"
											value={bodyKind}
											onchange={(event) => {
												draft.bodyKind = (event.currentTarget.value || 'json') as BodyKind;
											}}
										>
											{#each BODY_KINDS as kind (kind.value)}
												<option value={kind.value}>{kind.label}</option>
											{/each}
										</select>
										{#if manifestBody !== null && bodyKind === 'json'}
											<!-- Back to the generated skeleton, placeholders and all.
											     Filling a body in is destructive to the gaps that guided
											     it; this is the way back. Undo undoes it, so it need not
											     ask first. -->
											<button
												class="btn-ghost text-xs px-2 py-1"
												disabled={draft.body === manifestBody}
												title={draft.body === manifestBody
													? 'The body already matches the generated one'
													: 'Restore the generated body and its placeholders'}
												onclick={() => (draft.body = manifestBody)}
											>
												Reset
											</button>
										{/if}
										<button
											class="btn-ghost text-xs px-2 py-1 {manifestBody === null ||
											bodyKind !== 'json'
												? 'ml-0'
												: ''}"
											disabled={bodyKind !== 'json' && bodyKind !== 'text'}
											onclick={() => bodyEditor?.format()}
										>
											Format
										</button>
									{/if}
								</Tabs.List>

								<Tabs.Content value="body" class="flex-1 min-h-0 flex flex-col">
									{#if bodyKind === 'form' || bodyKind === 'multipart'}
										<div class="flex-1 min-h-0 overflow-y-auto p-2">
											<div class="flex flex-col gap-1">
												{#each draft.form ?? [] as field, index (field)}
													<div class="flex gap-1 items-center">
														<input
															bind:value={field.name}
															spellcheck="false"
															placeholder="Field"
															class="input-base flex-1 font-mono text-xs"
														/>
														{#if bodyKind === 'multipart'}
															<label
																class="flex items-center gap-1 text-2.5 text-muted shrink-0"
																title="Send this field as a file"
															>
																<input type="checkbox" bind:checked={field.isFile} />
																File
															</label>
														{/if}
														{#if field.isFile && bodyKind === 'multipart'}
															<input
																bind:value={field.file}
																spellcheck="false"
																placeholder="/path/to/file"
																class="input-base flex-[2] font-mono text-xs"
															/>
														{:else}
															<input
																bind:value={field.value}
																spellcheck="false"
																placeholder="Value"
																class="input-base flex-[2] font-mono text-xs"
															/>
														{/if}
														<span class="w-6 shrink-0">
															{#if removable(draft.form ?? [], index)}
																<button
																	class="w-6 h-6 grid place-items-center rounded text-muted hover:bg-bad/10 hover:text-bad transition-colors"
																	title={index === (draft.form?.length ?? 0) - 1
																		? 'Clear'
																		: 'Remove field'}
																	onclick={() => removeFormField(index)}
																>
																	<span class="i-lucide-x text-3"></span>
																</button>
															{/if}
														</span>
													</div>
												{/each}
											</div>
											<p class="mt-2 text-2.5 text-muted">
												{#if bodyKind === 'multipart'}
													Multipart fields. Tick File and paste an absolute path — Fiber reads
													the file when you send.
												{:else}
													Sent as <span class="font-mono">application/x-www-form-urlencoded</span>.
												{/if}
											</p>
										</div>
									{:else if bodyKind === 'file'}
										<div class="flex-1 min-h-0 p-3 flex flex-col gap-2">
											<label class="flex flex-col gap-1">
												<span class="text-xs text-muted">File path</span>
												<input
													bind:value={draft.file}
													spellcheck="false"
													placeholder="/absolute/path/to/file"
													class="input-base font-mono text-xs"
												/>
											</label>
											<p class="text-2.5 text-muted leading-relaxed">
												The file is read when you send, not stored in the collection. Paste an
												absolute path.
											</p>
										</div>
									{:else}
										{#key draft.id}
											<div class="flex-1 min-h-0">
												<Editor
													bind:this={bodyEditor}
													bind:value={draft.body}
													placeholder={bodyKind === 'json' ? '{}' : ''}
													scope="request"
													schema={bodyKind === 'json' ? bodySchema : null}
												/>
											</div>
										{/key}
										{#if bodySchemaErrors.length}
											<div
												class="border-t border-bad/40 bg-bad/8 px-3 py-2 text-xs text-bad"
												role="alert"
											>
												<p class="font-medium">Request body does not match the OpenAPI schema</p>
												<ul class="mt-1 list-disc pl-4 font-mono text-2.5 leading-relaxed">
													{#each bodySchemaErrors as error (error)}
														<li>{error}</li>
													{/each}
												</ul>
											</div>
										{/if}
									{/if}
								</Tabs.Content>

								<!-- Editing a row rewrites the query on `draft.path`, so the URL
								     bar above updates as you type and the two never disagree. -->
								<Tabs.Content value="params" class="flex-1 min-h-0 overflow-y-auto p-2">
									{#if filledPathParams.length || pathParamNames(draft.path).length}
										<p class="mb-1 text-2.5 text-muted">Path</p>
										<div class="flex flex-col gap-1 mb-3">
											{#each draft.pathParams ?? [] as param, index (param.name)}
												<div class="flex gap-1">
													<input
														value={param.name}
														readonly
														spellcheck="false"
														class="input-base flex-1 font-mono text-xs opacity-70"
													/>
													<input
														bind:value={param.value}
														spellcheck="false"
														placeholder="Value"
														class="input-base flex-[2] font-mono text-xs"
													/>
													<span class="w-6 shrink-0">
														{#if param.value}
															<button
																class="w-6 h-6 grid place-items-center rounded text-muted hover:bg-bad/10 hover:text-bad transition-colors"
																title="Clear"
																onclick={() => removePathParam(index)}
															>
																<span class="i-lucide-x text-3"></span>
															</button>
														{/if}
													</span>
												</div>
											{/each}
										</div>
										<p class="mb-1 text-2.5 text-muted">Query</p>
									{/if}
									<div class="flex flex-col gap-1">
										{#each queryParams as param, index (param)}
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
												<span class="w-6 shrink-0">
													{#if removable(queryParams, index)}
														<button
															class="w-6 h-6 grid place-items-center rounded text-muted hover:bg-bad/10 hover:text-bad transition-colors"
															title={index === queryParams.length - 1
																? 'Clear'
																: 'Remove parameter'}
															onclick={() => removeParam(index)}
														>
															<span class="i-lucide-x text-3"></span>
														</button>
													{/if}
												</span>
											</div>
										{/each}
									</div>
									<p class="mt-2 text-2.5 text-muted">
										Query values are appended to the URL. Path values replace
										{'{placeholders}'} when you send.
									</p>
								</Tabs.Content>

								<Tabs.Content value="headers" class="flex-1 min-h-0 overflow-y-auto p-2">
									<div class="flex flex-col gap-1">
										{#each draft.headers as header, index (header)}
											<div class="flex gap-1 items-center">
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
												<span class="w-6 shrink-0">
													{#if removable(draft.headers, index)}
														<button
															class="w-6 h-6 grid place-items-center rounded text-muted hover:bg-bad/10 hover:text-bad transition-colors"
															title={index === draft.headers.length - 1 ? 'Clear' : 'Remove header'}
															onclick={() => removeHeader(index)}
														>
															<span class="i-lucide-x text-3"></span>
														</button>
													{/if}
												</span>
											</div>
										{/each}
									</div>
									<p class="mt-2 text-2.5 text-muted">
										Content-Type defaults to application/json for JSON bodies. Form and
										multipart set their own. A collection's own auth header is added on the
										way out and is set in its settings — a Cookie typed here joins it rather
										than replacing it.
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
							{:else if shown.pending && streaming}
								<!-- The body is arriving. Showing it as it lands is the whole
								     point for anything slow or unbounded — an SSE stream never
								     "finishes", so waiting for the end shows nothing, ever.
								     Headers and timings aren't known yet, so this is only the
								     text; the full pane takes over once the request settles. -->
								<div class="flex flex-col h-full min-h-0">
									<div
										class="flex items-center gap-2 px-3 h-9 border-b border-border bg-panel shrink-0 font-mono text-2.5 text-muted"
									>
										<DotLoader size={14} class="text-muted" />
										Streaming
										<span class="ml-auto tabular-nums">
											{streaming.length.toLocaleString()} chars
										</span>
									</div>
									<div class="flex-1 min-h-0">
										<Editor value={streaming} readonly language="text" scope="response" />
									</div>
								</div>
							{:else if shown.pending}
								<!-- Nothing has arrived yet. The whole pane is empty while this
								     shows, so the loader is sized for the space rather than
								     squeezed onto the text baseline: stacked, and large enough
								     to actually read as the helix it is. `text-text` rather than
								     the accent, so it stays the foreground colour in either
								     theme. -->
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
										<!-- Disabled rather than hidden past the size limit: the tab
										     staying put with a reason on hover explains itself, where a
										     tab that vanished would just look broken. The pane falls
										     back to the raw text either way. -->
										<Tabs.Trigger
											value="pretty"
											disabled={oversized}
											title={oversized
												? `Too large to pretty-print — bodies over ${formatBytes(JSON_TOOLING_LIMIT)} are shown raw`
												: undefined}
											class="px-2 py-1 rounded text-xs text-muted data-[state=active]:bg-raised data-[state=active]:text-text hover:text-text transition-colors disabled:opacity-50 disabled:hover:text-muted"
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
											<Tabs.Content value="pretty" class="flex-1 min-h-0 flex flex-col">
												{#if responseImage}
													<div class="h-full overflow-auto p-3 grid place-items-center">
														<img
															src={responseImage}
															alt="Response"
															class="max-h-full max-w-full object-contain"
														/>
													</div>
												{:else if response.isBinary}
													<p class="p-3 text-xs text-muted">
														Binary response ({formatBytes(response.sizeBytes)}). Switch to Raw for
														base64.
													</p>
												{:else}
													<Editor value={responseText} readonly language={responseLanguage} scope="response" />
												{/if}
												{#if responseSchemaErrors.length}
													<div
														class="border-t border-bad/40 bg-bad/8 px-3 py-2 text-xs text-bad"
														role="alert"
													>
														<p class="font-medium">Response does not match the OpenAPI schema</p>
														<ul class="mt-1 list-disc pl-4 font-mono text-2.5 leading-relaxed">
															{#each responseSchemaErrors as error (error)}
																<li>{error}</li>
															{/each}
														</ul>
													</div>
												{/if}
											</Tabs.Content>

											<Tabs.Content value="raw" class="flex-1 min-h-0">
												<Editor value={shownBody} readonly language="text" scope="response" />
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
