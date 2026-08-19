<script lang="ts">
	import { Dialog, Select, Tabs } from 'bits-ui';
	import {
		browserCapture,
		browserClose,
		browserSignIn,
		deleteSecret,
		forgetToken,
		hasSecret,
		setSecret,
		type AuthKind,
		type CaptureKind,
		type Section
	} from '$lib/api';
	import { collections } from '$lib/collections.svelte';
	import CapturePicker from './CapturePicker.svelte';
	import LoaderTab from './LoaderTab.svelte';

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
	}

	async function removeSecret() {
		if (!section || !('secretRef' in section.auth)) return;
		await deleteSecret(section.auth.secretRef);
		stored = false;
		secretSaved = false;
		await forgetToken(section.id);
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
		} catch (failure) {
			captureError = String(failure);
		}
	}

	function close() {
		if (section) {
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
	<Dialog.Portal>
		<Dialog.Overlay class="fixed inset-0 bg-black/50" />
		<Dialog.Content
			class="fixed left-1/2 top-1/2 w-[min(560px,92vw)] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-panel shadow-2xl"
		>
			<div class="p-4 pb-0">
				<Dialog.Title class="text-sm font-semibold">Section settings</Dialog.Title>
				<Dialog.Description class="mt-1 text-xs text-muted">
					Requests in this section only need a path — the base URL is prepended to each one.
				</Dialog.Description>
			</div>

			{#if section}
				<Tabs.Root bind:value={tab}>
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
							Auth{kind === 'none' ? '' : ' •'}
						</Tabs.Trigger>
						<Tabs.Trigger
							value="loader"
							class="px-2 py-1.5 -mb-px border-b-2 border-transparent text-xs text-muted data-[state=active]:border-accent data-[state=active]:text-text hover:text-text transition-colors"
						>
							Loader{section.loader ? ' •' : ''}
						</Tabs.Trigger>
					</Tabs.List>

					<Tabs.Content value="general" class="p-4 flex flex-col gap-3">
						<label class="flex flex-col gap-1">
							<span class="text-xs text-muted">Name</span>
							<input bind:value={section.name} class="input-base text-xs" />
						</label>

						<label class="flex flex-col gap-1">
							<span class="text-xs text-muted">Base URL</span>
							<input
								bind:value={section.baseUrl}
								spellcheck="false"
								placeholder="https://api.example.com"
								class="input-base text-xs font-mono selectable"
							/>
						</label>

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
								Off by default. An agent calling this collection is authenticated as you, so
								nothing is exposed until you say so — a collection that isn't shared is invisible,
								not merely read-only. Credentials are never returned to the agent.
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
				</Tabs.Root>

				<div class="flex justify-end gap-2 p-4 pt-0">
					<button class="btn-primary text-xs" onclick={close}>Done</button>
				</div>
			{/if}
		</Dialog.Content>
	</Dialog.Portal>
</Dialog.Root>

{#if section}
	<CapturePicker
		open={pickerOpen}
		sectionId={section.id}
		onPick={applyRule}
		onClose={() => (pickerOpen = false)}
	/>
{/if}
