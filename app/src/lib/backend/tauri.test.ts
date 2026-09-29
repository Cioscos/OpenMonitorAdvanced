import { invoke } from '@tauri-apps/api/core';
import { createTauriBackend } from './tauri';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));

// Names must match app/src-tauri/src/commands.rs; Tauri maps camelCase keys to snake_case arguments.
test('commands and argument names match the Rust shell', async () => {
  const backend = createTauriBackend();
  await backend.getHistory(['a'], 60);
  expect(invoke).toHaveBeenLastCalledWith('get_history', { ids: ['a'], seconds: 60 });
  await backend.getHistory(['a'], 3600, 900);
  expect(invoke).toHaveBeenLastCalledWith('get_history', { ids: ['a'], seconds: 3600, maxPoints: 900 });
  await backend.getStats(['a', 'b']);
  expect(invoke).toHaveBeenLastCalledWith('get_stats', { ids: ['a', 'b'] });
  await backend.resetStats(['a']);
  expect(invoke).toHaveBeenLastCalledWith('reset_stats', { ids: ['a'] });
  await backend.getSession();
  expect(invoke).toHaveBeenLastCalledWith('get_session');
  await backend.getGpuProcesses('gpu/pci-0000:01:00.0');
  expect(invoke).toHaveBeenLastCalledWith('get_gpu_processes', { deviceId: 'gpu/pci-0000:01:00.0' });
  await backend.getServiceStatus();
  expect(invoke).toHaveBeenLastCalledWith('get_service_status');
  await backend.setAntiCheat(true);
  expect(invoke).toHaveBeenLastCalledWith('set_anti_cheat', { enabled: true });
  await backend.startService();
  expect(invoke).toHaveBeenLastCalledWith('start_service');
});

test('oma:service events are forwarded to the listener', async () => {
  const { listen } = await import('@tauri-apps/api/event');
  const backend = createTauriBackend();
  const cb = vi.fn();
  await backend.onServiceStatus(cb);
  expect(listen).toHaveBeenLastCalledWith('oma:service', expect.any(Function));
});

test('settings commands and events match the Rust shell', async () => {
  const { listen } = await import('@tauri-apps/api/event');
  const backend = createTauriBackend();
  await backend.getSettings();
  expect(invoke).toHaveBeenLastCalledWith('get_settings');
  await backend.updateSettings({ general: { intervalMs: 500 } });
  expect(invoke).toHaveBeenLastCalledWith('update_settings', { patch: { general: { intervalMs: 500 } } });
  await backend.importWebviewState({ section: 's', series: {} });
  expect(invoke).toHaveBeenLastCalledWith('import_webview_state', { legacy: { section: 's', series: {} } });
  await backend.takePendingView();
  expect(invoke).toHaveBeenLastCalledWith('take_pending_view');
  await backend.refreshAutostart();
  expect(invoke).toHaveBeenLastCalledWith('refresh_autostart');
  await backend.onSettings(() => {});
  expect(listen).toHaveBeenLastCalledWith('oma:settings', expect.any(Function));
  await backend.onNavigate(() => {});
  expect(listen).toHaveBeenLastCalledWith('oma:navigate', expect.any(Function));
});

test('event payloads reach the listeners unwrapped', async () => {
  const { listen } = await import('@tauri-apps/api/event');
  const backend = createTauriBackend();
  const onSettings = vi.fn();
  const onNavigate = vi.fn();
  await backend.onSettings(onSettings);
  const settingsHandler = vi.mocked(listen).mock.lastCall![1] as (e: { payload: unknown }) => void;
  settingsHandler({ payload: { seq: 3 } });
  expect(onSettings).toHaveBeenCalledWith({ seq: 3 });
  await backend.onNavigate(onNavigate);
  const navigateHandler = vi.mocked(listen).mock.lastCall![1] as (e: { payload: unknown }) => void;
  navigateHandler({ payload: 'advanced' });
  expect(onNavigate).toHaveBeenCalledWith('advanced');
});
