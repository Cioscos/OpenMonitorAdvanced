import type { Backend, Unsubscribe } from '../lib/backend/backend';
import type {
  GpuProcess,
  HistorySeed,
  HistoryWindow,
  Schema,
  SensorStats,
  ServiceStatus,
  Session,
  Snapshot,
  StartupStatus,
  StatsReply,
} from '../lib/types';

export interface HistoryCall {
  ids: string[];
  seconds: number;
  maxPoints: number | undefined;
}

/** Hand-driven backend for tests: emit events explicitly. */
export class FakeBackend implements Backend {
  schema: Schema;
  /** Oldest first; `getHistory(ids, seconds)` returns the last `seconds` samples (1 sample = 1 s). */
  history: HistoryWindow = { timestampsMs: [], series: [] };
  historyCalls: HistoryCall[] = [];
  schemaCalls = 0;
  startup: StartupStatus = { safeMode: false, reason: null, crashModule: null };
  enableCalls = 0;
  /** Stats by sensor id; ids without an entry read as null. `resetStats` deletes entries. */
  stats: Record<string, SensorStats> = {};
  statsCalls: string[][] = [];
  resetCalls: string[][] = [];
  session: Session = { startedAtMs: null, intervalMs: 1000 };
  /** Returned (copied) for every device id; `gpuProcessCalls` records the ids asked for. */
  gpuProcesses: GpuProcess[] = [];
  gpuProcessCalls: string[] = [];
  /** Current service status; `getServiceStatus` returns it, `emitServiceStatus` replaces it and notifies listeners. */
  serviceStatus: ServiceStatus = { state: 'connected', detail: null };
  /** Set to reject `setAntiCheat`/`startService` with this error instead of resolving. */
  setAntiCheatError: string | null = null;
  startServiceError: string | null = null;
  setAntiCheatCalls: boolean[] = [];
  startServiceCalls = 0;
  #schemaListeners = new Set<(s: Schema) => void>();
  #snapshotListeners = new Set<(s: Snapshot) => void>();
  #serviceListeners = new Set<(s: ServiceStatus) => void>();

  constructor(schema: Schema) {
    this.schema = schema;
  }

  async getSchema(): Promise<Schema> {
    this.schemaCalls++;
    return this.schema;
  }

  async getHistory(ids: string[], seconds: number, maxPoints?: number): Promise<HistorySeed> {
    this.historyCalls.push({ ids, seconds, maxPoints });
    const keep = Math.max(0, Math.floor(seconds));
    const from = Math.max(0, this.history.timestampsMs.length - keep);
    return {
      revision: this.schema.revision,
      seq: 0,
      timestampsMs: this.history.timestampsMs.slice(from),
      series: ids.map((_, i) => (this.history.series[i] ?? []).slice(from)),
    };
  }

  async onSchema(cb: (s: Schema) => void): Promise<Unsubscribe> {
    this.#schemaListeners.add(cb);
    return () => this.#schemaListeners.delete(cb);
  }

  async onSnapshot(cb: (s: Snapshot) => void): Promise<Unsubscribe> {
    this.#snapshotListeners.add(cb);
    return () => this.#snapshotListeners.delete(cb);
  }

  async getStartupStatus(): Promise<StartupStatus> {
    return this.startup;
  }

  async enableVendorLibraries(): Promise<StartupStatus> {
    this.enableCalls++;
    this.startup = { ...this.startup, safeMode: false };
    return this.startup;
  }

  async getStats(ids: string[]): Promise<StatsReply> {
    this.statsCalls.push(ids);
    return { revision: this.schema.revision, stats: ids.map((id) => this.stats[id] ?? null) };
  }

  async resetStats(ids: string[]): Promise<void> {
    this.resetCalls.push(ids);
    for (const id of ids) delete this.stats[id];
  }

  async getSession(): Promise<Session> {
    return this.session;
  }

  async getGpuProcesses(deviceId: string): Promise<GpuProcess[]> {
    this.gpuProcessCalls.push(deviceId);
    return [...this.gpuProcesses];
  }

  async getServiceStatus(): Promise<ServiceStatus> {
    return this.serviceStatus;
  }

  async onServiceStatus(cb: (s: ServiceStatus) => void): Promise<Unsubscribe> {
    this.#serviceListeners.add(cb);
    return () => this.#serviceListeners.delete(cb);
  }

  async setAntiCheat(enabled: boolean): Promise<ServiceStatus> {
    this.setAntiCheatCalls.push(enabled);
    if (this.setAntiCheatError !== null) throw new Error(this.setAntiCheatError);
    return this.serviceStatus;
  }

  async startService(): Promise<ServiceStatus> {
    this.startServiceCalls++;
    if (this.startServiceError !== null) throw new Error(this.startServiceError);
    return this.serviceStatus;
  }

  emitSchema(schema: Schema): void {
    this.schema = schema;
    this.#schemaListeners.forEach((cb) => cb(schema));
  }

  emitSnapshot(snapshot: Snapshot): void {
    this.#snapshotListeners.forEach((cb) => cb(snapshot));
  }

  emitServiceStatus(status: ServiceStatus): void {
    this.serviceStatus = status;
    this.#serviceListeners.forEach((cb) => cb(status));
  }
}
