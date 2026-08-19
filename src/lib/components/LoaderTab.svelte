<script lang="ts">
	import { Select } from 'bits-ui';
	import {
		defaultLoader,
		loaderExamples,
		loaderPreview,
		loaderProbe,
		methodColor,
		type LoadedEndpoint,
		type Section
	} from '$lib/api';
	import { collections } from '$lib/collections.svelte';
	import Editor from './Editor.svelte';

	interface Props {
		section: Section;
	}

	let { section }: Props = $props();

	/** The manifest as fetched. Held so the filter can be re-run without refetching. */
	let document = $state<unknown>(null);
	let probing = $state(false);
	let probeError = $state<string | null>(null);

	let preview = $state<LoadedEndpoint[]>([]);
	let previewError = $state<string | null>(null);

	let examples = $state<[string, string][]>([]);
	let runSummary = $state<string | null>(null);

	const running = $derived(collections.loading[section.id] === true);
	const cached = $derived(collections.loaderCaches[section.id]);

	$effect(() => {
		loaderExamples().then((found) => (examples = found));
	});

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

	// Re-map on every keystroke. This is only possible because a filter is a
	// pure transformation — nothing is refetched and nothing can escape.
	$effect(() => {
		const query = section.loader?.query ?? '';
		const input = document;
		if (input === null || !query.trim()) {
			preview = [];
			previewError = null;
			return;
		}

		let current = true;
		loaderPreview(input, query)
			.then((endpoints) => {
				if (!current) return;
				preview = endpoints;
				previewError = null;
			})
			.catch((error) => {
				if (!current) return;
				preview = [];
				previewError = String(error);
			});
		return () => {
			current = false;
		};
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

	const documentPreview = $derived(
		document === null ? '' : JSON.stringify(document, null, 2).slice(0, 20000)
	);
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
				<span class="i-lucide-loader-circle animate-spin"></span>
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
				spellcheck="false"
				placeholder="/internal/endpoints"
				class="input-base text-xs font-mono selectable"
			/>
		</label>
	</div>

	<div class="flex items-center gap-2">
		<button class="btn-ghost text-xs" disabled={probing} onclick={probe}>
			{#if probing}
				<span class="i-lucide-loader-circle animate-spin"></span>
			{/if}
			Fetch a sample
		</button>
		<span class="text-2.5 text-muted">
			{document === null
				? 'Fetch once, then edit the filter against it.'
				: 'Filter runs as you type — nothing is refetched.'}
		</span>

		{#if examples.length}
			<Select.Root
				type="single"
				value=""
				onValueChange={(next) => {
					if (next && section.loader) section.loader.query = next;
				}}
			>
				<Select.Trigger class="btn-ghost text-xs ml-auto">Examples</Select.Trigger>
				<Select.Portal>
					<Select.Content class="menu-content w-72">
						<Select.Viewport>
							{#each examples as [name, query] (name)}
								<Select.Item value={query} label={name} class="menu-item">
									{name}
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
			<Editor bind:value={section.loader.query} language="text" />
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
					<Editor value={documentPreview} readonly />
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

	{#if runSummary}
		<p class="text-2.5 text-ok">{runSummary}</p>
	{/if}
{/if}
