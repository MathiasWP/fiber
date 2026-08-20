<script lang="ts">
	// UnoCSS utilities are generated into app.css by @unocss/postcss.
	import '../app.css';
	import UpdateToast from '$lib/components/UpdateToast.svelte';
	import { updates } from '$lib/update.svelte';

	let { children } = $props();

	// Here rather than in the page so the check survives navigation, and so
	// there is exactly one of it.
	$effect(() => updates.watch());

	/**
	 * Suppresses the webview's own context menu, which in a packaged build offers
	 * "Reload" and nothing else — a browser control in an app with no address bar.
	 *
	 * Two things are deliberately left alone:
	 *
	 * - Editable fields, where the native menu is the only route to cut, copy and
	 *   paste for anyone who doesn't know the shortcuts. That includes CodeMirror,
	 *   which is a contenteditable rather than a textarea.
	 * - Anything inside a component that brings its own menu. Bits UI marks those
	 *   with `data-context-menu-trigger` and calls preventDefault itself; this
	 *   listener runs on the way up, after the trigger has already decided, and
	 *   bows out rather than cancelling twice.
	 */
	function suppressNativeMenu(event: MouseEvent) {
		const target = event.target as HTMLElement | null;
		if (!target) return;

		const editable = target.closest(
			'input, textarea, [contenteditable="true"], [contenteditable=""]'
		);
		if (editable) return;

		if (target.closest('[data-context-menu-trigger]')) return;

		event.preventDefault();
	}
</script>

<svelte:window oncontextmenu={suppressNativeMenu} />

<svelte:head>
	<title>Fiber</title>
</svelte:head>

{@render children()}

<UpdateToast />
