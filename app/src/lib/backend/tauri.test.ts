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
});
