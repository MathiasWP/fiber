<script lang="ts">
	import { Dialog, Select, Tabs } from 'bits-ui';
	import {
		browserCapture,
		browserClose,
		browserSignIn,
		deleteSecret,
		forgetToken,
		hasSecret,
		normalizeBaseUrl,
		setSecret,
		type AuthKind,
		type CaptureKind,
		type Section
	} from '$lib/api';
	import { collections } from '$lib/collections.svelte';
	import CapturePicker from './CapturePicker.svelte';
	import ImportSpec from './ImportSpec.svelte';
	import LoaderTab from './LoaderTab.svelte';
	import { urlField } from '$lib/urlfield';

	interface Props {
		/** The section being edited, or null when closed. */
		section: Section | null;
		onClose: () => void;
	}

	let { section, onClose }: Props = $props();

	let tab = $state('general');
	/** Whether the keychain already holds this section's credential. */
	let stored = $state(false);
	/** The pending value, only ever written — never read back. */
	let draftSecret = $state('');
	let secretSaved = $state(false);
	let pickerOpen = $state(false);
	let captureError = $state<string | null>(null);

	const open = $derived(section !== null);

	const KINDS: { value: AuthKind; label: string; hint: string }[] = [
		{ value: 'none', label: 'None', hint: 'Requests go out as written.' },
		{
			value: 'bearer',
			label: 'Bearer token',
			hint: 'A fixed token, kept in your OS keychain.'
		},
		{
			value: 'login',
			label: 'Login request',
			hint: 'Fetch a token by making a request. Refreshed automatically on 401.'
		},
		{
			value: 'browser',
			label: 'Browser session',
			hint: 'Sign in in a real browser window, then lift the credential out. For verification codes, SDK-minted tokens and HttpOnly session cookies.'
		}
	];

	/**
	 * What each tab says when it has something in it.
	 *
	 * These used to be a bare "•", which marked the tab as configured without
	 * saying so — a dot you have to already know the meaning of tells you nothing
	 * the first time you see it. Naming the thing costs a few characters and
	 * answers the question outright.
	 */
	const KIND_LABEL: Record<AuthKind, string> = {
		none: '',
		bearer: 'bearer',
		login: 'login',
		browser: 'browser'
	};

	const kind = $derived(section?.auth.kind ?? 'none');
	const hint = $derived(KINDS.find((entry) => entry.value === kind)?.hint ?? '');

	const SOURCE_LABEL: Record<CaptureKind, string> = {
		cookie: 'cookie',
		localStorage: 'storage',
		indexedDb: 'indexeddb'
	};

	// Ask whether a credential exists whenever the dialog opens on a section.
	$effect(() => {
		const reference = section && 'secretRef' in section.auth ? section.auth.secretRef : null;
		draftSecret = '';
		secretSaved = false;
		if (!reference) {
			stored = false;
			return;
		}
		hasSecret(reference).then((exists) => (stored = exists));
	});

	function changeKind(next: AuthKind) {
		if (!section) return;
		const secretRef = `${section.id}:auth`;

		if (next === 'none') {
			section.auth = { kind: 'none' };
		} else if (next === 'bearer') {
			section.auth = { kind: 'bearer', secretRef };
		} else if (next === 'browser') {
			section.auth = {
				kind: 'browser',
				loginUrl: section.baseUrl || 'https://',
				capture: 'localStorage',
				captureKey: '',
				capturePath: '',
				header: 'Authorization',
				prefix: 'Bearer',
				ttlSeconds: 0,
				secretRef
			};
		} else {
			section.auth = {
				kind: 'login',
				method: 'POST',
				url: '/login',
				tokenPath: '$.access_token',
				header: 'Authorization',
				prefix: 'Bearer',
				ttlSeconds: 0,
				secretRef
			};
		}
	}

	async function saveSecret() {
		if (!section || !('secretRef' in section.auth) || !draftSecret.trim()) return;
		await setSecret(section.auth.secretRef, draftSecret);
		// Drop it from memory the moment it's in the keychain.
		draftSecret = '';
		stored = true;
		secretSaved = true;
		await forgetToken(section.id);
		await collections.refreshCredential(section);
	}

	async function removeSecret() {
		if (!section || !('secretRef' in section.auth)) return;
		await deleteSecret(section.auth.secretRef);
		stored = false;
		secretSaved = false;
		await forgetToken(section.id);
		await collections.refreshCredential(section);
	}

	async function signIn() {
		if (!section) return;
		captureError = null;
		// The window is opened from the section on disk, so persist first.
		await collections.flush(section);
		try {
			await browserSignIn(section.id);
		} catch (failure) {
			captureError = String(failure);
		}
	}

	/**
	 * A picked candidate becomes the section's capture rule, then we run the
	 * capture itself so the credential lands in the keychain immediately.
	 */
	async function applyRule(rule: { capture: CaptureKind; key: string; path: string }) {
		if (!section || section.auth.kind !== 'browser') return;
		pickerOpen = false;
		captureError = null;

		section.auth.capture = rule.capture;
		section.auth.captureKey = rule.key;
		section.auth.capturePath = rule.path;
		// A cookie goes out whole, as a Cookie header; a token gets a scheme.
		section.auth.header = rule.capture === 'cookie' ? 'Cookie' : 'Authorization';
		section.auth.prefix = rule.capture === 'cookie' ? '' : 'Bearer';

		await collections.flush(section);
		try {
			await browserCapture(section.id);
			stored = true;
			secretSaved = true;
			// The no-credential-to-credential transition, which is precisely what
			// the sidebar's shield is reporting.
			await collections.refreshCredential(section);
		} catch (failure) {
			captureError = String(failure);
		}
	}

	/**
	 * Done, as opposed to dismissing.
	 *
	 * A loader configured in here is worth nothing until it has run, and the
	 * alternative was closing this and then going to find Refresh. Deliberately
	 * not in `close()`: Escape and the scrim go through there too, and neither
	 * of those should fire a request at someone's API.
	 *
	 * Not awaited — the drawer shuts straight away and the sidebar shows the
	 * refresh running, which is where the result is going to appear anyway.
	 */
	function done() {
		if (section?.loader?.enabled) void collections.refresh(section);
		close();
	}

	function close() {
		if (section) {
			collections.refreshCredential(section);
			collections.flush(section);
			browserClose(section.id);
		}
		draftSecret = '';
		captureError = null;
		onClose();
	}
</script>

<Dialog.Root
	{open}
	onOpenChange={(next) => {
		if (!next) close();
	}}
>
	<!--
		No Portal, deliberately. Portalling would put this on `body`, where the only
		frame of reference is the window; rendered here it sits inside the pane
		beside the sidebar, so `inset-y-0 left-0` means the sidebar's own edge and
		stays right when that edge is dragged.

		The overlay covers the pane rather than the app, so the sidebar stays lit
		and usable — you are configuring a collection you can still see.
	-->
	<Dialog.Overlay class="drawer-scrim absolute inset-0 z-40 bg-black/40" />
	<Dialog.Content
		class="drawer absolute inset-y-0 left-0 z-50 flex w-[min(460px,80%)] flex-col border-r border-border bg-panel shadow-2xl"
	>
		<div class="p-4 pb-0 shrink-0">
				<Dialog.Title class="text-sm font-semibold">Section settings</Dialog.Title>
				<Dialog.Description class="mt-1 text-xs text-muted">
					Requests in this section only need a path — the base URL is prepended to each one.
				</Dialog.Description>
			</div>

			{#if section}
				<Tabs.Root bind:value={tab} class="flex min-h-0 flex-1 flex-col">
					<Tabs.List class="flex items-center gap-1 px-4 mt-3 border-b border-border">
						<Tabs.Trigger
							value="general"
							class="px-2 py-1.5 -mb-px border-b-2 border-transparent text-xs text-muted data-[state=active]:border-accent data-[state=active]:text-text hover:text-text transition-colors"
						>
							General
						</Tabs.Trigger>
						<Tabs.Trigger
							value="auth"
							class="px-2 py-1.5 -mb-px border-b-2 border-transparent text-xs text-muted data-[state=active]:border-accent data-[state=active]:text-text hover:text-text transition-colors"
						>
							Auth{kind === 'none' ? '' : ` · ${KIND_LABEL[kind]}`}
						</Tabs.Trigger>
						<Tabs.Trigger
							value="loader"
							class="px-2 py-1.5 -mb-px border-b-2 border-transparent text-xs text-muted data-[state=active]:border-accent data-[state=active]:text-text hover:text-text transition-colors"
						>
							Loader{section.loader ? (section.loader.enabled ? ' · on' : ' · off') : ''}
						</Tabs.Trigger>
					</Tabs.List>

					<!--
						The drawer is as tall as the window, so there is room for the
						tallest tab without anything being cut off — which is what made a
						fixed-height scroller necessary when this was a centred dialog.
						`overflow-y-auto` is left as a floor for a genuinely short window,
						not as the normal way to read a tab.
					-->
					<div class="flex-1 min-h-0 overflow-y-auto">
						<Tabs.Content value="general" class="p-4 flex flex-col gap-3">
						<label class="flex flex-col gap-1">
							<span class="text-xs text-muted">Name</span>
							<input bind:value={section.name} class="input-base text-xs" />
						</label>

						<label class="flex flex-col gap-1">
							<span class="text-xs text-muted">Base URL</span>
							<!-- Tidied on the way out rather than as you type, so a slash
							     you are still typing past isn't snatched away. -->
							<input
								bind:value={section.baseUrl}
								use:urlField
								spellcheck="false"
								placeholder="https://api.example.com"
								class="input-base text-xs font-mono selectable"
								onblur={() => {
									section.baseUrl = normalizeBaseUrl(section.baseUrl);
									collections.touch(section);
								}}
							/>
						</label>

						<ImportSpec {section} />

						<div class="rounded border border-border p-3 flex flex-col gap-2">
							<div class="flex items-center gap-2">
								<span class="i-lucide-bot text-3 text-muted"></span>
								<span class="text-xs font-medium">Share with agents (MCP)</span>
							</div>

							<label class="flex items-center gap-1.5 text-xs">
								<input type="checkbox" bind:checked={section.mcp.enabled} />
								Let agents see and call this collection
							</label>
							<label class="flex items-center gap-1.5 text-xs {section.mcp.enabled ? '' : 'opacity-40'}">
								<input
									type="checkbox"
									disabled={!section.mcp.enabled}
									bind:checked={section.mcp.allowWrites}
								/>
								Allow more than GET, HEAD and OPTIONS
							</label>

							<p class="text-2.5 text-muted leading-relaxed">
								Shared read-only by default: an agent can see and call this collection, but only
								with GET, HEAD and OPTIONS until you allow more. It's authenticated as you, and
								credentials are never returned. Turn the top switch off to hide it entirely — an
								unshared collection is invisible, not merely read-only.
							</p>
						</div>
					</Tabs.Content>

					<Tabs.Content value="loader" class="p-4 flex flex-col gap-3">
						<LoaderTab {section} />
					</Tabs.Content>

					<Tabs.Content value="auth" class="p-4 flex flex-col gap-3">
						<label class="flex flex-col gap-1">
							<span class="text-xs text-muted">Method</span>
							<Select.Root
								type="single"
								value={kind}
								onValueChange={(next) => changeKind(next as AuthKind)}
							>
								<Select.Trigger class="input-base flex items-center gap-2 text-xs w-full">
									<span class="flex-1 text-left">
										{KINDS.find((entry) => entry.value === kind)?.label}
									</span>
									<span class="i-lucide-chevron-down text-3 text-muted"></span>
								</Select.Trigger>
								<Select.Portal>
									<Select.Content class="menu-content w-64">
										<Select.Viewport>
											{#each KINDS as entry (entry.value)}
												<Select.Item value={entry.value} label={entry.label} class="menu-item">
													{#snippet children({ selected })}
														{entry.label}
														{#if selected}
															<span class="i-lucide-check ml-auto text-3 text-muted"></span>
														{/if}
													{/snippet}
												</Select.Item>
											{/each}
										</Select.Viewport>
									</Select.Content>
								</Select.Portal>
							</Select.Root>
							<span class="text-2.5 text-muted">{hint}</span>
						</label>

						{#if section.auth.kind === 'login'}
							<div class="grid grid-cols-[1fr_2fr] gap-2">
								<label class="flex flex-col gap-1">
									<span class="text-xs text-muted">Method</span>
									<input
										bind:value={section.auth.method}
										spellcheck="false"
										class="input-base text-xs font-mono"
									/>
								</label>
								<label class="flex flex-col gap-1">
									<span class="text-xs text-muted">Login URL</span>
									<input
										bind:value={section.auth.url}
										use:urlField
										spellcheck="false"
										placeholder="/login"
										class="input-base text-xs font-mono selectable"
									/>
								</label>
							</div>

							<label class="flex flex-col gap-1">
								<span class="text-xs text-muted">Token path</span>
								<input
									bind:value={section.auth.tokenPath}
									spellcheck="false"
									placeholder="$.access_token"
									class="input-base text-xs font-mono selectable"
								/>
								<span class="text-2.5 text-muted">
									Where the token sits in the login response. Dotted path; numbers index arrays.
								</span>
							</label>

							<div class="grid grid-cols-3 gap-2">
								<label class="flex flex-col gap-1">
									<span class="text-xs text-muted">Header</span>
									<input
										bind:value={section.auth.header}
										spellcheck="false"
										class="input-base text-xs font-mono"
									/>
								</label>
								<label class="flex flex-col gap-1">
									<span class="text-xs text-muted">Prefix</span>
									<input
										bind:value={section.auth.prefix}
										spellcheck="false"
										class="input-base text-xs font-mono"
									/>
								</label>
								<label class="flex flex-col gap-1">
									<span class="text-xs text-muted">TTL (s)</span>
									<input
										type="number"
										min="0"
										bind:value={section.auth.ttlSeconds}
										class="input-base text-xs font-mono"
									/>
								</label>
							</div>
							<p class="text-2.5 text-muted -mt-1">
								TTL 0 keeps the token until the API rejects it. Either way a 401 triggers one silent
								re-login and retry.
							</p>
						{/if}

						{#if section.auth.kind === 'browser'}
							<label class="flex flex-col gap-1">
								<span class="text-xs text-muted">Sign-in page</span>
								<input
									bind:value={section.auth.loginUrl}
									use:urlField
									spellcheck="false"
									placeholder="https://app.example.com/login"
									class="input-base text-xs font-mono selectable"
								/>
							</label>

							<div class="rounded border border-border p-3 flex flex-col gap-2">
								<div class="flex items-center gap-2">
									<span class="i-lucide-globe text-3 text-muted"></span>
									<span class="text-xs font-medium">Browser session</span>
									<span class="ml-auto text-2.5 {stored ? 'text-ok' : 'text-muted'}">
										{stored ? 'captured' : 'not captured'}
									</span>
								</div>

								<ol class="text-2.5 text-muted leading-relaxed list-decimal pl-4 flex flex-col gap-0.5">
									<li>Open the sign-in window and log in exactly as you normally would.</li>
									<li>
										Pick which value is your credential — you can close the sign-in window first,
										the session is remembered.
									</li>
								</ol>

								<div class="flex items-center gap-2">
									<button class="btn-primary text-xs" onclick={signIn}>
										<span class="i-lucide-external-link"></span>
										Open sign-in
									</button>
									<button class="btn-ghost text-xs" onclick={() => (pickerOpen = true)}>
										Pick credential…
									</button>
									{#if stored}
										<button class="btn-ghost text-xs" onclick={removeSecret}>Remove</button>
									{/if}
								</div>

								{#if section.auth.captureKey}
									<p class="text-2.5 text-muted font-mono truncate">
										{SOURCE_LABEL[section.auth.capture]} ·
										{section.auth.captureKey}{section.auth.capturePath
											? ` › ${section.auth.capturePath}`
											: ''}
									</p>
								{/if}

								{#if captureError}
									<p class="text-2.5 text-bad">{captureError}</p>
								{/if}

								<p class="text-2.5 text-muted leading-relaxed">
									On a 401 the sign-in page is reopened hidden. If the identity provider still
									considers you signed in, a fresh credential is captured and you never see a
									window; otherwise it comes forward so you can sign in again.
								</p>
							</div>
						{/if}

						{#if section.auth.kind === 'bearer' || section.auth.kind === 'login'}
							<div class="rounded border border-border p-3 flex flex-col gap-2">
								<div class="flex items-center gap-2">
									<span class="i-lucide-key-round text-3 text-muted"></span>
									<span class="text-xs font-medium">
										{section.auth.kind === 'bearer' ? 'Token' : 'Login request body'}
									</span>
									<span class="ml-auto text-2.5 {stored ? 'text-ok' : 'text-muted'}">
										{stored ? 'stored in keychain' : 'not set'}
									</span>
								</div>

								<textarea
									bind:value={draftSecret}
									spellcheck="false"
									rows={section.auth.kind === 'bearer' ? 2 : 4}
									placeholder={section.auth.kind === 'bearer'
										? 'Paste the token'
										: '{"username": "me", "password": "…"}'}
									class="input-base text-xs font-mono resize-none selectable"
								></textarea>

								<div class="flex items-center gap-2">
									<button
										class="btn-primary text-xs"
										disabled={!draftSecret.trim()}
										onclick={saveSecret}
									>
										{stored ? 'Replace' : 'Save'}
									</button>
									{#if stored}
										<button class="btn-ghost text-xs" onclick={removeSecret}>Remove</button>
									{/if}
									{#if section.auth.kind === 'login'}
										<button
											class="btn-ghost text-xs"
											title="Discard the cached token so the next send logs in again"
											onclick={() => section && forgetToken(section.id)}
										>
											Forget token
										</button>
									{/if}
									{#if secretSaved}
										<span class="text-2.5 text-ok">Saved</span>
									{/if}
								</div>

								<p class="text-2.5 text-muted leading-relaxed">
									Kept in your OS keychain, never in the section file — so the file stays safe to
									share or commit. It can't be read back out, only replaced.
								</p>
							</div>
						{/if}
						</Tabs.Content>
					</div>
				</Tabs.Root>

				<div class="flex justify-end gap-2 p-4 pt-0">
					<button class="btn-primary text-xs" onclick={done}>Done</button>
				</div>
			{/if}
	</Dialog.Content>
</Dialog.Root>

{#if section}
	<CapturePicker
		open={pickerOpen}
		sectionId={section.id}
		onPick={applyRule}
		onClose={() => (pickerOpen = false)}
	/>
{/if}

<style>
	/*
	 * Bits UI renders the content, so these have to be :global — a scoped rule
	 * would never reach it. It keeps the element mounted through the closing
	 * animation and swaps data-state, which is what both of these hang off.
	 */
	:global(.drawer[data-state='open']) {
		animation: drawer-in 180ms cubic-bezier(0.32, 0.72, 0, 1);
	}
	:global(.drawer[data-state='closed']) {
		animation: drawer-out 140ms cubic-bezier(0.32, 0.72, 0, 1);
	}
	:global(.drawer-scrim[data-state='open']) {
		animation: scrim-in 180ms ease-out;
	}
	:global(.drawer-scrim[data-state='closed']) {
		animation: scrim-out 140ms ease-in;
	}

	@keyframes drawer-in {
		from {
			transform: translateX(-100%);
		}
	}
	@keyframes drawer-out {
		to {
			transform: translateX(-100%);
		}
	}
	@keyframes scrim-in {
		from {
			opacity: 0;
		}
	}
	@keyframes scrim-out {
		to {
			opacity: 0;
		}
	}

	/* Sliding panels are the classic case for this: motion that conveys nothing
	   the layout does not already say. */
	@media (prefers-reduced-motion: reduce) {
		:global(.drawer),
		:global(.drawer-scrim) {
			animation: none;
		}
	}
</style>
