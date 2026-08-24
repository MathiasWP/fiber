<script lang="ts">
	import { Dialog } from 'bits-ui';
	import { browserSnapshot, type CaptureKind, type Snapshot } from '$lib/api';
	import { identify } from '$lib/providers';
	import DotLoader from '$lib/components/DotLoader.svelte';

	interface Props {
		open: boolean;
		sectionId: string;
		onPick: (rule: { capture: CaptureKind; key: string; path: string }) => void;
		onClose: () => void;
	}

	let { open, sectionId, onPick, onClose }: Props = $props();

	/**
	 * Everything the signed-in session is holding. `$state.raw` because it is
	 * only ever replaced wholesale — a deep proxy would wrap every cookie and
	 * storage entry just to hold a snapshot we never edit.
	 */
	let snapshot = $state.raw<Snapshot | null>(null);
	let error = $state<string | null>(null);
	let loading = $state(false);
	let query = $state('');
	let loadToken = 0;

	interface Candidate {
		/**
		 * Unique per row. The rule itself isn't: two cookies can share a name on
		 * different domains, and a nested leaf can land on the same key/path pair
		 * as another entry. Keying an `{#each}` on the rule blows up on those.
		 */
		id: string;
		capture: CaptureKind;
		key: string;
		path: string;
		/** What the user reads: `sid` or `@@auth0spajs@@… › body.access_token`. */
		label: string;
		detail: string;
		preview: string;
		/** Set when the key matches a known provider's storage format. */
		provider: string | null;
		/** Higher sorts first. */
		score: number;
	}

	async function load() {
		const token = ++loadToken;
		loading = true;
		error = null;
		try {
			const next = await browserSnapshot(sectionId);
			if (token === loadToken) snapshot = next;
		} catch (failure) {
			if (token === loadToken) {
				error = String(failure);
				snapshot = null;
			}
		} finally {
			if (token === loadToken) loading = false;
		}
	}

	$effect(() => {
		if (open) {
			query = '';
			load();
		} else {
			loadToken += 1;
			snapshot = null;
			loading = false;
		}
	});

	/** Flattens a JSON value into leaf paths, so a nested token is one click. */
	function leaves(value: unknown, prefix: string, out: { path: string; value: string }[]) {
		if (out.length > 200) return out;
		if (value && typeof value === 'object') {
			for (const [key, nested] of Object.entries(value)) {
				leaves(nested, prefix ? `${prefix}.${key}` : key, out);
			}
		} else if (value !== null && value !== undefined) {
			out.push({ path: prefix, value: String(value) });
		}
		return out;
	}

	/**
	 * Guesses which entries are credentials so the likely one floats to the top.
	 * A JWT is unmistakable; otherwise go on naming and length.
	 */
	function scoreOf(name: string, value: string): number {
		let score = 0;
		if (value.startsWith('eyJ')) score += 10;
		if (/token|jwt|auth|session|access|credential|sid/i.test(name)) score += 5;
		if (value.length >= 20) score += 2;
		if (value.length >= 200) score += 1;
		if (/^(true|false|\d+)$/.test(value)) score -= 5;
		return score;
	}

	function candidate(
		capture: CaptureKind,
		key: string,
		path: string,
		label: string,
		detail: string,
		value: string,
		bonus = 0
	): Omit<Candidate, 'id'> {
		// A recognised provider outweighs every heuristic — if we know Auth0 keeps
		// its token here, that beats guessing from the shape of the string.
		const known = identify(capture, key, path);
		return {
			capture,
			key,
			path,
			label,
			detail,
			preview: value,
			provider: known?.provider ?? null,
			score: scoreOf(`${key} ${path}`, value) + bonus + (known?.weight ?? 0)
		};
	}

	const candidates = $derived.by<Candidate[]>(() => {
		if (!snapshot) return [];
		const found: Omit<Candidate, 'id'>[] = [];

		for (const cookie of snapshot.cookies) {
			found.push(
				candidate(
					'cookie',
					cookie.name,
					'',
					cookie.name,
					cookie.httpOnly ? `${cookie.domain} · HttpOnly` : cookie.domain,
					cookie.value,
					// An HttpOnly cookie is almost always the session — and it's the
					// case nothing but this app can reach.
					cookie.httpOnly ? 4 : 0
				)
			);
		}

		for (const entry of snapshot.localStorage) {
			let parsed: unknown;
			try {
				parsed = JSON.parse(entry.value);
			} catch {
				parsed = undefined;
			}

			if (parsed && typeof parsed === 'object') {
				for (const leaf of leaves(parsed, '', [])) {
					found.push(
						candidate(
							'localStorage',
							entry.key,
							leaf.path,
							leaf.path,
							entry.key,
							leaf.value
						)
					);
				}
			} else {
				found.push(
					candidate('localStorage', entry.key, '', entry.key, 'localStorage', entry.value)
				);
			}
		}

		for (const entry of snapshot.indexedDb) {
			const identity = `${entry.database}/${entry.store}/${entry.key}`;
			let parsed: unknown;
			try {
				parsed = JSON.parse(entry.value);
			} catch {
				parsed = undefined;
			}

			if (parsed && typeof parsed === 'object') {
				for (const leaf of leaves(parsed, '', [])) {
					found.push(
						candidate('indexedDb', identity, leaf.path, leaf.path, identity, leaf.value)
					);
				}
			} else {
				found.push(
					candidate('indexedDb', identity, '', entry.key, `${entry.database}/${entry.store}`, entry.value)
				);
			}
		}

		return found
			.map((entry, index) => ({ ...entry, id: String(index) }))
			.sort((a, b) => b.score - a.score);
	});

	const SOURCE_STYLE: Record<CaptureKind, string> = {
		cookie: 'bg-accent/15 text-accent',
		localStorage: 'bg-ok/15 text-ok',
		indexedDb: 'bg-bad/15 text-bad'
	};

	const SOURCE_LABEL: Record<CaptureKind, string> = {
		cookie: 'cookie',
		localStorage: 'storage',
		indexedDb: 'indexeddb'
	};

	/**
	 * Every whitespace-separated term must appear somewhere in the row. Plain
	 * substring rather than fuzzy: against a list containing base64 blobs, a
	 * subsequence match finds almost everything, which is worse than useless.
	 */
	const visible = $derived.by(() => {
		const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
		if (terms.length === 0) return candidates;
		return candidates.filter((candidate) => {
			const haystack =
				`${candidate.label} ${candidate.detail} ${candidate.preview}`.toLowerCase();
			return terms.every((term) => haystack.includes(term));
		});
	});

	function truncate(text: string, max = 72) {
		return text.length > max ? `${text.slice(0, max)}…` : text;
	}
</script>

<Dialog.Root
	{open}
	onOpenChange={(next) => {
		if (!next) onClose();
	}}
>
	<Dialog.Portal>
		<Dialog.Overlay class="dialog-scrim" />
		<Dialog.Content
			class="fixed left-1/2 top-1/2 z-60 w-[min(680px,94vw)] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-panel shadow-2xl flex flex-col max-h-[80vh]"
		>
			<div class="p-4 pb-3 border-b border-border">
				<Dialog.Title class="text-sm font-semibold">Pick your credential</Dialog.Title>
				<Dialog.Description class="mt-1 text-xs text-muted leading-relaxed">
					Everything the signed-in session is holding. Click the one that's your token — the
					likeliest candidates are first. HttpOnly cookies are included; the page's own
					JavaScript can't read those, but this app can.
				</Dialog.Description>

				<div class="relative mt-3">
					<span
						class="i-lucide-search absolute left-2 top-1/2 -translate-y-1/2 text-muted text-3"
					></span>
					<!-- svelte-ignore a11y_autofocus -->
					<input
						bind:value={query}
						autofocus
						spellcheck="false"
						placeholder="Filter by name, path or value…"
						class="input-base w-full text-xs pl-7"
					/>
				</div>
			</div>

			<div class="flex-1 overflow-y-auto min-h-0">
				{#if loading}
					<p class="p-4 text-xs text-muted flex items-center gap-2">
						<DotLoader size={16} class="text-text" />
						Reading the sign-in window…
					</p>
				{:else if error}
					<div class="p-4">
						<p class="text-xs text-bad">{error}</p>
						<button class="btn-ghost text-xs mt-2 px-0" onclick={load}>Try again</button>
					</div>
				{:else if candidates.length === 0}
					<div class="p-4 text-xs text-muted leading-relaxed flex flex-col gap-2">
						<p>Nothing found. Make sure you've finished signing in, then Refresh.</p>
						<p class="text-2.5">
							Cookies, localStorage and IndexedDB are all read. One thing still isn't
							recoverable: MSAL v4 encrypts its storage unless “keep me signed in” was ticked.
							If that's your setup, a cookie is usually still capturable.
						</p>
					</div>
				{:else if visible.length === 0}
					<p class="p-4 text-xs text-muted">
						Nothing matches “{query}” — {candidates.length} values in this session.
					</p>
				{:else}
					{#each visible as candidate (candidate.id)}
						<button
							class="w-full text-left px-4 py-2 border-b border-border/50 hover:bg-raised transition-colors"
							onclick={() =>
								onPick({
									capture: candidate.capture,
									key: candidate.key,
									path: candidate.path
								})}
						>
							<div class="flex items-center gap-2">
								<span
									class="font-mono text-2.5 px-1 rounded shrink-0 {SOURCE_STYLE[candidate.capture]}"
								>
									{SOURCE_LABEL[candidate.capture]}
								</span>
								<span class="font-mono text-xs truncate">{candidate.label}</span>
								{#if candidate.provider}
									<span
										class="font-medium text-2.5 px-1 rounded shrink-0 bg-warn/15 text-warn"
										title="Matches {candidate.provider}'s documented storage format"
									>
										{candidate.provider}
									</span>
								{/if}
								<span class="ml-auto text-2.5 text-muted truncate max-w-48">
									{candidate.detail}
								</span>
							</div>
							<div class="mt-0.5 font-mono text-2.5 text-muted truncate">
								{truncate(candidate.preview)}
							</div>
						</button>
					{/each}
				{/if}
			</div>

			<div class="flex items-center gap-2 p-3 border-t border-border">
				<button class="btn-ghost text-xs" onclick={load} disabled={loading}>Refresh</button>
				{#if candidates.length}
					<span class="text-2.5 text-muted">
						{visible.length === candidates.length
							? `${candidates.length} values`
							: `${visible.length} of ${candidates.length}`}
					</span>
				{/if}
				<button class="btn-ghost text-xs ml-auto" onclick={onClose}>Cancel</button>
			</div>
		</Dialog.Content>
	</Dialog.Portal>
</Dialog.Root>
