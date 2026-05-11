<script lang="ts">
	import type { Subscription } from '$lib/types';

	interface Props {
		subscriptions: Subscription[];
		onRefresh: (id: string) => Promise<void>;
		onDelete: (id: string, deleteServers: boolean) => Promise<void>;
	}

	const { subscriptions, onRefresh, onDelete }: Props = $props();

	// Per-row busy flag so spinners only spin on the row the user clicked.
	let busyId = $state<string | null>(null);
	let confirmDeleteId = $state<string | null>(null);

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
		try {
			await onRefresh(id);
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

{#if subscriptions.length > 0}
	<div class="w-full flex flex-col gap-1.5">
		<h2 class="text-xs font-medium text-muted-foreground uppercase tracking-wide">Subscriptions</h2>
		<ul class="flex flex-col gap-1.5">
			{#each subscriptions as sub (sub.id)}
				<li class="bg-card border border-border rounded-lg px-3 py-2 flex items-center gap-3">
					<div class="flex-1 min-w-0">
						<div class="text-sm text-foreground font-medium truncate">{sub.name}</div>
						<div class="text-[11px] text-muted-foreground truncate font-mono">{sub.url}</div>
						<div class="text-[11px] text-muted-foreground">
							{sub.last_server_count ?? 0} server(s) · updated {formatTimestamp(sub.last_updated_at)}
						</div>
					</div>

					{#if confirmDeleteId === sub.id}
						<div class="flex items-center gap-1.5">
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
								onclick={() => { confirmDeleteId = null; }}
								disabled={busyId === sub.id}
								class="text-[11px] text-muted-foreground hover:text-foreground transition-colors px-2 py-1 rounded disabled:opacity-50"
							>
								Cancel
							</button>
						</div>
					{:else}
						<div class="flex items-center gap-1.5">
							<button
								onclick={() => handleRefresh(sub.id)}
								disabled={busyId === sub.id}
								class="text-[11px] text-muted-foreground hover:text-foreground transition-colors px-2 py-1 rounded border border-border hover:border-zinc-500 disabled:opacity-50"
								title="Re-fetch URL and replace servers"
							>
								{busyId === sub.id ? 'Refreshing…' : 'Refresh'}
							</button>
							<button
								onclick={() => { confirmDeleteId = sub.id; }}
								disabled={busyId === sub.id}
								class="text-[11px] text-muted-foreground hover:text-destructive transition-colors px-2 py-1 rounded border border-border hover:border-destructive/40 disabled:opacity-50"
								title="Delete subscription"
							>
								Delete
							</button>
						</div>
					{/if}
				</li>
			{/each}
		</ul>
	</div>
{/if}
