import type { Subscription } from '$lib/types';
import * as api from '$lib/api/tauri';

function createSubscriptionsStore() {
	let subscriptions = $state<Subscription[]>([]);
	let loadError = $state<string | null>(null);

	async function load() {
		try {
			subscriptions = await api.listSubscriptions();
			loadError = null;
		} catch (err) {
			loadError = err instanceof Error ? err.message : String(err);
			throw err;
		}
	}

	/** Save a new subscription and import its servers. Returns the number of
	 * servers imported so the caller can show a toast. */
	async function add(name: string, url: string): Promise<number> {
		const result = await api.addSubscription(name, url);
		await load();
		return result.servers.length;
	}

	/** Re-fetch the saved URL and replace every server tagged with this
	 * subscription's id. */
	async function refresh(id: string): Promise<number> {
		const result = await api.refreshSubscription(id);
		await load();
		return result.servers.length;
	}

	async function remove(id: string, deleteServers: boolean): Promise<void> {
		await api.deleteSubscription(id, deleteServers);
		await load();
	}

	return {
		get subscriptions() {
			return subscriptions;
		},
		get loadError() {
			return loadError;
		},
		load,
		add,
		refresh,
		remove
	};
}

export const subscriptionsStore = createSubscriptionsStore();
