<script lang="ts">
	interface Props {
		onImport: (name: string, url: string) => Promise<void>;
		onCancel: () => void;
	}

	const { onImport, onCancel }: Props = $props();

	let name = $state('');
	let url = $state('');
	let error = $state('');
	let busy = $state(false);

	async function handleSubmit(e: Event) {
		e.preventDefault();
		const trimmedName = name.trim();
		const trimmedUrl = url.trim();
		if (!trimmedName) {
			error = 'Please enter a name for this subscription';
			return;
		}
		if (!trimmedUrl) {
			error = 'Please enter a subscription URL';
			return;
		}
		if (!(trimmedUrl.startsWith('http://') || trimmedUrl.startsWith('https://'))) {
			error = 'URL must start with http:// or https://';
			return;
		}
		busy = true;
		try {
			await onImport(trimmedName, trimmedUrl);
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
		aria-label="Import from subscription URL"
		tabindex="-1"
	>
		<div class="flex items-center justify-between px-5 py-4 border-b border-border">
			<h2 class="text-base font-semibold text-foreground">Import from Subscription URL</h2>
			<button
				onclick={onCancel}
				disabled={busy}
				class="text-muted-foreground hover:text-foreground transition-colors p-1 rounded hover:bg-accent disabled:opacity-50"
				aria-label="Close"
			>
				<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<line x1="18" y1="6" x2="6" y2="18"/>
					<line x1="6" y1="6" x2="18" y2="18"/>
				</svg>
			</button>
		</div>

		<form onsubmit={handleSubmit} class="px-5 py-4 flex flex-col gap-4">
			<div class="flex flex-col gap-1">
				<label for="sub-name-input" class="text-xs font-medium text-muted-foreground uppercase tracking-wide">
					Name
				</label>
				<input
					id="sub-name-input"
					type="text"
					bind:value={name}
					placeholder="My provider"
					disabled={busy}
					class="w-full bg-background border border-border rounded-lg px-3 py-2 text-sm text-foreground placeholder:text-muted-foreground/50 focus:outline-none focus:ring-2 focus:ring-ring disabled:opacity-50"
					oninput={() => { error = ''; }}
				/>
				<p class="text-[11px] text-muted-foreground">
					A label you'll see in the Subscriptions list.
				</p>
			</div>

			<div class="flex flex-col gap-1">
				<label for="sub-url-input" class="text-xs font-medium text-muted-foreground uppercase tracking-wide">
					Subscription URL
				</label>
				<input
					id="sub-url-input"
					type="url"
					bind:value={url}
					placeholder="https://example.com/sub"
					disabled={busy}
					class="w-full bg-background border border-border rounded-lg px-3 py-2 text-sm text-foreground font-mono placeholder:text-muted-foreground/50 focus:outline-none focus:ring-2 focus:ring-ring disabled:opacity-50"
					oninput={() => { error = ''; }}
				/>
				<p class="text-[11px] text-muted-foreground">
					Plaintext or base64 list of <code>vless://</code> URIs, one per line.
				</p>
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
					disabled={busy}
					class="flex-1 py-2 rounded-lg bg-zinc-700 hover:bg-zinc-600 text-sm font-medium text-foreground transition-colors disabled:opacity-50"
				>
					{busy ? 'Fetching…' : 'Import'}
				</button>
			</div>
		</form>
	</div>
</div>
