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
  await backend.getRuleStatus();
  expect(invoke).toHaveBeenLastCalledWith('get_rule_status');
  await backend.getDefaultRules();
  expect(invoke).toHaveBeenLastCalledWith('get_default_rules');
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
  navigateHandler({ payload: { view: 'advanced', deviceId: 'gpu/0' } });
  expect(onNavigate).toHaveBeenCalledWith({ view: 'advanced', deviceId: 'gpu/0' });
});

test('log commands and event match the Rust shell', async () => {
  const { listen } = await import('@tauri-apps/api/event');
  const backend = createTauriBackend();
  await backend.getLogStatus();
  expect(invoke).toHaveBeenLastCalledWith('get_log_status');
  await backend.logStart();
  expect(invoke).toHaveBeenLastCalledWith('log_start');
  await backend.logPause();
  expect(invoke).toHaveBeenLastCalledWith('log_pause');
  await backend.logResume();
  expect(invoke).toHaveBeenLastCalledWith('log_resume');
  await backend.logStop();
  expect(invoke).toHaveBeenLastCalledWith('log_stop');
  await backend.openLogFolder();
  expect(invoke).toHaveBeenLastCalledWith('open_log_folder');
  await backend.pickLogFolder();
  expect(invoke).toHaveBeenLastCalledWith('pick_log_folder');
  await backend.onLogStatus(vi.fn());
  expect(listen).toHaveBeenLastCalledWith('oma:log', expect.any(Function));
});

test('the disk state command and event match the Rust shell', async () => {
  const { listen } = await import('@tauri-apps/api/event');
  const backend = createTauriBackend();
  await backend.getDiskStates();
  expect(invoke).toHaveBeenLastCalledWith('get_disk_states');
  await backend.onDiskStates(vi.fn());
  expect(listen).toHaveBeenLastCalledWith('oma:disk-states', expect.any(Function));
});

test('overlay commands and event match the Rust shell', async () => {
  const { listen } = await import('@tauri-apps/api/event');
  const backend = createTauriBackend();
  await backend.getOverlayStatus();
  expect(invoke).toHaveBeenLastCalledWith('get_overlay_status');
  await backend.overlayRetry();
  expect(invoke).toHaveBeenLastCalledWith('overlay_retry');
  await backend.overlayReloadProfiles();
  expect(invoke).toHaveBeenLastCalledWith('overlay_reload_profiles');
  await backend.setOverlayHidden(false);
  expect(invoke).toHaveBeenLastCalledWith('set_overlay_hidden', { hidden: false });
  await backend.onOverlayStatus(vi.fn());
  expect(listen).toHaveBeenLastCalledWith('overlay-status', expect.any(Function));
});

// Names must match app/src-tauri/src/overlay/{editor,runner,benchmark}.rs and window.rs.
test('editor and benchmark commands match the Rust shell', async () => {
  const { listen } = await import('@tauri-apps/api/event');
  const backend = createTauriBackend();
  const calls: [() => Promise<unknown>, string, Record<string, unknown>?][] = [
    [() => backend.overlayLoadProfile('builtin-gaming'), 'overlay_load_profile', { id: 'builtin-gaming' }],
    [() => backend.overlaySaveProfile(null, '{}'), 'overlay_save_profile', { id: null, json: '{}' }],
    [() => backend.overlayDeleteProfile('x'), 'overlay_delete_profile', { id: 'x' }],
    [() => backend.overlayDuplicateProfile('x'), 'overlay_duplicate_profile', { id: 'x' }],
    [() => backend.overlayImportProfile(), 'overlay_import_profile'],
    [() => backend.overlayExportProfile('x'), 'overlay_export_profile', { id: 'x' }],
    [() => backend.overlayFontFamilies(), 'overlay_font_families'],
    [() => backend.overlayPreview(null), 'overlay_preview', { json: null }],
    [() => backend.overlayEditorProfile('{}'), 'overlay_editor_profile', { json: '{}' }],
    [() => backend.overlayUseNow('x'), 'overlay_use_now', { id: 'x' }],
    [() => backend.overlayEditorDirty(true), 'overlay_editor_dirty', { dirty: true }],
    [() => backend.openOverlayEditor(), 'open_overlay_editor'],
    [() => backend.appQuitConfirmed(), 'app_quit_confirmed'],
    [() => backend.performanceQuitConfirmed(), 'performance_quit_confirmed'],
    [() => backend.benchmarkToggle(), 'benchmark_toggle'],
    [() => backend.benchmarkList(), 'benchmark_list'],
    [() => backend.benchmarkOpenCsv('x'), 'benchmark_open_csv', { id: 'x' }],
    [() => backend.benchmarkOpenFolder(), 'benchmark_open_folder'],
    [() => backend.benchmarkDelete('x'), 'benchmark_delete', { id: 'x' }],
  ];
  for (const [call, name, args] of calls) {
    await call();
    if (args === undefined) expect(invoke).toHaveBeenLastCalledWith(name);
    else expect(invoke).toHaveBeenLastCalledWith(name, args);
  }
  await backend.onOverlayEditorData(() => {});
  expect(listen).toHaveBeenLastCalledWith('overlay-editor-data', expect.any(Function));
  await backend.onOverlayPreview(() => {});
  expect(listen).toHaveBeenLastCalledWith('overlay-preview', expect.any(Function));
  const onQuit = vi.fn();
  await backend.onOverlayEditorQuit(onQuit);
  expect(listen).toHaveBeenLastCalledWith('overlay-editor-quit', expect.any(Function));
  (vi.mocked(listen).mock.lastCall![1] as (e: { payload: unknown }) => void)({ payload: null });
  expect(onQuit).toHaveBeenCalledOnce();
});
