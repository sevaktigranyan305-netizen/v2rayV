export interface RealitySettings {
	public_key: string;
	short_id: string;
	server_name: string;
	fingerprint: string;
}

export interface VirtualNetSettings {
	enabled: boolean;
	subnet: string;
	vnet_ip: string;
	default_route: boolean;
	interface_name?: string | null;
	mtu?: number | null;
}

export interface ServerConfig {
	id: string;
	name: string;
	address: string;
	port: number;
	uuid: string;
	flow: string;
	reality: RealitySettings;
	virtualnet?: VirtualNetSettings | null;
	subscription_id?: string | null;
}

export interface Subscription {
	id: string;
	name: string;
	url: string;
	last_updated_at?: number | null;
	last_server_count?: number | null;
}

export interface SubscriptionRefresh {
	subscription: Subscription;
	servers: ServerConfig[];
}

export type ConnectionStatus =
	| 'disconnected'
	| 'connecting'
	| 'connected'
	| 'disconnecting'
	| 'error';

export interface ConnectionInfo {
	status: ConnectionStatus;
	server_name: string | null;
	server_address: string | null;
	connected_since: number | null;
	error_message: string | null;
}

export interface SpeedStats {
	upload_speed: number;
	download_speed: number;
	total_upload: number;
	total_download: number;
}

export interface LogEntry {
	timestamp: number;
	level: string;
	message: string;
}

export interface AppSettings {
	auto_connect: boolean;
	last_server_id: string | null;
	bypass_domains: string[];
}

export interface DetectedVpn {
	interface: string;
	vpn_type: string;
	subnets: string[];
	server_ip: string | null;
}

// Platform types for cross-platform UI adaptations
export type PlatformType = 'windows' | 'macos' | 'linux';

export interface PlatformInfo {
	platform: PlatformType;
}
