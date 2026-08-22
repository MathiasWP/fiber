<script lang="ts">
	import {
		endpointKey,
		methodColor,
		parseOpenApi,
		splitQuery,
		withQuery,
		type Import,
		type Section
	} from '$lib/api';
	import { collections } from '$lib/collections.svelte';

	interface Props {
		section: Section;
	}

	let { section }: Props = $props();

	/** The parsed spec, replaced wholesale once the file is read. */
	let parsed = $state.raw<Import | null>(null);
	let error = $state<string | null>(null);
	let fileName = $state('');
	let done = $state<string | null>(null);

	/** What's already here, so the preview can say what it would actually add. */
	const existing = $derived(
		new Set(
			section.requests.map((request) =>
				endpointKey(request.method, splitQuery(request.path).base)
			)
		)
	);
	/**
	 * Operations are identified by method and path. A malformed or merged spec
	 * can repeat one; importing it twice would create two indistinguishable
	 * sidebar rows even though neither existed before the import.
	 */
	const fresh = $derived.by(() => {
		const seen = new Set(existing);
		return (
			parsed?.endpoints.filter((endpoint) => {
				const key = endpointKey(endpoint.method, endpoint.path);
				if (seen.has(key)) return false;
				seen.add(key);
				return true;
			}) ?? []
		);
	});

	let pickToken = 0;

	async function pick(event: Event) {
		const input = event.target as HTMLInputElement;
		const file = input.files?.[0];
		if (!file) return;
		const token = ++pickToken;

		fileName = file.name;
		error = null;
		done = null;
		parsed = null;

		try {
			const result = await parseOpenApi(await file.text());
			if (token === pickToken) parsed = result;
		} catch (failure) {
			if (token === pickToken) error = String(failure);
		}
		// Let the same file be chosen again after a failed parse.
		input.value = '';
	}

	async function apply() {
		if (!parsed) return;

		const adding = [...fresh];
		const previousBaseUrl = section.baseUrl;
		const addedIds: string[] = [];
		for (const endpoint of adding) {
			const query = (endpoint.parameters ?? [])
				.filter((param) => param.in === 'query')
				.map((param) => ({ name: param.name, value: param.example || '' }));
			const path = query.some((param) => param.name)
				? withQuery(endpoint.path, query)
				: endpoint.path;
			const id = crypto.randomUUID();
			addedIds.push(id);
			section.requests.push({
				id,
				name: endpoint.name || endpoint.path,
				method: endpoint.method,
				path,
				description: endpoint.description || '',
				tag: endpoint.tag || '',
				body: endpoint.body,
				bodyKind: endpoint.bodyKind ?? 'json',
				form: endpoint.form?.map((field) => ({ ...field })) ?? [],
				file: '',
				pathParams: (endpoint.parameters ?? [])
					.filter((param) => param.in === 'path')
					.map((param) => ({ name: param.name, value: param.example || '' })),
				headers: []
			});
		}
		// Only fill the base URL in when there's nothing to overwrite.
		if (!section.baseUrl.trim() && parsed.baseUrl) section.baseUrl = parsed.baseUrl;

		const added = adding.length;
		if (!(await collections.flush(section))) {
			const ids = new Set(addedIds);
			section.requests = section.requests.filter((request) => !ids.has(request.id));
			section.baseUrl = previousBaseUrl;
			error = collections.error ?? 'The imported endpoints could not be saved.';
			return;
		}
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
				{#each parsed.endpoints as endpoint, index (endpoint.method + endpoint.path + '\0' + index)}
					{@const isNew = fresh.includes(endpoint)}
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
