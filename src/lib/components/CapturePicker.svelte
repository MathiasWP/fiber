<script lang="ts">
	import { Dialog } from 'bits-ui';
	import { browserSnapshot, type CaptureKind, type Snapshot } from '$lib/api';

	interface Props {
		open: boolean;
		sectionId: string;
		onPick: (rule: { capture: CaptureKind; key: string; path: string }) => void;
		onClose: () => void;
	}

	let { open, sectionId, onPick, onClose }: Props = $props();

	let snapshot = $state<Snapshot | null>(null);
	let error = $state<string | null>(null);
	let loading = $state(false);

	interface Candidate {
		capture: CaptureKind;
		key: string;
		path: string;
		/** What the user reads: `sid` or `@@auth0spajs@@… › body.access_token`. */
		label: string;
		detail: string;
		preview: string;
		/** Higher sorts first. */
		score: number;
	}

	async function load() {
		loading = true;
		error = null;
		try {
			snapshot = await browserSnapshot(sectionId);
		} catch (failure) {
			error = String(failure);
			snapshot = null;
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		if (open) load();
		else snapshot = null;
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

	const candidates = $derived.by<Candidate[]>(() => {
		if (!snapshot) return [];
		const found: Candidate[] = [];

		for (const cookie of snapshot.cookies) {
			found.push({
				capture: 'cookie',
				key: cookie.name,
				path: '',
				label: cookie.name,
				detail: cookie.httpOnly ? `${cookie.domain} · HttpOnly` : cookie.domain,
				preview: cookie.value,
				// An HttpOnly cookie is almost always the session — and it's the
				// case nothing but this app can reach.
				score: scoreOf(cookie.name, cookie.value) + (cookie.httpOnly ? 4 : 0)
			});
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
					found.push({
						capture: 'localStorage',
						key: entry.key,
						path: leaf.path,
						label: leaf.path,
						detail: entry.key,
						preview: leaf.value,
						score: scoreOf(`${entry.key} ${leaf.path}`, leaf.value)
					});
				}
			} else {
				found.push({
					capture: 'localStorage',
					key: entry.key,
					path: '',
					label: entry.key,
					detail: 'localStorage',
					preview: entry.value,
					score: scoreOf(entry.key, entry.value)
				});
			}
		}

		return found.sort((a, b) => b.score - a.score);
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
		<Dialog.Overlay class="fixed inset-0 bg-black/50" />
		<Dialog.Content
			class="fixed left-1/2 top-1/2 w-[min(680px,94vw)] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-panel shadow-2xl flex flex-col max-h-[80vh]"
		>
			<div class="p-4 pb-3 border-b border-border">
				<Dialog.Title class="text-sm font-semibold">Pick your credential</Dialog.Title>
				<Dialog.Description class="mt-1 text-xs text-muted leading-relaxed">
					Everything the signed-in session is holding. Click the one that's your token — the
					likeliest candidates are first. HttpOnly cookies are included; the page's own
					JavaScript can't read those, but this app can.
				</Dialog.Description>
			</div>

			<div class="flex-1 overflow-y-auto min-h-0">
				{#if loading}
					<p class="p-4 text-xs text-muted flex items-center gap-2">
						<span class="i-lucide-loader-circle animate-spin"></span>
						Reading the sign-in window…
					</p>
				{:else if error}
					<div class="p-4">
						<p class="text-xs text-bad">{error}</p>
						<button class="btn-ghost text-xs mt-2 px-0" onclick={load}>Try again</button>
					</div>
				{:else if candidates.length === 0}
					<p class="p-4 text-xs text-muted leading-relaxed">
						Nothing found. Make sure you've finished signing in, then try again.
					</p>
				{:else}
					{#each candidates as candidate (candidate.capture + candidate.key + candidate.path)}
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
									class="font-mono text-2.5 px-1 rounded shrink-0 {candidate.capture === 'cookie'
										? 'bg-accent/15 text-accent'
										: 'bg-ok/15 text-ok'}"
								>
									{candidate.capture === 'cookie' ? 'cookie' : 'storage'}
								</span>
								<span class="font-mono text-xs truncate">{candidate.label}</span>
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
				<button class="btn-ghost text-xs ml-auto" onclick={onClose}>Cancel</button>
			</div>
		</Dialog.Content>
	</Dialog.Portal>
</Dialog.Root>
