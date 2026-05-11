<script lang="ts">
	import { cn } from '$lib/utils';
	import { serversStore } from '$lib/stores/servers.svelte';
	import type { ServerConfig } from '$lib/types';

	interface Props {
		onEdit: (server: ServerConfig) => void;
	}

	const { onEdit }: Props = $props();

	const store = serversStore;

	// Subscription-owned servers are rendered (read-only) inside
	// SubscriptionList — they don't get edit/delete buttons there because
	// they're owned by the subscription. Here we only show servers added
	// by hand so the buttons match the lifecycle.
	const manualServers = $derived(store.servers.filter((s) => !s.subscription_id));
</script>

{#if manualServers.length > 0}
<div class="w-full flex flex-col gap-1">
	<div class="flex items-center justify-between mb-2">
		<span class="text-xs font-semibold uppercase tracking-widest text-muted-foreground">Servers</span>
	</div>

	{#each manualServers as server (server.id)}
		<div
			class={cn(
				'flex items-center justify-between rounded-lg px-3 py-2.5 border cursor-pointer transition-colors',
				server.id === store.selectedId
					? 'border-zinc-500 bg-zinc-800/60 text-foreground'
					: 'border-transparent hover:border-zinc-700 hover:bg-zinc-800/30 text-foreground/70'
			)}
			onclick={() => store.selectServer(server.id)}
			role="button"
			tabindex="0"
			onkeydown={(e) => e.key === 'Enter' && store.selectServer(server.id)}
			aria-pressed={server.id === store.selectedId}
		>
			<div class="flex flex-col min-w-0">
				<span class="text-sm font-medium truncate">{server.name}</span>
				<span class="text-xs text-muted-foreground font-mono truncate">
					{server.address}:{server.port}
				</span>
			</div>

			<div class="flex items-center gap-1 ml-2 shrink-0">
				<button
					onclick={(e) => { e.stopPropagation(); onEdit(server); }}
					class="touch-target flex items-center justify-center rounded hover:bg-zinc-700 text-muted-foreground hover:text-foreground transition-colors"
					aria-label="Edit server"
				>
					<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						<path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/>
						<path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"/>
					</svg>
				</button>

				{#if manualServers.length > 0}
					<button
						onclick={(e) => { e.stopPropagation(); store.deleteServer(server.id); }}
						class="touch-target flex items-center justify-center rounded hover:bg-zinc-700 text-muted-foreground hover:text-destructive transition-colors"
						aria-label="Delete server"
					>
						<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
							<polyline points="3 6 5 6 21 6"/>
							<path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/>
							<path d="M10 11v6"/>
							<path d="M14 11v6"/>
							<path d="M9 6V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2"/>
						</svg>
					</button>
				{/if}
			</div>
		</div>
	{/each}
</div>
{/if}
