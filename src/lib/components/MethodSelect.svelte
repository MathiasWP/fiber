<script lang="ts">
	import { Select } from 'bits-ui';
	import { METHODS, methodColor } from '$lib/api';

	interface Props {
		value: string;
	}

	let { value = $bindable('GET') }: Props = $props();

	const items = METHODS.map((method) => ({ value: method, label: method }));
</script>

<Select.Root type="single" bind:value {items}>
	<Select.Trigger
		class="input-base flex items-center gap-1.5 font-mono font-bold text-xs w-24 shrink-0 {methodColor(
			value
		)}"
		aria-label="HTTP method"
	>
		<span class="flex-1 text-left">{value}</span>
		<span class="i-lucide-chevron-down text-3 text-muted"></span>
	</Select.Trigger>

	<Select.Portal>
		<Select.Content class="menu-content w-24">
			<Select.Viewport>
				{#each items as item (item.value)}
					<Select.Item value={item.value} label={item.label} class="menu-item font-mono font-bold">
						{#snippet children({ selected })}
							<span class={methodColor(item.value)}>{item.label}</span>
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
