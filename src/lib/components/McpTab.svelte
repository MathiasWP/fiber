<script lang="ts">
	import {
		mcpBinary,
		mcpClients,
		mcpInstall,
		mcpUninstall,
		type McpClient
	} from '$lib/api';
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

<div class="flex-1 overflow-y-auto min-h-0">
	<p class="px-4 pt-3 pb-2 text-2.5 text-muted leading-relaxed">
		Fiber is its own MCP server, so there is nothing to install — adding it to a client is one
		line in that client's config. Collections stay hidden until you share them under
		<span class="text-text">Section settings → MCP</span>.
	</p>

	{#if loading}
		<p class="px-4 py-2 text-xs text-muted">Reading client configs…</p>
	{:else}
		{#each clients as client (client.id)}
			<div class="px-4 py-2.5 border-b border-border/50">
				<div class="flex items-center gap-2">
					<span class="min-w-0 truncate text-xs {client.detected ? 'text-text' : 'text-muted'}">
						{client.name}
					</span>
					{#if client.state === 'installed'}
						<span class="i-lucide-circle-check text-3 text-ok shrink-0" title="Added"></span>
					{:else if client.state === 'outdated'}
						<span
							class="i-lucide-circle-alert text-3 text-warn shrink-0"
							title="Points at another copy of Fiber"
						></span>
					{:else if client.state === 'absent' && !client.detected}
						<!-- Not a problem, just worth saying before someone clicks Add on
						     an editor they don't have: the file would be created for a
						     client that never reads it. -->
						<span class="text-2.5 text-muted shrink-0">not found</span>
					{/if}

					{#if client.state === 'unreadable'}
						<button
							class="ml-auto shrink-0 px-2 py-0.5 rounded text-2.5 text-muted hover:bg-raised hover:text-text transition-colors"
							onclick={() => copy(client.id, snippet)}
						>
							{copied === client.id ? 'Copied' : 'Copy JSON'}
						</button>
					{:else}
						<button
							class="ml-auto shrink-0 px-2 py-0.5 rounded text-2.5 transition-colors disabled:opacity-40 {client.state ===
							'installed'
								? 'text-muted hover:bg-raised hover:text-text'
								: 'bg-accent/15 text-accent hover:bg-accent/25'}"
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
						Runs <span class="font-mono">{client.command}</span> today. Update points it at this
						copy.
					</p>
				{:else if client.state === 'unreadable' && client.message}
					<p class="mt-1 text-2.5 text-bad leading-relaxed">{client.message}</p>
				{/if}
			</div>
		{/each}

		<!--
			Every other client. The path is the part nobody can guess — the rest of
			the entry is two fields — so it gets a copy button of its own.
		-->
		<div class="px-4 py-3">
			<p class="text-2.5 text-muted leading-relaxed">
				Any other client takes the same two things in its own config file:
			</p>
			<pre
				class="mt-1.5 overflow-x-auto rounded border border-border bg-raised p-2 font-mono text-2.5 text-text">{snippet}</pre>
			<div class="mt-1.5 flex gap-2">
				<button
					class="px-2 py-0.5 rounded text-2.5 text-muted hover:bg-raised hover:text-text transition-colors"
					onclick={() => copy('snippet', snippet)}
				>
					{copied === 'snippet' ? 'Copied' : 'Copy JSON'}
				</button>
				<button
					class="px-2 py-0.5 rounded text-2.5 text-muted hover:bg-raised hover:text-text transition-colors"
					onclick={() => copy('binary', binary)}
				>
					{copied === 'binary' ? 'Copied' : 'Copy path'}
				</button>
			</div>
		</div>

		<div class="px-4 py-3 border-t border-border">
			<p class="text-2.5 text-muted leading-relaxed">
				Serving a collections repo from a server, or putting a proxy and an audit log in front of
				your agents? Fiber runs as a container under ToolHive. One command, credentials included
				— it needs ToolHive and a container runtime, so it belongs in a terminal rather than
				here.
			</p>
			<pre
				class="mt-1.5 overflow-x-auto rounded border border-border bg-raised p-2 font-mono text-2.5 text-text">{TOOLHIVE_COMMAND}</pre>
			<div class="mt-1.5 flex gap-2">
				<button
					class="px-2 py-0.5 rounded text-2.5 text-muted hover:bg-raised hover:text-text transition-colors"
					onclick={() => copy('toolhive', TOOLHIVE_COMMAND)}
				>
					{copied === 'toolhive' ? 'Copied' : 'Copy command'}
				</button>
				<button
					class="px-2 py-0.5 rounded text-2.5 text-muted hover:bg-raised hover:text-text transition-colors"
					onclick={() => openUrl(TOOLHIVE_GUIDE)}
				>
					Read the guide
				</button>
			</div>
		</div>
	{/if}
</div>

{#if error}
	<p class="px-4 py-2 text-2.5 text-bad border-t border-border shrink-0">{error}</p>
{/if}
