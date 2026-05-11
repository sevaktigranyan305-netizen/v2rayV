import { getConnectionInfo, connect, disconnect, getSpeedStats } from '$lib/api/tauri';
import type { ConnectionInfo, ServerConfig, SpeedStats } from '$lib/types';

const DEFAULT_INFO: ConnectionInfo = {
	status: 'disconnected',
	server_name: null,
	server_address: null,
	connected_since: null,
	error_message: null
};

const DEFAULT_STATS: SpeedStats = {
	upload_speed: 0,
	download_speed: 0,
	total_upload: 0,
	total_download: 0
};

function createConnectionStore() {
	let info = $state<ConnectionInfo>({ ...DEFAULT_INFO });
	let isLoading = $state(false);
	let pollInterval: ReturnType<typeof setInterval> | null = null;
	let stats = $state<SpeedStats>({ ...DEFAULT_STATS });
	let speedHistory = $state<{ upload: number[]; download: number[] }>({ upload: [], download: [] });

	const isConnected = $derived(info.status === 'connected');
	const isTransitioning = $derived(
		info.status === 'connecting' || info.status === 'disconnecting'
	);

	function startPolling() {
		if (pollInterval !== null) return;
		pollInterval = setInterval(async () => {
			try {
				const result = await getConnectionInfo();
				info = result;
				if (result.status === 'connected') {
					const s = await getSpeedStats();
					stats = s;
					// Keep last 60 data points
					speedHistory = {
						upload: [...speedHistory.upload, s.upload_speed].slice(-60),
						download: [...speedHistory.download, s.download_speed].slice(-60)
					};
				} else {
					stats = { ...DEFAULT_STATS };
					speedHistory = { upload: [], download: [] };
					// Once the backend has settled into a non-active state, stop
					// polling so we don't burn IPC for nothing. The next
					// connectVpn / disconnectVpn call restarts the loop.
					if (result.status === 'disconnected' || result.status === 'error') {
						stopPolling();
					}
				}
			} catch {
				// Ignore polling errors silently
			}
		}, 1000);
	}

	function stopPolling() {
		if (pollInterval !== null) {
			clearInterval(pollInterval);
			pollInterval = null;
		}
	}

	async function refresh() {
		try {
			info = await getConnectionInfo();
		} catch {
			// Ignore
		}
	}

	async function connectVpn(config: ServerConfig) {
		// Guard against double-click while a transition is in flight. Use the
		// observed status (driven by the poll loop) rather than the in-flight
		// `isLoading` flag, so a stale lingering `isLoading=true` after the
		// backend has already settled doesn't lock the UI.
		if (info.status === 'connecting' || info.status === 'disconnecting') return;
		isLoading = true;
		info = { ...info, status: 'connecting', error_message: null };
		try {
			await connect(config);
			await refresh();
			startPolling();
		} catch (err) {
			info = {
				...info,
				status: 'error',
				error_message: err instanceof Error ? err.message : String(err)
			};
		} finally {
			isLoading = false;
		}
	}

	async function disconnectVpn() {
		if (info.status === 'disconnecting' || info.status === 'disconnected') return;
		isLoading = true;
		info = { ...info, status: 'disconnecting' };

		// Race the disconnect IPC against an 8 s watchdog. On the slow
		// Parallels x86_64 emulation we have observed cases where xray has
		// already exited (`xray terminated` shows up in our log) but the
		// `disconnect` IPC response is still in flight, leaving the button
		// stuck on "Disconnecting…". The watchdog forces us to fall back
		// to the poll loop, which reads the authoritative state every 1 s
		// directly from the XrayManager mutex.
		const watchdog = new Promise<'timeout'>((resolve) =>
			setTimeout(() => resolve('timeout'), 8000)
		);

		try {
			const result = await Promise.race([
				disconnect().then(() => 'done' as const),
				watchdog
			]);
			if (result === 'timeout') {
				// Force a single status read to push the UI off
				// "Disconnecting…" right now; the poll loop will keep it
				// honest from here on out.
				await refresh();
				if (info.status === 'disconnecting') {
					info = { ...info, status: 'disconnected' };
				}
			} else {
				await refresh();
				stats = { ...DEFAULT_STATS };
				speedHistory = { upload: [], download: [] };
			}
		} catch (err) {
			info = {
				...info,
				status: 'error',
				error_message: err instanceof Error ? err.message : String(err)
			};
			await refresh();
		} finally {
			isLoading = false;
			// Don't aggressively stopPolling here — let the poll loop itself
			// stop once it observes a settled status (see startPolling).
		}
	}

	return {
		get info() {
			return info;
		},
		get isLoading() {
			return isLoading;
		},
		get isConnected() {
			return isConnected;
		},
		get isTransitioning() {
			return isTransitioning;
		},
		get stats() {
			return stats;
		},
		get speedHistory() {
			return speedHistory;
		},
		refresh,
		connectVpn,
		disconnectVpn,
		startPolling,
		stopPolling
	};
}

export const connectionStore = createConnectionStore();
