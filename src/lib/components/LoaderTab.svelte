<script lang="ts">
	import { Select } from 'bits-ui';
	import {
		defaultLoader,
		loaderTemplates,
		loaderPreview,
		loaderProbe,
		methodColor,
		type LoadedEndpoint,
		type Section
	} from '$lib/api';
	import { collections } from '$lib/collections.svelte';
	import Editor from './Editor.svelte';
	import DotLoader from '$lib/components/DotLoader.svelte';
	import { urlField } from '$lib/urlfield';

	interface Props {
		section: Section;
	}

	let { section }: Props = $props();

	/**
	 * The manifest as fetched. Held so the filter can be re-run without
	 * refetching. `$state.raw` because it is only ever replaced wholesale, never
	 * edited in place — and a deep proxy over a whole OpenAPI document meant
	 * proxying thousands of nodes just to hold something read-only.
	 */
	let document = $state.raw<unknown>(null);
	let probing = $state(false);
	let probeError = $state<string | null>(null);

	/** Replaced wholesale as the filter re-runs; never edited in place. */
	let preview = $state.raw<LoadedEndpoint[]>([]);
	let previewError = $state<string | null>(null);

	let templates = $state.raw<[string, string][]>([]);
	let runSummary = $state<string | null>(null);

	/** The template the filter still matches, or nothing once it is edited. */
	const current = $derived(
		templates.find(([, query]) => query === section.loader?.query)?.[0] ?? null
	);

	const running = $derived(collections.loading[section.id] === true);
	const cached = $derived(collections.loaderCaches[section.id]);

	loaderTemplates().then((found) => (templates = found));

	async function addLoader() {
		section.loader = await defaultLoader();
		await collections.flush(section);
	}

	async function removeLoader() {
		section.loader = null;
		document = null;
		preview = [];
		probeError = null;
		previewError = null;
		runSummary = null;
		await collections.flush(section);
	}

	/** Fetches the manifest once; the filter then runs against it locally. */
	async function probe() {
		probing = true;
		probeError = null;
		await collections.flush(section);
		try {
			document = await loaderProbe(section.id);
		} catch (error) {
			probeError = String(error);
			document = null;
		} finally {
			probing = false;
		}
	}

	/**
	 * Re-mapped as you type. Possible at all because a filter is a pure
	 * transformation — nothing is refetched and nothing can escape — but the
	 * document crosses the IPC boundary whole on every call, so the call waits
	 * for a short pause in the typing rather than firing per keystroke. The
	 * token guards against out-of-order replies: a slow run of an old filter
	 * must not land on top of a fast run of the current one.
	 */
	let previewToken = 0;
	$effect(() => {
		const query = section.loader?.query ?? '';
		const input = document;
		const token = ++previewToken;
		if (input === null || !query.trim()) {
			preview = [];
			previewError = null;
			return;
		}

		const timer = setTimeout(() => {
			loaderPreview(input, query)
				.then((endpoints) => {
					if (token !== previewToken) return;
					preview = endpoints;
					previewError = null;
				})
				.catch((error) => {
					if (token !== previewToken) return;
					preview = [];
					previewError = String(error);
				});
		}, 150);
		return () => clearTimeout(timer);
	});

	async function runNow() {
		runSummary = null;
		await collections.flush(section);
		const outcome = await collections.refresh(section);
		if (typeof outcome === 'string') {
			probeError = outcome;
			return;
		}
		const changes = [
			outcome.added.length ? `${outcome.added.length} new` : '',
			outcome.removed.length ? `${outcome.removed.length} removed` : ''
		].filter(Boolean);
		runSummary = `${outcome.endpoints.length} endpoints from ${outcome.pages} page${
			outcome.pages === 1 ? '' : 's'
		}${changes.length ? ` · ${changes.join(', ')}` : ''}`;
	}

	const documentPreview = $derived(document === null ? '' : previewJson(document));

	/**
	 * Pretty-print is for reading. A compact stringify of a large OpenAPI
	 * document is already a full walk; pretty-printing it just to keep the
	 * first 20 KB is a second, much larger copy that froze the settings
	 * drawer after "Fetch a sample".
	 */
	function previewJson(value: unknown, limit = 20_000): string {
		const compact = JSON.stringify(value);
		if (compact.length <= limit) return JSON.stringify(value, null, 2);
		return compact.slice(0, limit);
	}
</script>

{#if !section.loader}
	<p class="text-xs text-muted leading-relaxed">
		A loader keeps this section's endpoints in step with the API. Point it at the endpoint that
		lists your routes, then describe how to turn that response into endpoints with a
		<a href="https://jqlang.org/manual/" target="_blank" rel="noreferrer" class="text-accent">
			jq filter
		</a>.
	</p>
	<div>
		<button class="btn-primary text-xs" onclick={addLoader}>
			<span class="i-lucide-plus"></span>
			Add a loader
		</button>
	</div>
{:else}
	<div class="flex items-center gap-2">
		<label class="flex items-center gap-1.5 text-xs">
			<input type="checkbox" bind:checked={section.loader.enabled} />
			Enabled
		</label>
		<span class="ml-auto text-2.5 text-muted">
			{cached ? `${cached.endpoints.length} endpoints loaded` : 'never run'}
		</span>
		<button class="btn-ghost text-xs" disabled={running} onclick={runNow}>
			{#if running}
				<DotLoader size={14} class="text-text" />
			{/if}
			Run now
		</button>
		<button class="btn-ghost text-xs" onclick={removeLoader}>Remove</button>
	</div>

	<div class="grid grid-cols-[1fr_2fr] gap-2">
		<label class="flex flex-col gap-1">
			<span class="text-xs text-muted">Method</span>
			<input
				bind:value={section.loader.method}
				spellcheck="false"
				class="input-base text-xs font-mono"
			/>
		</label>
		<label class="flex flex-col gap-1">
			<span class="text-xs text-muted">Manifest URL</span>
			<input
				bind:value={section.loader.url}
				{@attach urlField}
				spellcheck="false"
				placeholder="/openapi.json"
				class="input-base text-xs font-mono selectable"
			/>
		</label>
	</div>

	<div class="flex items-center gap-2">
		<button class="btn-ghost text-xs" disabled={probing} onclick={probe}>
			{#if probing}
				<DotLoader size={14} class="text-text" />
			{/if}
			Fetch a sample
		</button>
		<span class="text-2.5 text-muted">
			{document === null
				? 'Fetch once, then edit the filter against it.'
				: 'Filter runs as you type — nothing is refetched.'}
		</span>

		{#if templates.length}
			<Select.Root
				type="single"
				value={section.loader.query}
				onValueChange={(next) => {
					if (next && section.loader) section.loader.query = next;
				}}
			>
				<!-- The chevron is the point: without it a Select trigger reads as a
				     button that does something, rather than one that offers a list.
				     The name matters too — the filter below is the truth, but it is
				     a line of jq, and "OpenAPI" answers "what is this" at a glance.
				     Edit it and the name goes honest rather than stale. -->
				<Select.Trigger class="btn-ghost text-xs ml-auto">
					{current ?? 'Custom'}
					<span class="i-lucide-chevron-down text-3"></span>
				</Select.Trigger>
				<Select.Portal>
					<Select.Content class="menu-content w-72">
						<Select.Viewport>
							{#each templates as [name, query] (name)}
								<Select.Item value={query} label={name} class="menu-item">
									{#snippet children({ selected })}
										<!-- Held rather than hidden, so the names stay in one
										     column whether or not one is ticked. -->
										<span
											class="i-lucide-check text-3 shrink-0 {selected ? '' : 'opacity-0'}"
										></span>
										{name}
									{/snippet}
								</Select.Item>
							{/each}
						</Select.Viewport>
					</Select.Content>
				</Select.Portal>
			</Select.Root>
		{/if}
	</div>

	{#if probeError}
		<p class="text-2.5 text-bad">{probeError}</p>
	{/if}

	<label class="flex flex-col gap-1">
		<span class="text-xs text-muted">Filter</span>
		<div class="h-20 rounded border border-border overflow-hidden">
			<Editor bind:value={section.loader.query} language="text" scope="request" />
		</div>
	</label>

	<!-- Document in, endpoints out, side by side. -->
	<div class="grid grid-cols-2 gap-2 h-48">
		<div class="flex flex-col rounded border border-border overflow-hidden">
			<p class="px-2 py-1 text-2.5 text-muted border-b border-border">Response</p>
			{#if document === null}
				<p class="p-2 text-2.5 text-muted">Fetch a sample to see it here.</p>
			{:else}
				<div class="flex-1 min-h-0">
					<Editor value={documentPreview} readonly scope="response" />
				</div>
			{/if}
		</div>

		<div class="flex flex-col rounded border border-border overflow-hidden">
			<p class="px-2 py-1 text-2.5 text-muted border-b border-border">
				Endpoints{preview.length ? ` (${preview.length})` : ''}
			</p>
			<div class="flex-1 overflow-y-auto min-h-0">
				{#if previewError}
					<p class="p-2 text-2.5 text-bad whitespace-pre-wrap">{previewError}</p>
				{:else if preview.length === 0}
					<p class="p-2 text-2.5 text-muted">
						{document === null ? 'Waiting for a sample.' : 'The filter produced nothing.'}
					</p>
				{:else}
					{#each preview as endpoint (endpoint.method + endpoint.path)}
						<div class="flex items-center gap-2 px-2 py-0.5">
							<span class="font-mono text-2.5 font-bold w-9 shrink-0 {methodColor(endpoint.method)}">
								{endpoint.method}
							</span>
							<span class="font-mono text-2.5 truncate">{endpoint.path}</span>
							<span class="ml-auto text-2.5 text-muted truncate max-w-32">{endpoint.name}</span>
						</div>
					{/each}
				{/if}
			</div>
		</div>
	</div>

	<div class="grid grid-cols-[2fr_1fr] gap-2">
		<label class="flex flex-col gap-1">
			<span class="text-xs text-muted">Next page (optional)</span>
			<input
				bind:value={section.loader.next}
				spellcheck="false"
				placeholder=".links.next"
				class="input-base text-xs font-mono selectable"
			/>
			<span class="text-2.5 text-muted">
				A filter yielding the next page's URL, or null when there are no more.
			</span>
		</label>

		<label class="flex flex-col gap-1">
			<span class="text-xs text-muted">Refresh after (s)</span>
			<input
				type="number"
				min="0"
				bind:value={section.loader.ttlSeconds}
				class="input-base text-xs font-mono"
			/>
			<span class="text-2.5 text-muted">
				0 only when asked. Otherwise refreshed in the background — at startup, and whenever
				you come back to the window — once the cache is older than this.
			</span>
		</label>
	</div>

	{#if runSummary}
		<p class="text-2.5 text-ok">{runSummary}</p>
	{/if}
{/if}
