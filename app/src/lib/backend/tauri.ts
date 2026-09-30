import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type {
  AppInfo,
  AutostartStatus,
  GpuProcess,
  HistorySeed,
  Schema,
  ServiceStatus,
  Session,
  SettingsState,
  Snapshot,
  StartupStatus,
  StatsReply,
  NavigationTarget,
} from '../types';
import type { Backend } from './backend';

/** Command and event names are defined in app/src-tauri (commands.rs, main.rs). */
export function createTauriBackend(): Backend {
  return {
    getSchema: () => invoke<Schema>('get_schema'),
    // Tauri maps the camelCase `maxPoints` key to the `max_points` argument; a missing key is `None`.
    getHistory: (ids, seconds, maxPoints) =>
      invoke<HistorySeed>('get_history', maxPoints === undefined ? { ids, seconds } : { ids, seconds, maxPoints }),
    onSchema: (cb) => listen<Schema>('oma:schema', (e) => cb(e.payload)),
    onSnapshot: (cb) => listen<Snapshot>('oma:snapshot', (e) => cb(e.payload)),
    getStartupStatus: () => invoke<StartupStatus>('get_startup_status'),
    enableVendorLibraries: () => invoke<StartupStatus>('enable_vendor_libraries'),
    getStats: (ids) => invoke<StatsReply>('get_stats', { ids }),
    resetStats: (ids) => invoke<void>('reset_stats', { ids }),
    getSession: () => invoke<Session>('get_session'),
    getGpuProcesses: (deviceId) => invoke<GpuProcess[]>('get_gpu_processes', { deviceId }),
    getServiceStatus: () => invoke<ServiceStatus>('get_service_status'),
    onServiceStatus: (cb) => listen<ServiceStatus>('oma:service', (e) => cb(e.payload)),
    setAntiCheat: (enabled) => invoke<ServiceStatus>('set_anti_cheat', { enabled }),
    startService: () => invoke<ServiceStatus>('start_service'),
    getSettings: () => invoke<SettingsState>('get_settings'),
    // A rejected patch arrives as the serialized `{ field, key }` object.
    updateSettings: (patch) => invoke<SettingsState>('update_settings', { patch }),
    resetRuleOverride: (ruleId) => invoke<SettingsState>('reset_rule_override', { ruleId }),
    onSettings: (cb) => listen<SettingsState>('oma:settings', (e) => cb(e.payload)),
    importWebviewState: (legacy) => invoke<SettingsState>('import_webview_state', { legacy }),
    takePendingView: () => invoke<NavigationTarget | null>('take_pending_view'),
    onNavigate: (cb) => listen<NavigationTarget>('oma:navigate', (e) => cb(e.payload)),
    refreshAutostart: () => invoke<AutostartStatus>('refresh_autostart'),
    getAppInfo: () => invoke<AppInfo>('get_app_info'),
    // A unit variant of `KnownPath`: the camelCase string is the whole value.
    openKnownPath: (target) => invoke<void>('open_known_path', { target }),
  };
}
