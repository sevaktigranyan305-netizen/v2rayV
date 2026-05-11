<script lang="ts">
	import { cn } from '$lib/utils';
	import { serversStore } from '$lib/stores/servers.svelte';
	import { subscriptionsStore } from '$lib/stores/subscriptions.svelte';
	import type { Subscription, ServerConfig } from '$lib/types';

	interface Props {
		subscriptions: Subscription[];
		servers: ServerConfig[];
		onRefresh: (id: string) => Promise<void>;
		onDelete: (id: string, deleteServers: boolean) => Promise<void>;
	}

	const { subscriptions, servers, onRefresh, onDelete }: Props = $props();

	const store = serversStore;

	// Per-row busy flag so spinners only spin on the row the user clicked.
	let busyId = $state<string | null>(null);
	let confirmDeleteId = $state<string | null>(null);

	// Sort subscriptions by add time (oldest first). last_updated_at is also
	// the creation time on first import, so it doubles as a created_at when
	// a subscription has never been refreshed.
	const sortedSubs = $derived(
		[...subscriptions].sort((a, b) => (a.last_updated_at ?? 0) - (b.last_updated_at ?? 0))
	);

	function serversForSubscription(subId: string): ServerConfig[] {
		return servers.filter((s) => s.subscription_id === subId);
	}

	function formatTimestamp(ts: number | null | undefined): string {
		if (!ts) return 'never';
		const d = new Date(ts * 1000);
		const now = Date.now();
		const diffSec = Math.floor((now - ts * 1000) / 1000);
		if (diffSec < 60) return 'just now';
		if (diffSec < 3600) return `${Math.floor(diffSec / 60)}m ago`;
		if (diffSec < 86400) return `${Math.floor(diffSec / 3600)}h ago`;
		return d.toLocaleDateString();
	}

	async function handleRefresh(id: string) {
		busyId = id;
		// Race the refresh against a 25-second watchdog. On the slow Parallels
		// x86_64 emulation we have observed cases where the IPC response is
		// delayed long after the backend has finished writing
		// subscriptions.json, leaving the row stuck on "Refreshing…" until the
		// app is restarted. The watchdog forces a manual store reload so the
		// UI converges on the on-disk state even if the original promise
		// never resolves.
		const watchdog = new Promise<'timeout'>((resolve) =>
			setTimeout(() => resolve('timeout'), 25000)
		);
		try {
			const result = await Promise.race([
				onRefresh(id).then(() => 'done' as const),
				watchdog
			]);
			if (result === 'timeout') {
				try {
					await Promise.all([subscriptionsStore.load(), serversStore.load()]);
				} catch {
					// Ignore — the next user action will retry.
				}
			}
		} finally {
			busyId = null;
		}
	}

	async function handleDelete(id: string, deleteServers: boolean) {
		busyId = id;
		try {
			await onDelete(id, deleteServers);
			confirmDeleteId = null;
		} finally {
			busyId = null;
		}
	}
</script>

{#if sortedSubs.length > 0}
	<div class="w-full flex flex-col gap-3">
		{#each sortedSubs as sub (sub.id)}
			{@const subServers = serversForSubscription(sub.id)}
			<div class="w-full flex flex-col gap-1.5">
				<!-- Subscription header (name + Refresh/Delete) -->
				<div class="flex items-center justify-between gap-2 px-1">
					<div class="flex-1 min-w-0">
						<div class="text-sm font-semibold text-foreground truncate">{sub.name}</div>
						<div class="text-[11px] text-muted-foreground">
							{subServers.length} server(s) · updated {formatTimestamp(sub.last_updated_at)}
						</div>
					</div>

					{#if confirmDeleteId === sub.id}
						<div class="flex items-center gap-1.5 shrink-0">
							<button
								onclick={() => handleDelete(sub.id, true)}
								disabled={busyId === sub.id}
								class="text-[11px] text-destructive hover:text-red-400 transition-colors px-2 py-1 rounded border border-destructive/40 disabled:opacity-50"
								title="Delete subscription and its servers"
							>
								Delete + servers
							</button>
							<button
								onclick={() => handleDelete(sub.id, false)}
								disabled={busyId === sub.id}
								class="text-[11px] text-muted-foreground hover:text-foreground transition-colors px-2 py-1 rounded border border-border disabled:opacity-50"
								title="Delete subscription only, keep servers"
							>
								Keep servers
							</button>
							<button
								onclick={() => {
									confirmDeleteId = null;
								}}
								disabled={busyId === sub.id}
								class="text-[11px] text-muted-foreground hover:text-foreground transition-colors px-2 py-1 rounded disabled:opacity-50"
							>
								Cancel
							</button>
						</div>
					{:else}
						<div class="flex items-center gap-1.5 shrink-0">
							<button
								onclick={() => handleRefresh(sub.id)}
								disabled={busyId === sub.id}
								class="text-[11px] text-muted-foreground hover:text-foreground transition-colors px-2 py-1 rounded border border-border hover:border-zinc-500 disabled:opacity-50"
								title="Re-fetch URL and replace servers"
							>
								{busyId === sub.id ? 'Refreshing…' : 'Refresh'}
							</button>
							<button
								onclick={() => {
									confirmDeleteId = sub.id;
								}}
								disabled={busyId === sub.id}
								class="text-[11px] text-muted-foreground hover:text-destructive transition-colors px-2 py-1 rounded border border-border hover:border-destructive/40 disabled:opacity-50"
								title="Delete subscription"
							>
								Delete
							</button>
						</div>
					{/if}
				</div>

				<!-- Subscription's servers (no edit/delete — owned by the sub) -->
				{#if subServers.length === 0}
					<div class="text-xs text-muted-foreground italic px-3 py-2">
						(no servers)
					</div>
				{:else}
					<div class="flex flex-col gap-1">
						{#each subServers as server (server.id)}
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
							</div>
						{/each}
					</div>
				{/if}
			</div>
		{/each}
	</div>
{/if}
