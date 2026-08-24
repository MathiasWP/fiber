<script lang="ts">
	/**
	 * Says so when the frontend falls over.
	 *
	 * A Svelte app that throws — an effect loop, a bad read during render — stops
	 * updating but leaves its last paint on screen. Styled, complete, and inert:
	 * the app looks frozen, and nothing anywhere records why. That has cost two
	 * rounds of guesswork already.
	 *
	 * So this listens for what would otherwise vanish and puts it on screen, along
	 * with the version, so a report can start from the actual error.
	 *
	 * Deliberately plain: inline styles, no runes in the handlers, no dependency on
	 * anything that might itself be broken by the time it runs.
	 */
	import { getVersion } from '@tauri-apps/api/app';

	let message = $state<string | null>(null);
	let detail = $state('');
	let version = $state('');
	getVersion().then((value) => (version = value));

	function record(what: string, error: unknown, source: string) {
		// First one wins. A loop would otherwise overwrite the original cause with
		// its own aftermath.
		if (message) return;
		message = what;
		const stack = error instanceof Error ? (error.stack ?? error.message) : String(error);
		detail = `${source}\n${stack}`;
		// Also to the console, for whoever has devtools open.
		console.error('[fiber]', what, error);
	}

	// `svelte:window` types these as plain Events, so narrow here rather than
	// claim a shape the compiler can't see.
	function onError(event: Event) {
		const failure = event as ErrorEvent;
		record(
			failure.message || 'The interface hit an error',
			failure.error,
			`${failure.filename ?? 'unknown'}:${failure.lineno ?? 0}`
		);
	}

	function onRejection(event: Event) {
		const rejection = event as PromiseRejectionEvent;
		record('A background task failed', rejection.reason, 'unhandled promise rejection');
	}

	/**
	 * Keeps a click on Copy or Hide from reaching the dialog underneath.
	 *
	 * Bits UI watches `document` for a pointerdown outside the open dialog and
	 * treats it as "dismiss me". This banner is outside every dialog, so without
	 * this the first click closes the topmost one instead of pressing a button —
	 * and with dialogs stacked, the banner stays unusable until they're all gone.
	 */
	function keepClick(event: PointerEvent) {
		event.stopPropagation();
	}

	async function copy() {
		await navigator.clipboard.writeText(`Fiber ${version}\n${message}\n\n${detail}`);
	}

	interface FiberTestHooks {
		crash?: (message: string) => void;
		reject?: (message: string) => void;
	}

	// E2E: Chromium does not deliver synthetic `error` / `unhandledrejection`
	// events to `<svelte:window onerror>` the way a real throw does. Register
	// synchronously so a test that runs right after `goto` can already call it.
	const crashHooks = (window as unknown as { __FIBER_TEST__?: FiberTestHooks }).__FIBER_TEST__;
	if (crashHooks) {
		crashHooks.crash = (text) => record(text, new Error(text), 'test');
		crashHooks.reject = (text) =>
			record('A background task failed', new Error(text), 'unhandled promise rejection');
	}
</script>

<svelte:window onerror={onError} onunhandledrejection={onRejection} />

{#if message}
	<!--
		`pointer-events-auto` because a modal dialog sets `pointer-events: none` on
		`<body>`, which this inherits: the click would pass straight through to
		whatever is behind it. Above the z-index, which is already the highest one.
	-->
	<div
		class="pointer-events-auto fixed inset-x-0 bottom-0 z-200 border-t border-bad bg-panel p-3 shadow-2xl"
		role="alert"
		onpointerdown={keepClick}
	>
		<div class="flex items-start gap-2">
			<span class="i-lucide-circle-alert mt-0.5 shrink-0 text-3.5 text-bad"></span>
			<div class="min-w-0 flex-1">
				<p class="text-xs font-medium text-text">
					{message}
				</p>
				<p class="mt-0.5 text-2.5 text-muted">
					Fiber {version} — the window may stop responding. Reopening the app is safe; nothing
					is lost.
				</p>
				<pre
					class="mt-2 max-h-32 overflow-auto whitespace-pre-wrap break-all rounded bg-raised p-2 font-mono text-2.5 text-muted selectable">{detail}</pre>
			</div>
			<div class="flex shrink-0 flex-col gap-1">
				<button class="btn-ghost text-xs" onclick={copy}>Copy</button>
				<button class="btn-ghost text-xs" onclick={() => (message = null)}>Hide</button>
			</div>
		</div>
	</div>
{/if}
