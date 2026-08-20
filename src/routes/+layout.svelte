<script lang="ts">
	// UnoCSS utilities are generated into app.css by @unocss/postcss.
	import '../app.css';
	import UpdateToast from '$lib/components/UpdateToast.svelte';
	import { updates } from '$lib/update.svelte';

	let { children } = $props();

	// Here rather than in the page so the check survives navigation, and so
	// there is exactly one of it.
	$effect(() => updates.watch());
</script>

<svelte:head>
	<title>Fiber</title>
</svelte:head>

{@render children()}

<UpdateToast />
