<script lang="ts">
	import { storeSudoPassword } from '$lib/api/tauri';

	interface Props {
		/** Called after the password validated against `sudo -v` and was
		 * persisted to the OS credential store (Keychain on macOS,
		 * Secret Service on Linux). The caller typically re-runs
		 * `connectVpn` here. */
		onSuccess: () => void;
		/** Called when the user explicitly cancels the prompt (Esc /
		 * Cancel button / backdrop click). */
		onCancel: () => void;
	}

	const { onSuccess, onCancel }: Props = $props();

	let password = $state('');
	let error = $state('');
	let busy = $state(false);

	async function submit(e: Event) {
		e.preventDefault();
		if (busy) return;
		const trimmed = password;
		if (!trimmed) {
			error = 'Please enter your sudo password';
			return;
		}
		busy = true;
		error = '';
		try {
			await storeSudoPassword(trimmed);
			password = '';
			onSuccess();
		} catch (err) {
			error = err instanceof Error ? err.message : String(err);
		} finally {
			busy = false;
		}
	}

	function handleBackdropClick(e: MouseEvent) {
		if (e.target === e.currentTarget && !busy) onCancel();
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape' && !busy) onCancel();
	}
</script>

<svelte:window onkeydown={handleKeydown} />

<div
	class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
	onclick={handleBackdropClick}
	onkeydown={(e) => e.key === 'Escape' && !busy && onCancel()}
	role="presentation"
	tabindex="-1"
>
	<div
		class="w-full max-w-md mx-4 bg-card border border-border rounded-xl shadow-2xl overflow-hidden"
		role="dialog"
		aria-modal="true"
		aria-label="Enter sudo password"
		tabindex="-1"
	>
		<div class="flex items-center justify-between px-5 py-4 border-b border-border">
			<h2 class="text-base font-semibold text-foreground">Sudo password</h2>
			<button
				onclick={onCancel}
				disabled={busy}
				class="text-muted-foreground hover:text-foreground transition-colors p-1 rounded hover:bg-accent disabled:opacity-50"
				aria-label="Close"
			>
				<svg
					xmlns="http://www.w3.org/2000/svg"
					width="16"
					height="16"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
					stroke-width="2"
					stroke-linecap="round"
					stroke-linejoin="round"
				>
					<line x1="18" y1="6" x2="6" y2="18" />
					<line x1="6" y1="6" x2="18" y2="18" />
				</svg>
			</button>
		</div>

		<form onsubmit={submit} class="px-5 py-4 flex flex-col gap-4">
			<p class="text-sm text-muted-foreground leading-relaxed">
				v2rayV needs your account's sudo password once so it can run
				xray-core with the privileges required to create the L3 TUN
				network interface. The password is stored in your OS credential
				store (login Keychain on macOS, Secret Service on Linux) and is
				not transmitted anywhere.
			</p>

			<div class="flex flex-col gap-1">
				<label
					for="sudo-password-input"
					class="text-xs font-medium text-muted-foreground uppercase tracking-wide"
				>
					Password
				</label>
				<input
					id="sudo-password-input"
					type="password"
					bind:value={password}
					autocomplete="current-password"
					disabled={busy}
					class="w-full bg-background border border-border rounded-lg px-3 py-2 text-sm text-foreground placeholder:text-muted-foreground/50 focus:outline-none focus:ring-2 focus:ring-ring disabled:opacity-50"
					oninput={() => {
						error = '';
					}}
				/>
				{#if error}
					<p class="text-xs text-destructive">{error}</p>
				{/if}
			</div>

			<div class="flex gap-3">
				<button
					type="button"
					onclick={onCancel}
					disabled={busy}
					class="flex-1 py-2 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:text-foreground hover:border-zinc-500 transition-colors disabled:opacity-50"
				>
					Cancel
				</button>
				<button
					type="submit"
					disabled={busy || !password}
					class="flex-1 py-2 rounded-lg bg-zinc-700 hover:bg-zinc-600 text-sm font-medium text-foreground transition-colors disabled:opacity-50"
				>
					{busy ? 'Verifying…' : 'Save & connect'}
				</button>
			</div>
		</form>
	</div>
</div>
