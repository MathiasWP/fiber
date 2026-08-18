<script lang="ts">
	import { Dialog, Select, Tabs } from 'bits-ui';
	import {
		deleteSecret,
		forgetToken,
		hasSecret,
		setSecret,
		type AuthKind,
		type Section
	} from '$lib/api';
	import { collections } from '$lib/collections.svelte';

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
		}
	];

	const kind = $derived(section?.auth.kind ?? 'none');
	const hint = $derived(KINDS.find((entry) => entry.value === kind)?.hint ?? '');

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

	function close() {
		if (section) collections.flush(section);
		draftSecret = '';
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

						<p class="text-2.5 text-muted leading-relaxed">
							Dynamic endpoint loaders will live here too.
						</p>
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

						{#if section.auth.kind !== 'none'}
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
