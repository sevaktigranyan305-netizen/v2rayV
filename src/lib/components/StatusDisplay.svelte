<script lang="ts">
	import { cn } from '$lib/utils';
	import type { ConnectionInfo } from '$lib/types';

	interface Props {
		info: ConnectionInfo;
	}

	const { info }: Props = $props();

	const statusLabel = $derived(() => {
		switch (info.status) {
			case 'connected':
				return 'Connected';
			case 'connecting':
				return 'Connecting...';
			case 'disconnecting':
				return 'Disconnecting...';
			case 'error':
				return 'Error';
			default:
				return 'Disconnected';
		}
	});

	const statusColor = $derived(() => {
		switch (info.status) {
			case 'connected':
				return 'bg-green-500';
			case 'connecting':
			case 'disconnecting':
				return 'bg-yellow-500';
			case 'error':
				return 'bg-red-500';
			default:
				return 'bg-zinc-500';
		}
	});
</script>

<!-- Status dot + label -->
<div class="flex items-center gap-2">
	<span class={cn('w-2.5 h-2.5 rounded-full', statusColor())}></span>
	<span class="text-sm font-medium text-foreground/80">{statusLabel()}</span>
</div>

<!-- Server info -->
{#if info.server_name || info.server_address}
	<div class="w-full bg-card rounded-lg border border-border p-4 flex flex-col gap-2">
		{#if info.server_name}
			<div class="flex justify-between text-sm">
				<span class="text-muted-foreground">Server</span>
				<span class="text-foreground font-medium">{info.server_name}</span>
			</div>
		{/if}
		{#if info.server_address}
			<div class="flex justify-between text-sm">
				<span class="text-muted-foreground">Address</span>
				<span class="text-foreground font-mono">{info.server_address}</span>
			</div>
		{/if}
	</div>
{/if}

<!-- Error message -->
{#if info.status === 'error' && info.error_message}
	<div class="w-full bg-destructive/10 border border-destructive/40 rounded-lg p-3">
		<p class="text-xs text-destructive text-center">{info.error_message}</p>
	</div>
{/if}
