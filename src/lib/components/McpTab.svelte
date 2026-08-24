<script lang="ts">
	import { mcpBinary, mcpClients, mcpInstall, mcpUninstall, type McpClient } from '$lib/api';
	import { openUrl } from '@tauri-apps/plugin-opener';

	/**
	 * The MCP tab: one row per AI client, and a button that writes Fiber into
	 * its config.
	 *
	 * The whole point is that there is nothing to install — `fiber mcp` is this
	 * binary with a different first argument — so what a client needs is one
	 * entry in one file. Doing that by hand means finding the file, getting the
	 * JSON right and knowing where the app lives; the button is the same edit
	 * without any of that.
	 *
	 * Configs are read when the tab opens rather than at startup: a session that
	 * never comes here never touches another program's files.
	 */

	let clients = $state.raw<McpClient[]>([]);
	let binary = $state('');
	let loading = $state(true);
	let error = $state<string | null>(null);
	/** The client currently being written, so only its own button spins. */
	let busy = $state<string | null>(null);
	/** What was last copied, so the button can say so. */
	let copied = $state<string | null>(null);
	let copiedTimer: ReturnType<typeof setTimeout> | undefined;
	/** Whether the clients Fiber couldn't find are being shown as well. */
	let showMissing = $state(false);

	async function load() {
		try {
			const [list, path] = await Promise.all([mcpClients(), mcpBinary()]);
			clients = list;
			binary = path;
			error = null;
		} catch (err) {
			error = String(err);
		} finally {
			loading = false;
		}
	}
	load();

	/**
	 * Only what is actually on this machine. Seven rows, five of them for
	 * editors you don't have, is a list you have to read rather than scan.
	 *
	 * Detection is a heuristic — a config file or the directory that holds it —
	 * so the rest stay one click away rather than gone. A client already holding
	 * an entry is always shown, whatever the guess says: something wrote it.
	 */
	const shown = $derived(
		clients.filter((client) => showMissing || client.detected || client.state !== 'absent')
	);
	const missing = $derived(clients.length - shown.length);

	/** The install already returns the client's new state, so nothing re-reads. */
	function replace(updated: McpClient) {
		clients = clients.map((client) => (client.id === updated.id ? updated : client));
	}

	async function toggle(client: McpClient) {
		busy = client.id;
		error = null;
		try {
			replace(
				client.state === 'installed' ? await mcpUninstall(client.id) : await mcpInstall(client.id)
			);
		} catch (err) {
			error = String(err);
		} finally {
			busy = null;
		}
	}

	/**
	 * The same entry, for a client not on the list. Written in the shape almost
	 * everything but VS Code takes, which is the one worth defaulting to.
	 *
	 * Laid out by hand rather than by `JSON.stringify`, which puts a one-element
	 * array on four lines — in a pane this narrow that is most of the snippet.
	 * `JSON.stringify` still does the escaping, which is the part worth not
	 * getting wrong on a Windows path.
	 */
	const snippet = $derived(
		`{
  "mcpServers": {
    "fiber": {
      "command": ${JSON.stringify(binary)},
      "args": ["mcp"]
    }
  }
}`
	);

	/**
	 * The other way to run the server: as a container, for collections that live
	 * in a repo rather than on this laptop. It isn't a client and there is no
	 * file to edit, so it sits under the list rather than in it — and it stays a
	 * command, because it needs ToolHive and a container runtime that a button
	 * here can't conjure.
	 */
	const TOOLHIVE_COMMAND =
		'curl -fsSL https://raw.githubusercontent.com/MathiasWP/fiber/main/scripts/toolhive.sh | bash';
	const TOOLHIVE_GUIDE = 'https://github.com/MathiasWP/fiber/blob/main/deploy/toolhive.md';

	function copy(what: string, text: string) {
		navigator.clipboard.writeText(text);
		copied = what;
		clearTimeout(copiedTimer);
		copiedTimer = setTimeout(() => (copied = null), 1500);
	}

	/** The right-hand button's word, which is also its whole explanation. */
	function action(client: McpClient): string {
		if (client.state === 'installed') return 'Remove';
		if (client.state === 'outdated') return 'Update';
		return 'Add';
	}
</script>

<!--
	Three parts, each under its own heading: the clients on this machine, the
	entry to paste anywhere else, and the container route. Without the headings
	it reads as one list that stops making sense two thirds of the way down.
-->
{#snippet heading(text: string)}
	<h3
		class="px-4 pt-4 pb-1.5 text-2.5 font-semibold uppercase tracking-wider text-muted select-none"
	>
		{text}
	</h3>
{/snippet}

<!--
	A copy button is a control, so it looks like one whether or not the pointer
	is over it. The first version was bare text under the block, which read as a
	caption until you hovered it and a background appeared.
-->
{#snippet copyButton(what: string, text: string, label: string)}
	<button
		class="shrink-0 inline-flex items-center gap-1 rounded border border-border px-1.5 py-0.5 text-2.5 text-muted transition-colors hover:bg-raised hover:text-text"
		onclick={() => copy(what, text)}
	>
		<span class="{copied === what ? 'i-lucide-check' : 'i-lucide-copy'} text-3"></span>
		{copied === what ? 'Copied' : label}
	</button>
{/snippet}

<div class="flex-1 overflow-y-auto min-h-0">
	<p class="px-4 pt-3 text-2.5 text-muted leading-relaxed">
		Fiber is its own MCP server, so there is nothing to install — adding it to a client is one
		line in that client's config. Collections stay hidden until you share them under
		<span class="text-text">Section settings → MCP</span>.
	</p>

	{#if loading}
		<p class="px-4 py-3 text-xs text-muted">Reading client configs…</p>
	{:else}
		{@render heading('Clients')}

		{#each shown as client (client.id)}
			<div class="px-4 py-2 border-t border-border/50">
				<div class="flex items-center gap-1.5">
					<span class="min-w-0 truncate text-xs text-text">{client.name}</span>
					{#if client.state === 'installed'}
						<span class="i-lucide-circle-check text-3 text-ok shrink-0" title="Added"></span>
					{:else if client.state === 'outdated'}
						<span
							class="i-lucide-circle-alert text-3 text-warn shrink-0"
							title="Points at another copy of Fiber"
						></span>
					{:else if !client.detected}
						<!-- Only reachable through "show the rest": worth saying that adding
						     this one writes a config for a client that may never read it. -->
						<span class="shrink-0 text-2.5 text-muted">not found</span>
					{/if}

					{#if client.state === 'unreadable'}
						{@render copyButton(client.id, snippet, 'Copy entry')}
					{:else}
						<button
							class="ml-auto shrink-0 rounded px-2 py-0.5 text-2.5 font-medium transition-colors disabled:opacity-40 {client.state ===
							'installed'
								? 'border border-border text-muted hover:bg-raised hover:text-text'
								: 'bg-accent text-white hover:bg-accent/85'}"
							disabled={busy !== null}
							onclick={() => toggle(client)}
						>
							{busy === client.id ? '…' : action(client)}
						</button>
					{/if}
				</div>

				<p class="mt-0.5 font-mono text-2.5 text-muted truncate" title={client.path}>
					{client.path}
				</p>

				{#if client.state === 'outdated' && client.command}
					<p class="mt-1 text-2.5 text-warn leading-relaxed">
						Runs <span class="font-mono break-all">{client.command}</span> today. Update points it
						at this copy.
					</p>
				{:else if client.state === 'unreadable' && client.message}
					<p class="mt-1 text-2.5 text-bad leading-relaxed">{client.message}</p>
				{/if}
			</div>
		{:else}
			<p class="px-4 py-2 text-2.5 text-muted leading-relaxed border-t border-border/50">
				None of the clients Fiber knows about are on this machine.
			</p>
		{/each}

		{#if missing > 0 || showMissing}
			<button
				class="w-full border-t border-border/50 px-4 py-2 text-left text-2.5 text-muted transition-colors hover:bg-raised hover:text-text"
				onclick={() => (showMissing = !showMissing)}
			>
				{showMissing
					? 'Hide the ones that were not found'
					: `Show ${missing} more ${missing === 1 ? 'client' : 'clients'} Fiber didn't find`}
			</button>
		{/if}

		{@render heading('Any other client')}
		<div class="px-4 pb-1">
			<div class="flex items-center justify-between gap-2">
				<p class="min-w-0 text-2.5 text-muted">The same entry, in its own config file</p>
				{@render copyButton('snippet', snippet, 'Copy entry')}
			</div>
			<!--
				Wrapped rather than scrolled. A binary path is longer than this pane
				is wide, and a horizontal scrollbar inside a 200px column is worse
				than a wrapped line.
			-->
			<pre
				class="mt-1.5 whitespace-pre-wrap break-all rounded border border-border bg-raised p-2 font-mono text-2.5 text-text">{snippet}</pre>

			<div class="mt-2 flex items-center justify-between gap-2">
				<p class="min-w-0 text-2.5 text-muted">Or just the path to this binary</p>
				{@render copyButton('binary', binary, 'Copy path')}
			</div>
			<p class="mt-1.5 break-all font-mono text-2.5 text-muted">{binary}</p>
		</div>

		{@render heading('On a server')}
		<div class="px-4 pb-4">
			<p class="text-2.5 text-muted leading-relaxed">
				For collections that live in a repo rather than on this machine, or to put a proxy and an
				audit log in front of your agents, Fiber runs as a container under ToolHive. One command,
				credentials included — it needs ToolHive and a container runtime, so it belongs in a
				terminal rather than here.
			</p>

			<div class="mt-2 flex items-center justify-between gap-2">
				<p class="min-w-0 text-2.5 text-muted">Sets everything up</p>
				{@render copyButton('toolhive', TOOLHIVE_COMMAND, 'Copy command')}
			</div>
			<pre
				class="mt-1.5 whitespace-pre-wrap break-all rounded border border-border bg-raised p-2 font-mono text-2.5 text-text">{TOOLHIVE_COMMAND}</pre>

			<button
				class="mt-2 inline-flex items-center gap-1 text-2.5 text-accent transition-colors hover:underline"
				onclick={() => openUrl(TOOLHIVE_GUIDE)}
			>
				Read the guide
				<span class="i-lucide-external-link text-3"></span>
			</button>
		</div>
	{/if}
</div>

{#if error}
	<p class="px-4 py-2 text-2.5 text-bad border-t border-border shrink-0">{error}</p>
{/if}
