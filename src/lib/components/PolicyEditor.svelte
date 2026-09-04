<script lang="ts">
	import { Select } from 'bits-ui';
	import {
		methodColor,
		policyPreview,
		policyTemplates,
		type Access,
		type PolicyRow,
		type Section
	} from '$lib/api';
	import { collections } from '$lib/collections.svelte';
	import Editor from './Editor.svelte';

	interface Props {
		section: Section;
	}

	let { section }: Props = $props();

	let templates = $state.raw<[string, string][]>([]);
	/** Replaced wholesale as the policy re-runs; never edited in place. */
	let rows = $state.raw<PolicyRow[]>([]);
	let warning = $state<string | null>(null);

	const policy = $derived(section.mcp.policy ?? '');
	/** The template the policy still matches, or nothing once it is edited. */
	const current = $derived(templates.find(([, filter]) => filter === policy)?.[0] ?? null);

	const counts = $derived({
		allow: rows.filter((row) => row.access === 'allow').length,
		ask: rows.filter((row) => row.access === 'ask').length,
		deny: rows.filter((row) => row.access === 'deny').length
	});

	policyTemplates().then((found) => (templates = found));

	const COLOR: Record<Access, string> = {
		allow: 'text-muted',
		ask: 'text-warn',
		deny: 'text-bad'
	};

	async function addPolicy() {
		// The first template is read-only, which is what the collection already
		// does — so adding a policy changes nothing until it is edited, and the
		// preview below shows that rather than a surprise.
		section.mcp.policy = templates[0]?.[1] ?? '"deny"';
		await collections.flush(section);
	}

	async function removePolicy() {
		section.mcp.policy = '';
		rows = [];
		warning = null;
		await collections.flush(section);
	}

	/**
	 * Re-run as you type, against this collection's real endpoints. Possible
	 * because a policy is a pure filter over data already on disk — nothing is
	 * fetched and nothing is sent — but it still crosses the IPC boundary, so it
	 * waits for a pause in the typing. The token guards against out-of-order
	 * replies: a slow run of an old policy must not land on top of a fast run of
	 * the current one.
	 */
	let previewToken = 0;
	$effect(() => {
		const filter = policy;
		const id = section.id;
		const token = ++previewToken;
		if (!filter.trim()) {
			rows = [];
			warning = null;
			return;
		}

		const timer = setTimeout(() => {
			policyPreview(id, filter)
				.then((preview) => {
					if (token !== previewToken) return;
					rows = preview.items;
					warning = preview.warning ?? null;
				})
				.catch((error) => {
					if (token !== previewToken) return;
					rows = [];
					warning = String(error);
				});
		}, 150);
		return () => clearTimeout(timer);
	});
</script>

{#if !policy}
	<p class="text-2.5 text-muted leading-relaxed">
		An access policy decides per endpoint instead of by HTTP method — for an API where the method
		says nothing, because every call is a POST. It is a
		<a href="https://jqlang.org/manual/" target="_blank" rel="noreferrer" class="text-accent">
			jq filter
		</a>
		answering <span class="font-mono">"allow"</span>, <span class="font-mono">"ask"</span> or
		<span class="font-mono">"deny"</span> for each one, reading whatever your manifest publishes
		about it.
	</p>
	<div>
		<button class="btn-ghost text-xs" onclick={addPolicy}>
			<span class="i-lucide-plus"></span>
			Add an access policy
		</button>
	</div>
{:else}
	<div class="flex items-center gap-2">
		<span class="text-xs text-muted">Access policy</span>

		{#if templates.length}
			<Select.Root
				type="single"
				value={policy}
				onValueChange={(next) => {
					if (next) section.mcp.policy = next;
				}}
			>
				<Select.Trigger class="btn-ghost text-xs ml-auto">
					{current ?? 'Custom'}
					<span class="i-lucide-chevron-down text-3"></span>
				</Select.Trigger>
				<Select.Portal>
					<Select.Content class="menu-content w-72">
						<Select.Viewport>
							{#each templates as [name, filter] (name)}
								<Select.Item value={filter} label={name} class="menu-item">
									{#snippet children({ selected })}
										<span class="i-lucide-check text-3 shrink-0 {selected ? '' : 'opacity-0'}"></span>
										{name}
									{/snippet}
								</Select.Item>
							{/each}
						</Select.Viewport>
					</Select.Content>
				</Select.Portal>
			</Select.Root>
		{/if}
		<button class="btn-ghost text-xs {templates.length ? '' : 'ml-auto'}" onclick={removePolicy}>
			Remove
		</button>
	</div>

	<div class="h-24 rounded border border-border overflow-hidden">
		<Editor bind:value={section.mcp.policy} language="text" scope="request" />
	</div>

	<!-- The endpoints it was just run against, so a policy is read as counts
	     and rows rather than as a guess about what it does. -->
	<div class="flex flex-col rounded border border-border overflow-hidden">
		<p class="px-2 py-1 text-2.5 text-muted border-b border-border flex items-center gap-2">
			<span>This collection's endpoints</span>
			{#if rows.length}
				<span class="ml-auto flex items-center gap-2">
					<span class={COLOR.allow}>{counts.allow} allow</span>
					<span class={COLOR.ask}>{counts.ask} ask</span>
					<span class={COLOR.deny}>{counts.deny} deny</span>
				</span>
			{/if}
		</p>
		<div class="max-h-40 overflow-y-auto">
			{#if warning}
				<p class="p-2 text-2.5 text-bad whitespace-pre-wrap">{warning}</p>
			{:else if rows.length === 0}
				<p class="p-2 text-2.5 text-muted">
					No endpoints yet — type some, import a spec, or run a loader.
				</p>
			{:else}
				{#each rows as row (row.method + row.path + row.name)}
					<div class="flex items-center gap-2 px-2 py-0.5">
						<span class="font-mono text-2.5 font-bold w-9 shrink-0 {methodColor(row.method)}">
							{row.method}
						</span>
						<span class="font-mono text-2.5 truncate">{row.path}</span>
						<span class="ml-auto text-2.5 shrink-0 {COLOR[row.access]}">{row.access}</span>
					</div>
				{/each}
			{/if}
		</div>
	</div>

	<p class="text-2.5 text-muted leading-relaxed">
		The policy decides everything while it is set, GET included — a read that returns your whole
		customer table can say so. Anything it cannot answer for, including a path the collection
		doesn't list, is denied. <span class="font-mono">"ask"</span> puts the call in front of you in
		the agent's own client, and only sending it once you say so.
	</p>
{/if}
