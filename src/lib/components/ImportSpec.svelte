<script lang="ts">
	import { endpointKey, methodColor, parseOpenApi, type Import, type Section } from '$lib/api';
	import { collections } from '$lib/collections.svelte';

	interface Props {
		section: Section;
	}

	let { section }: Props = $props();

	let parsed = $state<Import | null>(null);
	let error = $state<string | null>(null);
	let fileName = $state('');
	let done = $state<string | null>(null);

	/** What's already here, so the preview can say what it would actually add. */
	const existing = $derived(
		new Set(section.requests.map((request) => endpointKey(request.method, request.path)))
	);
	const fresh = $derived(
		parsed?.endpoints.filter(
			(endpoint) => !existing.has(endpointKey(endpoint.method, endpoint.path))
		) ?? []
	);

	async function pick(event: Event) {
		const input = event.target as HTMLInputElement;
		const file = input.files?.[0];
		if (!file) return;

		fileName = file.name;
		error = null;
		done = null;
		parsed = null;

		try {
			parsed = await parseOpenApi(await file.text());
		} catch (failure) {
			error = String(failure);
		}
		// Let the same file be chosen again after a failed parse.
		input.value = '';
	}

	async function apply() {
		if (!parsed) return;

		for (const endpoint of fresh) {
			section.requests.push({
				id: crypto.randomUUID(),
				name: endpoint.name || endpoint.path,
				method: endpoint.method,
				path: endpoint.path,
				body: '',
				headers: []
			});
		}
		// Only fill the base URL in when there's nothing to overwrite.
		if (!section.baseUrl.trim() && parsed.baseUrl) section.baseUrl = parsed.baseUrl;

		const added = fresh.length;
		await collections.flush(section);
		done = `Added ${added} endpoint${added === 1 ? '' : 's'}.`;
		parsed = null;
	}
</script>

<div class="rounded border border-border p-3 flex flex-col gap-2">
	<div class="flex items-center gap-2">
		<span class="i-lucide-file-down text-3 text-muted"></span>
		<span class="text-xs font-medium">Import OpenAPI</span>
	</div>

	<p class="text-2.5 text-muted leading-relaxed">
		Reads an OpenAPI or Swagger file — JSON or YAML — and adds its operations as ordinary
		requests. Unlike a loader, nothing is fetched and nothing goes stale, so an imported
		collection works offline.
	</p>

	<!-- A label is not a button as far as the browser is concerned, so the cursor
     has to be said out loud here to match everything else. -->
<label class="btn-ghost text-xs self-start cursor-default">
		<span class="i-lucide-folder-open"></span>
		Choose a file…
		<input
			type="file"
			accept=".json,.yaml,.yml,application/json,text/yaml"
			class="hidden"
			onchange={pick}
		/>
	</label>

	{#if error}
		<p class="text-2.5 text-bad">{error}</p>
	{/if}

	{#if done}
		<p class="text-2.5 text-ok">{done}</p>
	{/if}

	{#if parsed}
		<div class="rounded border border-border">
			<div class="flex items-center gap-2 px-2 py-1 border-b border-border">
				<span class="text-2.5 truncate">
					{parsed.title || fileName}{parsed.version ? ` · ${parsed.version}` : ''}
				</span>
				<span class="ml-auto text-2.5 text-muted shrink-0">
					{fresh.length} new of {parsed.endpoints.length}
				</span>
			</div>

			<div class="max-h-40 overflow-y-auto">
				{#each parsed.endpoints as endpoint (endpoint.method + endpoint.path)}
					{@const isNew = !existing.has(endpointKey(endpoint.method, endpoint.path))}
					<div class="flex items-center gap-2 px-2 py-0.5 {isNew ? '' : 'opacity-40'}">
						<span class="font-mono text-2.5 font-bold w-9 shrink-0 {methodColor(endpoint.method)}">
							{endpoint.method}
						</span>
						<span class="font-mono text-2.5 truncate">{endpoint.path}</span>
						{#if !isNew}
							<span class="ml-auto text-2.5 text-muted shrink-0">already here</span>
						{/if}
					</div>
				{/each}
			</div>
		</div>

		{#if parsed.baseUrl && !section.baseUrl.trim()}
			<p class="text-2.5 text-muted">
				Base URL will be set to <span class="font-mono">{parsed.baseUrl}</span>.
			</p>
		{/if}

		<div class="flex items-center gap-2">
			<button class="btn-primary text-xs" disabled={fresh.length === 0} onclick={apply}>
				Add {fresh.length} endpoint{fresh.length === 1 ? '' : 's'}
			</button>
			<button class="btn-ghost text-xs" onclick={() => (parsed = null)}>Cancel</button>
		</div>
	{/if}
</div>
